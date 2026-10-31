-- Un même emplacement d'une source ne peut produire qu'un seul chapitre :
-- réimporter un dossier ne crée pas de doublons.
CREATE UNIQUE INDEX chapters_source_location
    ON chapters (source_id, source_subpath)
    WHERE source_id IS NOT NULL;
