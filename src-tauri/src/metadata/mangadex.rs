use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use super::http::{GuardedClient, HostRule, MAX_JSON_BYTES};
use super::{parse_json, safe_id, AltTitle, ChapterTitle, MetadataProvider, ProviderSeries, SeriesQuery};
use crate::error::{Error, Result};

const API: &str = "https://api.mangadex.org";
const INCLUDES: [(&str, &str); 3] = [("includes[]", "author"), ("includes[]", "artist"), ("includes[]", "cover_art")];
/// Langues préférées pour les textes.
const TEXT_LANGUAGES: [&str; 2] = ["fr", "en"];
const TITLE_LANGUAGES: [&str; 3] = ["en", "ja-ro", "fr"];

static SEPARATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)(?:---+|___+)(?:\s|$)").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());

pub struct MangaDex;

impl MetadataProvider for MangaDex {
    fn id(&self) -> &'static str {
        "mangadex"
    }

    fn name(&self) -> &'static str {
        "MangaDex"
    }

    // La documentation annonce environ 5 requêtes par seconde : on reste en dessous.
    fn hosts(&self) -> Vec<HostRule> {
        let min_interval = Duration::from_millis(300);
        vec![
            HostRule { host: "api.mangadex.org", subdomains: false, min_interval },
            HostRule { host: "uploads.mangadex.org", subdomains: false, min_interval },
        ]
    }

    fn site(&self) -> &'static str {
        "mangadex.org"
    }

    fn search_series(&self, query: &SeriesQuery, http: &GuardedClient) -> Result<Vec<ProviderSeries>> {
        let mut params = vec![("title", query.title.as_str()), ("limit", "10")];
        params.extend(INCLUDES);
        let url = reqwest::Url::parse_with_params(&format!("{API}/manga"), &params)
            .map_err(|_| Error::Network("adresse invalide".into()))?;
        let body = parse_json(&http.get(url.as_str(), MAX_JSON_BYTES)?)?;
        Ok(body["data"].as_array().map(|items| items.iter().filter_map(parse_manga).collect()).unwrap_or_default())
    }

    /// Chapitres traduits en français, sinon en anglais. Plusieurs traductions d'un
    /// même numéro : la première en français l'emporte.
    fn chapter_titles(&self, external_id: &str, http: &GuardedClient) -> Result<Vec<ChapterTitle>> {
        let id = safe_id(external_id)?;
        let mut titles: Vec<(ChapterTitle, bool)> = Vec::new();
        let mut offset = 0usize;
        loop {
            let offset_text = offset.to_string();
            let params = [
                ("translatedLanguage[]", "fr"),
                ("translatedLanguage[]", "en"),
                ("order[chapter]", "asc"),
                ("limit", "500"),
                ("offset", offset_text.as_str()),
            ];
            let url = reqwest::Url::parse_with_params(&format!("{API}/manga/{id}/feed"), &params)
                .map_err(|_| Error::Network("adresse invalide".into()))?;
            let body = parse_json(&http.get(url.as_str(), MAX_JSON_BYTES)?)?;
            let items = body["data"].as_array().cloned().unwrap_or_default();
            for item in &items {
                let attributes = &item["attributes"];
                let number = attributes["chapter"].as_str().and_then(|n| n.replace(',', ".").parse::<f64>().ok());
                let title = attributes["title"].as_str().map(str::trim).filter(|t| !t.is_empty());
                let (Some(number), Some(title)) = (number, title) else { continue };
                let french = attributes["translatedLanguage"] == "fr";
                match titles.iter_mut().find(|(t, _)| t.number == number) {
                    Some(existing) if french && !existing.1 => *existing = (ChapterTitle { number, title: title.to_string() }, true),
                    Some(_) => {}
                    None => titles.push((ChapterTitle { number, title: title.to_string() }, french)),
                }
            }
            let total = body["total"].as_u64().unwrap_or(0) as usize;
            offset += items.len();
            // La pagination de MangaDex s'arrête à 10 000 éléments.
            if items.is_empty() || offset >= total || offset >= 9_500 {
                break;
            }
        }
        Ok(titles.into_iter().map(|(t, _)| t).collect())
    }

    fn get_series(&self, external_id: &str, http: &GuardedClient) -> Result<ProviderSeries> {
        let url = reqwest::Url::parse_with_params(&format!("{API}/manga/{}", safe_id(external_id)?), &INCLUDES)
            .map_err(|_| Error::Network("adresse invalide".into()))?;
        let body = parse_json(&http.get(url.as_str(), MAX_JSON_BYTES)?)?;
        parse_manga(&body["data"]).ok_or_else(|| Error::Network("réponse illisible de MangaDex".into()))
    }
}

