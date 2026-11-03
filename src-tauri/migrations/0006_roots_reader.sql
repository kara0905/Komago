-- Dossiers suivis : date du dernier passage, et éléments que l'utilisateur a
-- choisi de ne plus voir proposés.
ALTER TABLE library_roots ADD COLUMN last_scanned_at TEXT;

CREATE TABLE ignored_sources (
    path       TEXT PRIMARY KEY NOT NULL,
    ignored_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT, WITHOUT ROWID;

-- Double page : décale l'appariement d'une page pour les tomes sans couverture.
ALTER TABLE series ADD COLUMN spread_offset INTEGER NOT NULL DEFAULT 0 CHECK (spread_offset IN (0, 1));
