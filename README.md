# Komago

Lecture et gestion locale de mangas, webcomics et comics. Sans compte, sans serveur, utilisable hors ligne.

**État : bibliothèque, import, lecteur et métadonnées fonctionnels. Testé sur macOS ; Windows et Linux restent à valider.** Voir [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Développement

Prérequis : [Bun](https://bun.sh), Rust ≥ 1.89 et les [prérequis système de Tauri](https://tauri.app/start/prerequisites/).

```bash
bun install
bun run tauri dev      # application en développement
bun run check          # typage du frontend
bun run test           # tests du frontend et de Rust
bun run tauri build    # paquet de l'application
```

Pour essayer sans toucher à sa bibliothèque, un build de développement accepte un dossier jetable :

```bash
KOMAGO_DEV_ROOT=/tmp/komago-essai bun run tauri dev
```
