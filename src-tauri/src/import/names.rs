//! Déduction d'informations à partir des noms de fichiers et de dossiers.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedName {
    /// Texte qui précède le numéro : probable titre de la série.
    pub series_hint: Option<String>,
    pub volume: Option<f64>,
    pub chapter: Option<f64>,
    /// Texte qui suit le numéro de chapitre.
    pub title: Option<String>,
}

impl ParsedName {
    /// Un dossier nommé « Volume 01 » regroupe des chapitres : ce n'est pas une série.
    pub fn is_volume_only(&self) -> bool {
        self.volume.is_some() && self.chapter.is_none()
    }
}

static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\([^)]*\)|\{[^}]*\}").unwrap());
static VOLUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[\s\-.,])(?:volume|vol\.?|tome|t\.?|v\.?)\s*(\d+(?:[.,]\d+)?)(?:$|[\s\-.,:])").unwrap()
});
static CHAPTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[\s\-.,])(?:chapter|chapitre|chap\.?|ch\.?|c|episode|épisode|ep\.?|issue|#)\s*(\d+(?:[.,]\d+)?)(?:$|[\s\-.,:])").unwrap()
});
static TRAILING_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(.*?)[\s\-#]+)?(\d+(?:[.,]\d+)?)$").unwrap());

pub fn parse_name(raw: &str) -> ParsedName {
    let cleaned = clean(raw);
    let mut parsed = ParsedName::default();
    let mut hint_end = cleaned.len();
    let mut after = None;

    if let Some(found) = VOLUME.captures(&cleaned) {
        let whole = found.get(0).unwrap();
        parsed.volume = number(found.get(1).unwrap().as_str());
        hint_end = hint_end.min(whole.start());
        after = Some(whole.end());
    }
    if let Some(found) = CHAPTER.captures(&cleaned) {
        let whole = found.get(0).unwrap();
        parsed.chapter = number(found.get(1).unwrap().as_str());
        hint_end = hint_end.min(whole.start());
        after = Some(whole.end());
    }

    if parsed.volume.is_none() && parsed.chapter.is_none() {
        if let Some(found) = TRAILING_NUMBER.captures(&cleaned) {
            let value = number(found.get(2).unwrap().as_str());
            // « Blade Runner 2049 » : une année probable n'est pas un numéro.
            let looks_like_year = found.get(1).is_some() && value.is_some_and(|n| (1900.0..=2100.0).contains(&n));
            if !looks_like_year {
                parsed.chapter = value;
                parsed.series_hint = found.get(1).map(|m| trim_separators(m.as_str())).filter(|s| !s.is_empty());
                return parsed;
            }
        }
        parsed.series_hint = Some(cleaned.clone()).filter(|s| !s.is_empty());
        return parsed;
    }

    parsed.series_hint = Some(trim_separators(&cleaned[..hint_end])).filter(|s| !s.is_empty());
    if let Some(start) = after {
        parsed.title = cleaned.get(start..).map(trim_separators).filter(|s| !s.is_empty());
    }
    parsed
}

/// Retire balises de groupe, années et séparateurs de type `_`.
pub fn clean(raw: &str) -> String {
    let without_tags = TAGS.replace_all(raw, " ");
    let spaced = without_tags.replace('_', " ");
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trim_separators(text: &str) -> String {
    text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | ':' | '.' | ',')).to_string()
}

fn number(text: &str) -> Option<f64> {
    text.replace(',', ".").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn check(raw: &str, hint: Option<&str>, volume: Option<f64>, chapter: Option<f64>) {
        let parsed = parse_name(raw);
        assert_eq!(parsed.series_hint.as_deref(), hint, "titre de « {raw} »");
        assert_eq!(parsed.volume, volume, "volume de « {raw} »");
        assert_eq!(parsed.chapter, chapter, "chapitre de « {raw} »");
    }

    #[test]
    fn explicit_markers() {
        check("Chapter 001", None, None, Some(1.0));
        check("Chapitre 12.5", None, None, Some(12.5));
        check("Volume 01", None, Some(1.0), None);
        check("Tome 3", None, Some(3.0), None);
        check("One Piece - Vol.12 Ch.105", Some("One Piece"), Some(12.0), Some(105.0));
        check("Naruto v05 c042", Some("Naruto"), Some(5.0), Some(42.0));
        check("Ma Série Tome 3", Some("Ma Série"), Some(3.0), None);
        check("Batman #12", Some("Batman"), None, Some(12.0));
    }

    #[test]
    fn group_tags_and_underscores_are_removed() {
        check("[Group] Berserk_c100 (2020) [HQ]", Some("Berserk"), None, Some(100.0));
    }

    #[test]
    fn trailing_numbers() {
        check("Dandadan 012", Some("Dandadan"), None, Some(12.0));
        check("Dandadan - 012", Some("Dandadan"), None, Some(12.0));
        check("012", None, None, Some(12.0));
    }

    #[test]
    fn ambiguous_names_stay_undecided() {
        check("Blade Runner 2049", Some("Blade Runner 2049"), None, None);
        check("Art Of War", Some("Art Of War"), None, None);
        check("Part5", Some("Part5"), None, None);
        check("Vinland Saga", Some("Vinland Saga"), None, None);
    }

    #[test]
    fn title_after_the_number_is_kept() {
        let parsed = parse_name("Chapter 3 - Le départ");
        assert_eq!(parsed.chapter, Some(3.0));
        assert_eq!(parsed.title.as_deref(), Some("Le départ"));
    }

    #[test]
    fn decimal_comma_is_understood() {
        assert_eq!(parse_name("Chapitre 10,5").chapter, Some(10.5));
    }

    #[test]
    fn volume_only_detection() {
        assert!(parse_name("Volume 2").is_volume_only());
        assert!(!parse_name("Chapter 2").is_volume_only());
        assert!(!parse_name("Berserk").is_volume_only());
    }
}
