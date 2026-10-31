//! Lecture de ComicInfo.xml. quick-xml ne résout aucune entité externe et le
//! fichier est plafonné en taille avant analyse.

use quick_xml::events::Event;
use quick_xml::Reader;

pub const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComicInfo {
    pub series: Option<String>,
    pub title: Option<String>,
    pub number: Option<f64>,
    pub volume: Option<f64>,
    pub summary: Option<String>,
    pub publisher: Option<String>,
    pub language: Option<String>,
    pub published_on: Option<String>,
    pub writers: Vec<String>,
    pub artists: Vec<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub right_to_left: bool,
    pub is_manga: bool,
}

/// Un fichier invalide donne `None` : il ne doit jamais bloquer l'import.
pub fn parse(bytes: &[u8]) -> Option<ComicInfo> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let mut reader = Reader::from_str(text);
    let mut info = ComicInfo::default();
    let (mut year, mut month, mut day) = (None, None, None);
    let mut depth = 0usize;
    let mut current: Option<String> = None;
    let mut buffer = String::new();
    let mut seen_root = false;

    loop {
        match reader.read_event().ok()? {
            Event::Start(start) => {
                depth += 1;
                let local = start.local_name();
                let name: &str = local.as_ref();
                if depth == 1 {
                    if name != "ComicInfo" {
                        return None;
                    }
                    seen_root = true;
                } else if depth == 2 {
                    current = Some(name.to_string());
                    buffer.clear();
                }
            }
            Event::End(_) => {
                if depth == 2 {
                    if let Some(field) = current.take() {
                        let value = buffer.trim();
                        if !value.is_empty() {
                            apply(&mut info, &field, value, &mut year, &mut month, &mut day);
                        }
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) if depth == 2 => buffer.push_str(&text.xml10_content()),
            Event::CData(data) if depth == 2 => buffer.push_str(&data.xml10_content()),
            // Seules les références standard sont résolues : une entité déclarée
            // par le fichier (DOCTYPE) est ignorée.
            Event::GeneralRef(reference) if depth == 2 => {
                if let Ok(Some(c)) = reference.resolve_char_ref() {
                    buffer.push(c);
                } else {
                    match reference.xml10_content().as_ref() {
                        "amp" => buffer.push('&'),
                        "lt" => buffer.push('<'),
                        "gt" => buffer.push('>'),
                        "quot" => buffer.push('"'),
                        "apos" => buffer.push('\''),
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_root {
        return None;
    }
    info.published_on = match (year, month, day) {
        (Some(y), Some(m), Some(d)) => Some(format!("{y:04}-{m:02}-{d:02}")),
        (Some(y), Some(m), None) => Some(format!("{y:04}-{m:02}")),
        (Some(y), _, _) => Some(format!("{y:04}")),
        _ => None,
    };
    Some(info)
}

fn apply(
    info: &mut ComicInfo,
    field: &str,
    value: &str,
    year: &mut Option<u32>,
    month: &mut Option<u32>,
    day: &mut Option<u32>,
) {
    let text = || Some(value.to_string());
    match field {
        "Series" => info.series = text(),
        "Title" => info.title = text(),
        "Number" => info.number = parse_number(value),
        "Volume" => info.volume = parse_number(value),
        "Summary" => info.summary = text(),
        "Publisher" => info.publisher = text(),
        "LanguageISO" => info.language = text(),
        "Writer" => info.writers.extend(split_list(value)),
        "Penciller" | "Inker" | "Colorist" | "CoverArtist" => {
            for name in split_list(value) {
                if !info.artists.contains(&name) {
                    info.artists.push(name);
                }
            }
        }
        "Genre" => info.genres.extend(split_list(value)),
        "Tags" => info.tags.extend(split_list(value)),
        "Year" => *year = value.parse::<u32>().ok().filter(|y| *y > 0),
        "Month" => *month = value.parse::<u32>().ok().filter(|m| (1..=12).contains(m)),
        "Day" => *day = value.parse::<u32>().ok().filter(|d| (1..=31).contains(d)),
        "Manga" => {
            info.right_to_left = value.eq_ignore_ascii_case("YesAndRightToLeft");
            info.is_manga = info.right_to_left || value.eq_ignore_ascii_case("Yes");
        }
        _ => {}
    }
}

fn parse_number(value: &str) -> Option<f64> {
    value.replace(',', ".").parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_useful_fields() {
        let xml = br#"<?xml version="1.0"?>
            <ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
              <Title>Le d&#233;part &amp; la suite</Title>
              <Series>Ma S&#xE9;rie</Series>
              <Number>10.5</Number>
              <Volume>2</Volume>
              <Summary>R&#233;sum&#233;</Summary>
              <Writer>Auteur A, Auteur B</Writer>
              <Penciller>Dessin</Penciller>
              <Inker>Dessin</Inker>
              <Genre>Action; Drame</Genre>
              <LanguageISO>fr</LanguageISO>
              <Year>2019</Year><Month>4</Month>
              <Manga>YesAndRightToLeft</Manga>
            </ComicInfo>"#;
        let info = parse(xml).unwrap();

        assert_eq!(info.series.as_deref(), Some("Ma Série"));
        assert_eq!(info.title.as_deref(), Some("Le départ & la suite"));
        assert_eq!(info.number, Some(10.5));
        assert_eq!(info.volume, Some(2.0));
        assert_eq!(info.writers, ["Auteur A", "Auteur B"]);
        assert_eq!(info.artists, ["Dessin"]);
        assert_eq!(info.genres, ["Action", "Drame"]);
        assert_eq!(info.published_on.as_deref(), Some("2019-04"));
        assert!(info.right_to_left && info.is_manga);
    }

    #[test]
    fn invalid_files_are_ignored_without_failing() {
        assert_eq!(parse(b"pas du xml <"), None);
        assert_eq!(parse(b"<Autre><Series>x</Series></Autre>"), None);
        assert_eq!(parse(&[0xFF, 0xFE, 0x00]), None);
        assert_eq!(parse(&vec![b' '; MAX_BYTES + 1]), None);
    }

    #[test]
    fn external_entities_are_not_resolved() {
        let xml = br#"<!DOCTYPE c [<!ENTITY x SYSTEM "file:///etc/passwd">]>
            <ComicInfo><Series>&x;</Series><Title>Ok</Title></ComicInfo>"#;
        let info = parse(xml).unwrap();
        assert_eq!(info.series, None);
        assert_eq!(info.title.as_deref(), Some("Ok"));
    }
}
