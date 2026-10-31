# Komago — Architecture

État : **phases 1 à 5 implémentées**, avec des limites listées plus bas. macOS est testé ; Windows et Linux restent à valider en CI. Ce document distingue ce qui existe de ce qui est prévu ; voir [Plan et état d'avancement](#plan-et-état-davancement).

## Principes

1. **Local d'abord.** Sans réseau, sans compte, sans serveur : tout l'essentiel fonctionne. Le réseau n'enrichit que les métadonnées.
2. **SQLite est la source de vérité.** Les fichiers restent là où ils sont ; une série n'est jamais assimilée à un dossier.
3. **Les originaux sont intouchables.** Aucune écriture dans une archive, aucune conversion à l'import.
4. **Le frontend ne touche pas au système.** Pas de plugin `fs` ni `shell` : le webview ne dispose que de commandes Rust étroites et validées.
5. **Peu de dépendances**, chacune justifiée ci-dessous.

## Pile technique (vérifiée le 31/10/2026)

| Composant | Version | Licence | Raison, ou écart par rapport au cahier des charges |
| --- | --- | --- | --- |
| Tauri | 2.11 | Apache-2.0/MIT | Tauri 3 n'est qu'en alpha : écarté. Tauri 2.12 exige Rust 1.90 : passer à Rust ≥ 1.90 permettra de monter crate et paquets npm ensemble (ils doivent rester sur la même version mineure). |
| Rust | 1.89 (MSRV du projet) | | Version installée. `resolver = "3"` choisit des dépendances compatibles. |
| rusqlite (`bundled`) | 0.41 | MIT | SQLite embarqué avec FTS5, même version sur tous les OS. |
| Svelte / Vite | 5.57 / 8.3 | MIT | Pas de SvelteKit : une SPA locale n'a besoin ni de SSR ni d'un routeur de fichiers. |
| TypeScript | 6.x | Apache-2.0 | **Pas la 7** : `svelte-check` 4.7 exige `^5 \|\| ^6`. |
| zip | 9.x | MIT | Lecture à la demande d'une entrée, sans extraction. |
| quick-xml | 0.42 | MIT | ComicInfo.xml, sans résolution d'entités externes. |
| image | 0.25 | MIT/Apache-2.0 | Couvertures uniquement ; les pages restent intactes. AVIF non décodé en Rust : la page s'importe, mais sans couverture générée. |
| tauri-plugin-dialog | 2.x | MIT/Apache-2.0 | Sélecteurs natifs ouverts côté Rust, sans permission pour le webview. |
| regex, blake3, uuid, unicode-normalization | | MIT/Apache-2.0 | Noms de fichiers, empreintes, identifiants, comparaison des titres. |
| reqwest | 0.13 | MIT/Apache-2.0 | Client bloquant, créé à la première requête, toujours derrière la garde réseau. TLS par rustls. |
| strsim | 0.11 | MIT | Similarité des titres pour le score. |

Écartés : `tauri-plugin-sql` (il exposerait du SQL au webview), `rusqlite_migration` (30 lignes suffisent et permettent une sauvegarde avant migration), `tauri-plugin-fs` (surface d'attaque inutile).

**À étudier avant adoption :** `unrar` a une licence restrictive (UnRAR) incompatible avec certaines distributions libres ; `sevenz-rust2` (CB7) a l'air sain ; `pdfium-render` impose d'embarquer un binaire Pdfium par plateforme. Aucun des trois n'entre avant CBZ/ZIP/dossiers.

## Vue d'ensemble

```
Svelte 5 (webview)
  │  invoke()  : commandes typées
  │  komago:// : octets d'images en flux
Rust
  commands/   adaptateurs Tauri, sans logique
  settings    paramètres typés sur la table settings
  storage     répertoires de l'application
  db/         connexions, migrations
  metadata/   client gardé, fournisseurs, score, fusion et provenance
  library     séries, volumes, chapitres, recherche
  collections collections personnelles et système
  import/     analyse, plan, validation, fichiers déplacés
  archive     lecture sécurisée CBZ/ZIP/dossiers
  comicinfo   lecture de ComicInfo.xml
  covers      génération et stockage des couvertures
  protocol    protocole komago://
  page_store  cache mémoire des pages, archives gardées ouvertes
  reader      ouverture des chapitres, progression, états de lecture
```

### Passage des images

Les pages ne passent **jamais** par `invoke` (JSON/IPC trop lent). Le protocole asynchrone `komago://` sert les couvertures, les aperçus d'import et les pages. La CSP l'autorise déjà pour `img-src`. Sous Windows, Tauri l'expose en `http://komago.localhost`, ce que la CSP prend aussi en compte.

### Concurrence SQLite

WAL, avec une connexion d'écriture et une connexion de lecture, chacune derrière un `Mutex`. Les écritures sont donc sérialisées et les lectures ne les bloquent pas. Les commandes tournent hors du thread principal. Un petit pool de lecture pourra remplacer la connexion unique sans changer l'API `read`/`write` si le profilage le justifie.

## Modèle de données

Identifiants : UUIDv7 en `TEXT`, stables et triables par date. Dates : ISO 8601 UTC. Dates de publication : précision partielle admise (`2019`, `2019-04`, `2019-04-12`). Le schéma complet est dans [`src-tauri/migrations/`](../src-tauri/migrations/).

```
library_roots ──< source_files ──< chapters >── series ──< volumes
                                      │           │  └──< series_titles
                     pages ───────────┤           ├──< series_people >── people
                     reading_progress ┤           ├──< series_genres >── genres
                     reading_history ─┘           ├──< series_tags   >── tags
                                                  ├──< external_ids
                                                  └──< metadata_snapshots
collections ──< collection_items → (series | chapter)
field_provenance → (series | volume | chapter, champ) → metadata_sources
```

Choix qui s'écartent de la liste indicative, ou qui la précisent :

- **`people` / `series_people(role)` au lieu de `authors`.** Un comics distingue scénariste, dessinateur, encreur, coloriste ; deux tables d'auteurs et d'artistes seraient vite insuffisantes.
- **`number` + `number_label` + `sort_key`.** Les numéros réels sont `10.5`, `Extra`, `Annual 1`. `number` est calculable et nul au besoin, `number_label` conserve l'affichage, `sort_key` donne l'ordre, modifiable à la main.
- **Pas d'unicité sur (série, numéro).** Deux traductions ou deux éditions du même chapitre sont légitimes.
- **`chapters.source_subpath`.** Un seul CBZ peut contenir plusieurs chapitres (tome complet).
- **`chapters.volume_id` nullable**, avec un trigger qui garantit que le volume appartient à la même série. La suppression d'un volume conserve ses chapitres.
- **Position de lecture : `page_index` + `page_offset` (fraction 0–1 de la page).** Un nombre de pixels deviendrait faux dès que la fenêtre ou le zoom change.
- **`field_provenance`** : une ligne par champ et par entité, avec sa source et un verrou `locked`. Toute édition manuelle pose `locked = 1`, et aucune actualisation ne peut alors écraser le champ.
- **`metadata_snapshots`** : réponse brute du fournisseur en JSON, pour rejouer un rapprochement ou revenir en arrière sans réseau.
- **Index de recherche FTS5 autonome** (`search_index`), tenu à jour par triggers. Les tables à clé `TEXT` n'ont pas de `rowid` stable après `VACUUM`, ce qui exclut le mode *external content*.

### Lecteur

- **Virtualisation.** En défilement vertical, seules les pages proches de l'écran sont dans le DOM (marge de deux écrans), positionnées d'après leurs dimensions. Six pages suivantes et deux précédentes sont préchargées. Mesuré sur un chapitre de 600 pages de test : 59 Mo pour le webview après un saut au milieu.
- **Dimensions.** Lues dans les en-têtes à la première ouverture du chapitre, puis conservées en base. Si elles sont illisibles (AVIF notamment), une taille provisoire est corrigée au chargement de l'image, sans déplacer la position de lecture.
- **Cache.** LRU en mémoire de 384 Mo maximum côté Rust, et trois archives gardées ouvertes. **Pas de cache de pages sur disque** : les pages sont déjà des images compressées, et les relire reste rapide. À reconsidérer seulement si des sources lentes (disque réseau) le justifient.
- **Progression.** Enregistrée 400 ms après chaque mouvement, et à la fermeture du lecteur ou quand la fenêtre passe en arrière-plan. La position est une fraction de page, donc elle reste juste si la fenêtre change de taille. Un arrêt brutal fait perdre au plus ces 400 ms.
- **États de lecture.** Première lecture : la série passe En cours, sauf si elle a déjà un état. Tout est lu : elle passe Terminés, sauf si elle a été abandonnée. Un chapitre à 99,5 % ou plus est lu, et le relire ne l'annule pas.
- **Sens de lecture.** Retenu par série. Le mode et l'ajustement par défaut sont des paramètres, avec un ajustement propre au défilement vertical.

## Stockage

| Contenu | Emplacement | Effaçable sans perte ? |
| --- | --- | --- |
| Base `komago.db` + sauvegardes pré-migration | données de l'application | Non |
| Couvertures téléchargées (`covers/`) | données de l'application | Non : les URL externes ne sont jamais conservées comme référence |
| Originaux copiés, si l'utilisateur le demande (`library/`) | données de l'application | Non |
| Dossiers référencés | ailleurs, chez l'utilisateur | Jamais modifiés |
| Miniatures, pages, extractions temporaires | cache de l'application | Oui, régénérables |

Fichiers déplacés : `source_files` conserve taille, date et empreinte partielle (BLAKE3 du début et de la fin). Un fichier introuvable passe en `missing` sans perdre aucune donnée ; à la réapparition d'une empreinte identique, le chemin est recollé et la progression reste intacte.

## Sécurité de l'import

Une archive est **lue, pas extraite**. Quand une extraction reste nécessaire (RAR solide, etc.), le nom d'entrée n'est jamais utilisé comme chemin de destination.

| Menace | Parade |
| --- | --- |
| Zip Slip, chemins absolus, `..`, `\`, liens symboliques | Noms traités comme des clés opaques. Toute entrée suspecte est rejetée et signalée. |
| Bombe ZIP | Plafonds d'entrées, de taille décompressée par entrée et totale, de ratio. La lecture s'arrête au plafond **réellement décompressé**, sans croire l'en-tête. |
| Archive corrompue | Aucune panique : erreur par fichier, l'import des autres continue. |
| Extension trompeuse | Détection par signature (octets magiques), jamais par extension. |
| Image malveillante | Plafond de dimensions avant décodage ; décodage Rust uniquement pour les miniatures. Le XML de ComicInfo est limité en taille. |

## Métadonnées

```rust
trait MetadataProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn hosts(&self) -> Vec<HostRule>;   // hôtes autorisés et débit
    fn search_series(&self, query: &SeriesQuery, http: &GuardedClient) -> Result<Vec<ProviderSeries>>;
    fn get_series(&self, external_id: &str, http: &GuardedClient) -> Result<ProviderSeries>;
}
```

Le trait est synchrone : c'est plus simple, et les appels tournent dans `spawn_blocking`. Ajouter un fournisseur revient à l'inscrire dans `providers()` et à ajouter sa ligne dans `metadata_sources`, sans toucher à la bibliothèque.

- **`GuardedClient`** est le seul accès réseau de l'application. Sans autorisation, aucune requête ne part. Il n'accepte que HTTPS et les hôtes déclarés, refuse les redirections, plafonne les réponses et espace les requêtes. La CSP interdit par ailleurs au webview de contacter un service : les couvertures de résultats passent par `komago://candidate`.
- **Données transmises** : uniquement le titre recherché, ou l'identifiant d'une fiche déjà associée.
- **Fiches gardées côté Rust** : le frontend les désigne par un numéro. Il ne peut donc pas faire enregistrer des données ni télécharger une adresse de son choix.
- **Score** : titres et titres alternatifs normalisés (70 %), auteur (20 %) et type (10 %) quand ils sont connus des deux côtés. « Très probable » exige 90 % et 10 points d'avance sur les autres fiches du même fournisseur.
- **Aucune association automatique, quel que soit le score.** Rechercher n'écrit rien : c'est toujours un clic de l'utilisateur. Une fiche écartée n'est plus proposée pour cette série (`rejected_matches`).
- **Modes** : automatique (les champs vides sont complétés dès le choix), assisté (champs vides cochés, le reste attend une décision), manuel (rien de coché). Réglage général, avec exception possible par série.
- **Provenance** : par champ, dans `field_provenance`. Une édition manuelle pose un verrou. Seule une case cochée explicitement peut remplacer une valeur verrouillée : le chemin automatique ne le peut jamais.
- **Conservation** : les 3 dernières réponses brutes par série et fournisseur dans `metadata_snapshots`, couverture réduite et stockée dans `covers/`. Aucune URL externe n'est nécessaire à l'affichage.

### Conditions des services (lues le 31/10/2026)

| Service | Limites | Obligations et points d'attention | État |
| --- | --- | --- | --- |
| MangaDex | Environ 5 requêtes/s par IP (Komago : 1 toutes les 300 ms) | User-Agent non falsifié, créditer MangaDex, pas de publicité ni de service payant | Intégré, crédit dans les paramètres. **Une version payante ou avec publicité exigerait de le retirer.** |
| AniList | 90 requêtes/min, réduites à 30 actuellement (Komago : 1 toutes les 2,1 s) | Non commercial sous 150 $/mois, pas de stockage de masse ni d'usage comme sauvegarde, **pas d'usage dans un service concurrent de type liste ou suivi de mangas** | Intégré, à la demande et fiche par fiche. **Point à trancher, voir les risques.** |
| Comic Vine | 200 requêtes/ressource/heure | Clé propre à chaque utilisateur, non commercial, lien de retour | Non intégré : à reconsidérer après la phase 5. |

## Risques

| Risque | Conséquence | Réponse |
| --- | --- | --- |
| Webtoons très longs avec de vraies images lourdes | Saccades au décodage | Virtualisation validée sur 600 pages de test légères ; à confirmer sur de vrais fichiers lourds. |
| Lecture ZIP compressée (Deflate) à la demande | Latence | Cache mémoire et préchargement. |
| WebKitGTK (Linux) moins performant que WebView2 / WKWebView | Lecteur moins fluide sous Linux | Tester tôt sur Linux, décodage d'images asynchrone. |
| Recherche CJK avec `unicode61` | Un titre japonais forme un seul jeton | Recherche par préfixe en phase 2 ; ajouter le tokeniseur `trigram` si insuffisant. |
| Signature et notarisation (macOS, Windows) | Avertissements de sécurité à l'installation | Nécessite des comptes développeur payants (décision à prendre en phase 5). |
| CBR / RAR | Licence UnRAR | À valider avant toute adoption. |
| Conditions AniList : Komago gère des états de lecture (En cours, Terminés), ce qui peut être lu comme un service de suivi | Accès refusé, en cas de désaccord | Komago est un lecteur local sans service en ligne, et AniList se désactive en un réglage. **Avant une diffusion publique, demander confirmation à contact@anilist.co ou retirer le fournisseur.** |

## Plan et état d'avancement

| Phase | Contenu | État |
| --- | --- | --- |
| 1. Fondations | Tauri + Svelte, SQLite, migrations avec sauvegarde, paramètres, stockage | **Fait** |
| 2. Bibliothèque | Lecture sécurisée CBZ/ZIP/dossiers, analyse et plan corrigeable, séries/volumes/chapitres, collections, recherche | **Fait** |
| 3. Lecteur | Pages via `komago://`, modes de lecture, progression, cache, préchargement | **Fait** |
| 4. Métadonnées | ComicInfo, MangaDex, AniList, rapprochement, snapshots et conflits | **Fait** |
| 5. Finitions | Écran d'échec au démarrage, annulation d'analyse, liens, à propos, accessibilité, mesures, CI et workflow de paquets | **Fait**, CI jamais exécutée |

### Non fonctionnel à ce jour

- **Identification** : à lancer série par série ; proposée à la fin d'un import pour la première série seulement, sans traitement groupé.
- **Actualisation** : sur demande uniquement ; aucune vérification périodique.
- **Volumes et chapitres** : seules les métadonnées de la série viennent des fournisseurs.
- **Formats** : CBR, CB7 et PDF sont signalés comme non pris en charge.
- **Dossiers référencés** : la table `library_roots` n'est pas utilisée : pas de surveillance ni de rescan d'un dossier, il faut réimporter pour ajouter de nouveaux fichiers.
- **Langue** : interface en français uniquement.
- **ComicInfo sur une série existante** : ses informations ne complètent que les séries créées par l'import.
- **Lecteur** : pas de zoom libre ; le geste de retour au chapitre précédent depuis la première page est immédiat, sans confirmation.
- **Plateformes** : testé uniquement sur macOS.

## Mesures (31/10/2026, Mac Apple Silicon, build release)

| Mesure | Résultat |
| --- | --- |
| Liste de 5 000 séries et 200 000 chapitres | 71 ms (tri titre), 97 ms (lecture récente) |
| Recherche dans ce catalogue | 0,9 ms |
| Fiche d'une série de 40 chapitres | 0,3 ms |
| Webview, chapitre de 600 pages légères, après saut au milieu | 59 Mo |
| Webview, liste de 5 000 séries sans couvertures | 248 Mo |

À refaire avec `cargo test --release --lib mesure -- --ignored --nocapture`.

## Distribution

- **CI** : `.github/workflows/ci.yml` (typage, tests et compilation sur les trois systèmes).
- **Paquets** : `.github/workflows/release.yml`, lancement manuel, produit des paquets non signés en artefacts sans rien publier.
- **Signature** : à décider. Sans certificat (Apple Developer, éditeur Windows), macOS et Windows afficheront des avertissements à l'installation.
- **Licence** : aucune choisie. Le dépôt n'accorde donc aucun droit de réutilisation tant qu'elle n'est pas définie.

## Décisions validées (31/10/2026)

1. Connexions externes désactivées par défaut.
2. En cours, Terminés et Abandonnés mutuellement exclusifs pour une série ; Favoris et À lire sont de simples listes.
3. Import par référence par défaut, copie en option. Retirer une série ne supprime jamais un fichier référencé.
4. Développement sur macOS d'abord, Windows et Linux vérifiés en CI en phase 5.
