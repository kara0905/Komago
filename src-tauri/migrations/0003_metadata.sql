-- Provenance des métadonnées : d'où vient chaque champ, et qui a le dernier mot.

CREATE TABLE metadata_sources (
    id   TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('archive', 'provider', 'manual'))
) STRICT;

INSERT INTO metadata_sources (id, name, kind) VALUES
    ('manual', 'Modification manuelle', 'manual'),
    ('comicinfo', 'ComicInfo.xml', 'archive'),
    ('mangadex', 'MangaDex', 'provider'),
    ('anilist', 'AniList', 'provider');

-- Association confirmée entre une série et une fiche externe.
CREATE TABLE external_ids (
    series_id    TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    source_id    TEXT NOT NULL REFERENCES metadata_sources (id),
    external_id  TEXT NOT NULL,
    url          TEXT,
    match_score  REAL,
    confirmed_at TEXT NOT NULL,
    PRIMARY KEY (series_id, source_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX external_ids_lookup ON external_ids (source_id, external_id);

-- Réponse brute d'une source, pour rejouer une fusion sans réseau.
CREATE TABLE metadata_snapshots (
    id          TEXT PRIMARY KEY NOT NULL,
    series_id   TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    source_id   TEXT NOT NULL REFERENCES metadata_sources (id),
    external_id TEXT,
    payload     TEXT NOT NULL CHECK (json_valid(payload)),
    fetched_at  TEXT NOT NULL
) STRICT;
CREATE INDEX metadata_snapshots_series ON metadata_snapshots (series_id, source_id, fetched_at);

-- Une ligne par champ d'une entité. locked = 1 : aucune actualisation
-- automatique ne peut écraser la valeur (posé par toute édition manuelle).
-- Entité polymorphe : le nettoyage est assuré par les triggers ci-dessous.
CREATE TABLE field_provenance (
    entity_type TEXT NOT NULL CHECK (entity_type IN ('series', 'volume', 'chapter')),
    entity_id   TEXT NOT NULL,
    field       TEXT NOT NULL,
    source_id   TEXT NOT NULL REFERENCES metadata_sources (id),
    snapshot_id TEXT REFERENCES metadata_snapshots (id) ON DELETE SET NULL,
    locked      INTEGER NOT NULL DEFAULT 0 CHECK (locked IN (0, 1)),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (entity_type, entity_id, field),
    CHECK (source_id <> 'manual' OR locked = 1)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER series_provenance_delete AFTER DELETE ON series
BEGIN
    DELETE FROM field_provenance WHERE entity_type = 'series' AND entity_id = OLD.id;
END;

CREATE TRIGGER volumes_provenance_delete AFTER DELETE ON volumes
BEGIN
    DELETE FROM field_provenance WHERE entity_type = 'volume' AND entity_id = OLD.id;
END;

CREATE TRIGGER chapters_provenance_delete AFTER DELETE ON chapters
BEGIN
    DELETE FROM field_provenance WHERE entity_type = 'chapter' AND entity_id = OLD.id;
END;