fn parse_manga(manga: &Value) -> Option<ProviderSeries> {
    let id = manga["id"].as_str()?;
    let attributes = &manga["attributes"];
    let titles = attributes["title"].as_object()?;

    let title = TITLE_LANGUAGES
        .iter()
        .find_map(|language| titles.get(*language))
        .or_else(|| titles.values().next())?
        .as_str()?
        .to_string();

    let mut alt_titles = Vec::new();
    let mut add = |language: &str, value: &Value| {
        if let Some(text) = value.as_str().filter(|t| *t != title) {
            if !alt_titles.iter().any(|a: &AltTitle| a.title == text) {
                alt_titles.push(AltTitle { title: text.to_string(), language: language.to_string() });
            }
        }
    };
    for (language, value) in titles {
        add(language, value);
    }
    for entry in attributes["altTitles"].as_array().into_iter().flatten() {
        for (language, value) in entry.as_object().into_iter().flatten() {
            add(language, value);
        }
    }

    let description = TEXT_LANGUAGES
        .iter()
        .find_map(|language| attributes["description"][*language].as_str())
        .map(clean_description)
        .filter(|text| !text.is_empty());

    let (mut genres, mut tags) = (Vec::new(), Vec::new());
    for tag in attributes["tags"].as_array().into_iter().flatten() {
        let Some(name) = tag["attributes"]["name"]["en"].as_str() else { continue };
        if tag["attributes"]["group"] == "genre" {
            genres.push(name.to_string());
        } else {
            tags.push(name.to_string());
        }
    }

    let (mut authors, mut artists, mut cover_url) = (Vec::new(), Vec::new(), None);
    for relation in manga["relationships"].as_array().into_iter().flatten() {
        let attrs = &relation["attributes"];
        match relation["type"].as_str() {
            Some("author") => authors.extend(attrs["name"].as_str().map(str::to_string)),
            Some("artist") => artists.extend(attrs["name"].as_str().map(str::to_string)),
            Some("cover_art") => {
                cover_url = attrs["fileName"]
                    .as_str()
                    .map(|file| format!("https://uploads.mangadex.org/covers/{id}/{file}.512.jpg"));
            }
            _ => {}
        }
    }

    let original_language = attributes["originalLanguage"].as_str().map(str::to_string);
    let kind = match original_language.as_deref() {
        Some("ja") => Some("manga"),
        Some("ko") => Some("manhwa"),
        Some("zh" | "zh-hk") => Some("manhua"),
        _ => None,
    };
    let status = match attributes["status"].as_str() {
        Some("ongoing") => Some("ongoing"),
        Some("completed") => Some("completed"),
        Some("hiatus") => Some("hiatus"),
        Some("cancelled") => Some("cancelled"),
        _ => None,
    };

    Some(ProviderSeries {
        external_id: id.to_string(),
        url: Some(format!("https://mangadex.org/title/{id}")),
        title,
        alt_titles,
        description,
        cover_url,
        authors,
        artists,
        genres,
        tags,
        status: status.map(str::to_string),
        kind: kind.map(str::to_string),
        original_language,
        publisher: None,
        published_from: attributes["year"].as_i64().map(|y| y.to_string()),
        published_to: None,
        payload: manga.clone(),
    })
}

