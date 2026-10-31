-- Fiches écartées par l'utilisateur : elles ne sont plus proposées pour cette série.
CREATE TABLE rejected_matches (
    series_id   TEXT NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    source_id   TEXT NOT NULL REFERENCES metadata_sources (id),
    external_id TEXT NOT NULL,
    PRIMARY KEY (series_id, source_id, external_id)
) STRICT, WITHOUT ROWID;
