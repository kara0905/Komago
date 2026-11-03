<p align="center">
  <img src="src/assets/komago-logo.png" alt="" width="96" height="96" />
</p>

<h1 align="center">Komago</h1>

<p align="center">
  Lecture et gestion locale de mangas, webcomics et comics.<br />
  Sans compte, sans serveur, entièrement utilisable hors ligne.
</p>

<p align="center">
  <a href="https://github.com/kara0905/Komago/actions/workflows/ci.yml"><img src="https://github.com/kara0905/Komago/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="#licence"><img src="https://img.shields.io/badge/licence-MIT%20%2F%20Apache--2.0-blue" alt="Licence MIT ou Apache-2.0" /></a>
</p>

> **Projet jeune.** Komago est en version 0.1. Il est utilisé et testé sur macOS ; Windows et Linux
> sont compilés et testés automatiquement, mais l'application n'y a pas encore été éprouvée à la main.
> Les paquets ne sont pas signés.

## Ce que fait Komago

**Bibliothèque**
- Import de CBZ, ZIP, CB7, CBR, de dossiers d'images ou d'images isolées, par glisser-déposer ou par sélection.
- Détection des séries, volumes et chapitres d'après les noms de fichiers et les dossiers, avec un plan modifiable avant tout import.
- Lecture de ComicInfo.xml quand l'archive en contient un.
- Dossiers suivis : les nouveaux chapitres sont repérés au démarrage.
- Collections, recherche sans accents, tri et filtres, vues grille et liste.
- Vos fichiers ne sont jamais modifiés ; un fichier déplacé est retrouvé sans perte de progression.

**Lecteur**
- Défilement vertical pour les webtoons, pagination de gauche à droite ou de droite à gauche.
- Double page, avec décalage pour les tomes sans couverture ; zoom et déplacement à la souris.
- Reprise exacte de la position, même après une fermeture brutale ; accueil « Continuer la lecture ».
- Réglages d'affichage : ajustement, largeur, espacement, luminosité, contraste, teinte nocturne, défilement automatique.

**Métadonnées (facultatif)**
- Identification des séries avec [MangaDex](https://mangadex.org) et [AniList](https://anilist.co), une par une ou en groupe.
- Rien n'est associé sans votre confirmation, sauf les correspondances très probables d'une identification groupée que vous lancez vous-même.
- Vos modifications manuelles ne sont jamais écrasées ; chaque information garde sa source.

**Vos données restent chez vous**
- Aucune télémétrie, aucun compte. Aucune connexion n'est établie tant que vous ne l'autorisez pas dans les paramètres, et seul le titre recherché est alors envoyé.
- Sauvegarde et restauration de toute la bibliothèque en un fichier.

## Installer

Les paquets de chaque version sont publiés dans les [Releases](https://github.com/kara0905/Komago/releases) : `.dmg` pour macOS, `.msi` ou `.exe` pour Windows, `.AppImage`, `.deb` ou `.rpm` pour Linux.

Ils ne sont pas signés :
- **macOS** : au premier lancement, faites un clic droit sur l'application, puis **Ouvrir**.
- **Windows** : SmartScreen peut afficher un avertissement ; choisissez **Informations complémentaires**, puis **Exécuter quand même**.

## Compiler depuis les sources

Prérequis : [Bun](https://bun.sh), Rust 1.89 ou plus récent, un compilateur C et C++, et les [prérequis système de Tauri](https://tauri.app/start/prerequisites/).

```bash
bun install
bun run tauri dev      # application en développement
bun run check          # typage du frontend
bun run test           # tests des vues (vitest) puis de Rust
bun run tauri build    # paquet de l'application
```

Pour essayer sans toucher à sa bibliothèque, un build de développement accepte un dossier jetable :

```bash
KOMAGO_DEV_ROOT=/tmp/komago-essai bun run tauri dev
```

L'architecture, les choix techniques et les limites connues sont décrits dans [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Contribuer

Les signalements de bugs, idées et contributions sont bienvenus : voir [CONTRIBUTING.md](CONTRIBUTING.md). Pour une faille de sécurité, suivez plutôt [SECURITY.md](SECURITY.md).

## Licence

Komago est distribué sous la licence de votre choix :

- [Apache License 2.0](LICENSE-APACHE)
- [Licence MIT](LICENSE-MIT)

Sauf mention contraire explicite, toute contribution que vous soumettez volontairement pour inclusion dans Komago, au sens de la licence Apache-2.0, est distribuée sous cette double licence, sans condition supplémentaire.

Komago s'appuie sur des composants tiers listés dans [THIRD-PARTY.md](THIRD-PARTY.md). La lecture des CBR repose sur le code UnRAR de RARLAB, sous [sa propre licence](licenses/UnRAR.txt).

Komago n'est affilié ni à MangaDex, ni à AniList. Les informations et couvertures qu'ils fournissent appartiennent à leurs auteurs respectifs.