/// Les descriptions sont en Markdown, souvent suivies de liens et de distinctions
/// après un séparateur : seul le résumé est gardé, en texte simple.
fn clean_description(text: &str) -> String {
    let summary = SEPARATOR.split(text).next().unwrap_or(text);
    let without_links = LINK.replace_all(summary, "$1");
    without_links.replace("**", "").replace("__", "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::http::test_support::{rule, FakeTransport};

    // Forme relevée sur l'API réelle le 31/10/2026, textes raccourcis.
    const SEARCH: &str = r#"{"result":"ok","data":[{
        "id":"5d1fc77e-706a-4fc5-bea8-486c9be0145d","type":"manga",
        "attributes":{
          "title":{"ja-ro":"Vinland Saga"},
          "altTitles":[{"ja":"ヴィンランド・サガ"},{"zh":"海盗战记"},{"en":"Vinland Saga"}],
          "description":{"en":"A viking story.","ru":"История"},
          "status":"completed","year":2005,"originalLanguage":"ja",
          "tags":[{"attributes":{"group":"format","name":{"en":"Award Winning"}}},
                  {"attributes":{"group":"genre","name":{"en":"Historical"}}},
                  {"attributes":{"group":"theme","name":{"en":"Vikings"}}}]
        },
        "relationships":[
          {"type":"author","attributes":{"name":"Yukimura Makoto"}},
          {"type":"artist","attributes":{"name":"Yukimura Makoto"}},
          {"type":"cover_art","attributes":{"fileName":"7fa6.jpg"}},
          {"type":"manga","related":"sequel"}]
      },{"id":"sans-attributs"}]}"#;

    fn client(fake: &FakeTransport) -> GuardedClient {
        GuardedClient::new(Box::new(fake.clone()), vec![rule("api.mangadex.org", false)], true)
    }

    #[test]
    fn search_maps_the_real_response_shape() {
        let fake = FakeTransport::with(vec![(200, SEARCH.as_bytes().to_vec())]);
        let query = SeriesQuery { title: "Vinland & Saga".into(), ..Default::default() };
        let results = MangaDex.search_series(&query, &client(&fake)).unwrap();

        assert_eq!(results.len(), 1, "une entrée incomplète est ignorée");
        let manga = &results[0];
        assert_eq!(manga.title, "Vinland Saga");
        assert_eq!(manga.external_id, "5d1fc77e-706a-4fc5-bea8-486c9be0145d");
        assert_eq!(manga.url.as_deref(), Some("https://mangadex.org/title/5d1fc77e-706a-4fc5-bea8-486c9be0145d"));
        assert_eq!(manga.description.as_deref(), Some("A viking story."));
        assert_eq!(manga.authors, ["Yukimura Makoto"]);
        assert_eq!(manga.artists, ["Yukimura Makoto"]);
        assert_eq!(manga.genres, ["Historical"]);
        assert_eq!(manga.tags, ["Award Winning", "Vikings"]);
        assert_eq!(manga.status.as_deref(), Some("completed"));
        assert_eq!(manga.kind.as_deref(), Some("manga"));
        assert_eq!(manga.published_from.as_deref(), Some("2005"));
        assert!(manga.cover_url.as_deref().unwrap().starts_with("https://uploads.mangadex.org/covers/5d1fc77e"));
        let alts: Vec<_> = manga.alt_titles.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(alts, ["ヴィンランド・サガ", "海盗战记"], "le titre principal n'est pas répété");

        let url = fake.requests.lock().unwrap()[0].url.clone();
        assert!(url.contains("title=Vinland+%26+Saga"), "{url}");
        assert!(url.contains("includes%5B%5D=cover_art"), "{url}");
    }

    #[test]
    fn descriptions_become_plain_text() {
        assert_eq!(
            clean_description("A **viking** [tale](https://x.test).\n\n___ - **Won the Prize**"),
            "A viking tale."
        );
        assert_eq!(clean_description("Before\n---\nAfter"), "Before");
        assert_eq!(clean_description("Rien à retirer - vraiment"), "Rien à retirer - vraiment");
        assert_eq!(clean_description("___ seulement"), "");
    }

    #[test]
    fn chapter_titles_prefer_french_and_skip_untitled() {
        let page = r#"{"total":4,"data":[
            {"attributes":{"chapter":"1","title":"The Start","translatedLanguage":"en"}},
            {"attributes":{"chapter":"1","title":"Le début","translatedLanguage":"fr"}},
            {"attributes":{"chapter":"2","title":"","translatedLanguage":"en"}},
            {"attributes":{"chapter":"2.5","title":"Extra","translatedLanguage":"en"}}]}"#;
        let fake = FakeTransport::with(vec![(200, page.as_bytes().to_vec())]);
        let titles = MangaDex.chapter_titles("abc-1", &client(&fake)).unwrap();
        assert_eq!(
            titles,
            [ChapterTitle { number: 1.0, title: "Le début".into() }, ChapterTitle { number: 2.5, title: "Extra".into() }]
        );
        assert_eq!(fake.requests.lock().unwrap().len(), 1, "une seule page suffit");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let fake = FakeTransport::with(vec![(200, b"<html>".to_vec()), (200, b"{\"data\":{}}".to_vec()), (200, b"{}".to_vec())]);
        let http = client(&fake);
        assert!(MangaDex.search_series(&SeriesQuery::default(), &http).is_err());
        assert!(MangaDex.get_series("abc", &http).is_err());
        assert!(MangaDex.search_series(&SeriesQuery::default(), &http).unwrap().is_empty());
    }

    #[test]
    fn a_stored_id_cannot_change_the_request_path() {
        let fake = FakeTransport::default();
        assert!(MangaDex.get_series("../admin", &client(&fake)).is_err());
        assert!(fake.requests.lock().unwrap().is_empty());
    }
}
