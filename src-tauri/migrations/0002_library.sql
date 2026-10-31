-- Bibliothèque : œuvres, localisation physique, collections, progression.
-- Les œuvres (series, volumes, chapters) sont séparées des fichiers
-- (library_roots, source_files) : une série n'est pas un dossier.

-- Localisation physique -------------------------------------------------------

CREATE TABLE library_roots (
    id         TEXT PRIMARY KEY NOT NULL,
    path       TEXT NOT NULL UNIQUE,
    label      TEXT,
    status     TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available', 'missing')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE source_files (
    id            TEXT PRIMARY KEY NOT NULL,
    root_id       TEXT REFERENCES library_roots (id) ON DELETE SET NULL,
    path          TEXT NOT NULL UNIQUE,
    kind          TEXT NOT NULL CHECK (kind IN ('cbz', 'zip', 'directory', 'cbr', 'cb7', 'pdf')),
    storage       TEXT NOT NULL DEFAULT 'referenced' CHECK (storage IN ('referenced', 'managed')),
    size_bytes    INTEGER NOT NULL CHECK (size_bytes >= 0),
    modified_at   TEXT,
    partial_hash  TEXT,
    status        TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available', 'missing', 'corrupt')),
    last_seen_at  TEXT,
    imported_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX source_files_root ON source_files (root_id);
CREATE INDEX source_files_hash ON source_files (partial_hash) WHERE partial_hash IS NOT NULL;
CREATE INDEX source_files_status ON source_files (status) WHERE status <> 'available';

-- Images stockées par Komago (couvertures). Jamais de binaire en base, et le
-- chemin est relatif au répertoire de données de l'application.
CREATE TABLE assets (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('cover')),
    path        TEXT NOT NULL UNIQUE,
    content_hash TEXT,
    width       INTEGER,
    height      INTEGER,
    origin_url  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Œuvres ----------------------------------------------------------------------

CREATE TABLE series (
    id                TEXT PRIMARY KEY NOT NULL,
    title             TEXT NOT NULL CHECK (length(title) > 0),
    sort_title        TEXT NOT NULL,
    kind              TEXT NOT NULL DEFAULT 'unknown'
                      CHECK (kind IN ('unknown', 'manga', 'manhwa', 'manhua', 'webcomic', 'comic', 'other')),
    description       TEXT,
    publisher         TEXT,
    published_from    TEXT,
    published_to      TEXT,
    status            TEXT NOT NULL DEFAULT 'unknown'
                      CHECK (status IN ('unknown', 'ongoing', 'completed', 'hiatus', 'cancelled')),
    original_language TEXT,
    reading_direction TEXT CHECK (reading_direction IN ('ltr', 'rtl', 'vertical')),
    cover_asset_id    TEXT REFERENCES assets (id) ON DELETE SET NULL,
    metadata_mode     TEXT CHECK (metadata_mode IN ('automatic', 'assisted', 'manual')),
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX series_sort_title ON series (sort_title COLLATE NOCASE);
CREATE INDEX series_updated ON series (updated_at);

CREATE TABLE series_titles (
    series_id TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    title     TEXT NOT NULL CHECK (length(title) > 0),
    language  TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (series_id, title, language)
) STRICT, WITHOUT ROWID;

CREATE TABLE people (
    id   TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
) STRICT;

CREATE TABLE series_people (
    series_id TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    person_id TEXT NOT NULL REFERENCES people (id) ON DELETE CASCADE,
    role      TEXT NOT NULL CHECK (role IN ('author', 'artist', 'writer', 'penciller', 'inker', 'colorist', 'letterer', 'other')),
    PRIMARY KEY (series_id, person_id, role)
) STRICT, WITHOUT ROWID;
CREATE INDEX series_people_person ON series_people (person_id);

CREATE TABLE genres (
    id   TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
) STRICT;

CREATE TABLE series_genres (
    series_id TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    genre_id  TEXT NOT NULL REFERENCES genres (id) ON DELETE CASCADE,
    PRIMARY KEY (series_id, genre_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX series_genres_genre ON series_genres (genre_id);

CREATE TABLE tags (
    id   TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
) STRICT;

CREATE TABLE series_tags (
    series_id TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    tag_id    TEXT NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (series_id, tag_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX series_tags_tag ON series_tags (tag_id);

CREATE TABLE volumes (
    id             TEXT PRIMARY KEY NOT NULL,
    series_id      TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    number         REAL,
    number_label   TEXT,
    title          TEXT,
    description    TEXT,
    published_on   TEXT,
    cover_asset_id TEXT REFERENCES assets (id) ON DELETE SET NULL,
    sort_key       REAL NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX volumes_series ON volumes (series_id, sort_key);

CREATE TABLE chapters (
    id             TEXT PRIMARY KEY NOT NULL,
    series_id      TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    volume_id      TEXT REFERENCES volumes (id) ON DELETE SET NULL,
    kind           TEXT NOT NULL DEFAULT 'chapter' CHECK (kind IN ('chapter', 'issue', 'oneshot', 'extra')),
    number         REAL,
    number_label   TEXT,
    title          TEXT,
    published_on   TEXT,
    language       TEXT,
    page_count     INTEGER NOT NULL DEFAULT 0 CHECK (page_count >= 0),
    sort_key       REAL NOT NULL DEFAULT 0,
    source_id      TEXT REFERENCES source_files (id) ON DELETE SET NULL,
    source_subpath TEXT NOT NULL DEFAULT '',
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX chapters_series ON chapters (series_id, sort_key);
CREATE INDEX chapters_volume ON chapters (volume_id, sort_key);
CREATE INDEX chapters_source ON chapters (source_id);

-- Un chapitre ne peut être rangé que dans un volume de sa propre série.
CREATE TRIGGER chapters_volume_same_series_insert
BEFORE INSERT ON chapters
WHEN NEW.volume_id IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM volumes WHERE id = NEW.volume_id AND series_id = NEW.series_id)
BEGIN
    SELECT RAISE(ABORT, 'chapter volume belongs to another series');
END;

CREATE TRIGGER chapters_volume_same_series_update
BEFORE UPDATE OF volume_id, series_id ON chapters
WHEN NEW.volume_id IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM volumes WHERE id = NEW.volume_id AND series_id = NEW.series_id)
BEGIN
    SELECT RAISE(ABORT, 'chapter volume belongs to another series');
END;

-- entry_name : nom de l'entrée dans la source. C'est une clé de lecture,
-- jamais un chemin de destination.
CREATE TABLE pages (
    chapter_id TEXT NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    page_index INTEGER NOT NULL CHECK (page_index >= 0),
    entry_name TEXT NOT NULL,
    size_bytes INTEGER,
    width      INTEGER,
    height     INTEGER,
    PRIMARY KEY (chapter_id, page_index)
) STRICT, WITHOUT ROWID;

-- Collections -----------------------------------------------------------------

CREATE TABLE collections (
    id         TEXT PRIMARY KEY NOT NULL,
    system_key TEXT UNIQUE CHECK (system_key IN ('favorites', 'to_read', 'reading', 'completed', 'dropped')),
    name       TEXT NOT NULL CHECK (length(name) > 0),
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

INSERT INTO collections (id, system_key, name, position) VALUES
    ('system-favorites', 'favorites', 'Favoris', 0),
    ('system-to-read', 'to_read', 'À lire', 1),
    ('system-reading', 'reading', 'En cours', 2),
    ('system-completed', 'completed', 'Terminés', 3),
    ('system-dropped', 'dropped', 'Abandonnés', 4);

CREATE TRIGGER collections_system_no_delete
BEFORE DELETE ON collections
WHEN OLD.system_key IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'system collections cannot be deleted');
END;

CREATE TABLE collection_items (
    id            TEXT PRIMARY KEY NOT NULL,
    collection_id TEXT NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    series_id     TEXT REFERENCES series (id) ON DELETE CASCADE,
    chapter_id    TEXT REFERENCES chapters (id) ON DELETE CASCADE,
    position      INTEGER NOT NULL DEFAULT 0,
    added_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((series_id IS NULL) <> (chapter_id IS NULL))
) STRICT;
CREATE UNIQUE INDEX collection_items_series ON collection_items (collection_id, series_id) WHERE series_id IS NOT NULL;
CREATE UNIQUE INDEX collection_items_chapter ON collection_items (collection_id, chapter_id) WHERE chapter_id IS NOT NULL;
CREATE INDEX collection_items_order ON collection_items (collection_id, position);

-- Lecture ---------------------------------------------------------------------

-- page_offset : fraction (0-1) de la page déjà défilée. Contrairement à des
-- pixels, elle reste juste si la fenêtre ou le zoom change.
CREATE TABLE reading_progress (
    chapter_id   TEXT PRIMARY KEY NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    page_index   INTEGER NOT NULL DEFAULT 0 CHECK (page_index >= 0),
    page_offset  REAL NOT NULL DEFAULT 0 CHECK (page_offset BETWEEN 0 AND 1),
    percent      REAL NOT NULL DEFAULT 0 CHECK (percent BETWEEN 0 AND 100),
    completed_at TEXT,
    last_read_at TEXT NOT NULL
) STRICT, WITHOUT ROWID;
CREATE INDEX reading_progress_recent ON reading_progress (last_read_at);

CREATE TABLE reading_history (
    id         TEXT PRIMARY KEY NOT NULL,
    chapter_id TEXT NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at   TEXT,
    first_page INTEGER,
    last_page  INTEGER
) STRICT;
CREATE INDEX reading_history_chapter ON reading_history (chapter_id, started_at);
CREATE INDEX reading_history_recent ON reading_history (started_at);

-- Recherche -------------------------------------------------------------------

-- Index autonome : les clés TEXT n'ont pas de rowid stable après VACUUM, ce qui
-- exclut le mode "external content".
CREATE VIRTUAL TABLE search_index USING fts5 (
    series_id UNINDEXED,
    title,
    alt_titles,
    people,
    tokenize = "unicode61 remove_diacritics 2",
    prefix = '2 3'
);

CREATE TRIGGER series_search_insert AFTER INSERT ON series
BEGIN
    INSERT INTO search_index (series_id, title, alt_titles, people)
    VALUES (NEW.id, NEW.title, '', '');
END;

CREATE TRIGGER series_search_title AFTER UPDATE OF title ON series
BEGIN
    UPDATE search_index SET title = NEW.title WHERE series_id = NEW.id;
END;

CREATE TRIGGER series_search_delete AFTER DELETE ON series
BEGIN
    DELETE FROM search_index WHERE series_id = OLD.id;
END;

CREATE TRIGGER series_titles_search_insert AFTER INSERT ON series_titles
BEGIN
    UPDATE search_index
    SET alt_titles = (SELECT group_concat(title, ' ') FROM series_titles WHERE series_id = NEW.series_id)
    WHERE series_id = NEW.series_id;
END;

CREATE TRIGGER series_titles_search_delete AFTER DELETE ON series_titles
BEGIN
    UPDATE search_index
    SET alt_titles = coalesce((SELECT group_concat(title, ' ') FROM series_titles WHERE series_id = OLD.series_id), '')
    WHERE series_id = OLD.series_id;
END;

CREATE TRIGGER series_people_search_insert AFTER INSERT ON series_people
BEGIN
    UPDATE search_index
    SET people = (SELECT group_concat(p.name, ' ') FROM series_people sp
                  JOIN people p ON p.id = sp.person_id WHERE sp.series_id = NEW.series_id)
    WHERE series_id = NEW.series_id;
END;

CREATE TRIGGER series_people_search_delete AFTER DELETE ON series_people
BEGIN
    UPDATE search_index
    SET people = coalesce((SELECT group_concat(p.name, ' ') FROM series_people sp
                           JOIN people p ON p.id = sp.person_id WHERE sp.series_id = OLD.series_id), '')
    WHERE series_id = OLD.series_id;
END;
