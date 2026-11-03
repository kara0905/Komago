# Contribuer à Komago

Merci de votre intérêt ! Ce guide explique comment signaler un problème, proposer une idée ou envoyer une modification.

## Signaler un bug ou proposer une idée

Ouvrez une [issue](https://github.com/kara0905/Komago/issues/new/choose) avec le modèle adapté. Pour un bug, indiquez votre système, la version de Komago (Paramètres → À propos) et les étapes pour le reproduire. N'y joignez pas de fichiers protégés par le droit d'auteur : décrivez plutôt leur structure (noms des dossiers et des fichiers).

Une faille de sécurité ne se signale pas dans une issue publique : voir [SECURITY.md](SECURITY.md).

## Avant de coder

Pour une modification importante, ouvrez d'abord une issue pour en discuter. Cela évite de travailler sur une direction qui ne serait pas retenue. Les principes du projet sont décrits dans [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) ; les plus structurants :

- **Local d'abord** : tout l'essentiel doit fonctionner sans réseau. Une fonction en ligne reste facultative et passe par le client réseau gardé (`src-tauri/src/metadata/http.rs`).
- **Les fichiers de l'utilisateur ne sont jamais modifiés.**
- **Le webview n'accède pas au système** : il passe par des commandes Rust étroites et validées.
- **Peu de dépendances**, chacune justifiée par sa maintenance, sa licence et sa compatibilité.

## Mettre en place l'environnement

Prérequis : [Bun](https://bun.sh), Rust 1.89 ou plus récent, un compilateur C et C++, et les [prérequis système de Tauri](https://tauri.app/start/prerequisites/).

```bash
bun install
bun run tauri dev
```

Pour essayer sans toucher à votre propre bibliothèque :

```bash
KOMAGO_DEV_ROOT=/tmp/komago-essai bun run tauri dev
```

## Avant d'envoyer une pull request

```bash
bun run check   # typage du frontend, sans erreur ni avertissement
bun run test    # tests des vues puis tests Rust
```

- Ajoutez des tests pour tout comportement nouveau : en Rust à côté du code (`#[cfg(test)]`), pour l'interface dans un fichier `*.test.ts` à côté du composant.
- Suivez le style du code qui entoure votre modification. Le code, les commentaires et l'interface sont en français.
- Si vous ajoutez, retirez ou mettez à jour une dépendance, régénérez l'inventaire des licences avec `bun run third-party` et vérifiez que la nouvelle licence est compatible avec MIT et Apache-2.0.
- Toute modification du schéma passe par une **nouvelle** migration dans `src-tauri/migrations/` : une migration publiée ne se modifie jamais.
- Décrivez dans la pull request ce qui change, pourquoi, et comment vous l'avez vérifié.

## Licence des contributions

En envoyant une contribution, vous acceptez qu'elle soit distribuée sous la double licence MIT et Apache-2.0 du projet, sans condition supplémentaire.

## Comportement

Ce projet suit un [code de conduite](CODE_OF_CONDUCT.md). En participant, vous vous engagez à le respecter.
