use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};

use super::http::{GuardedClient, HostRule, MAX_JSON_BYTES};
use super::{parse_json, safe_id, AltTitle, MetadataProvider, ProviderSeries, SeriesQuery};
use crate::error::{Error, Result};

const API: &str = "https://graphql.anilist.co";
const FIELDS: &str = "id siteUrl format status countryOfOrigin
    title { romaji english native } synonyms description(asHtml: false)
    coverImage { extraLarge large } genres tags { name isMediaSpoiler }
    startDate { year month day } endDate { year month day }
    staff(perPage: 12) { edges { role node { name { full } } } }";

static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());

pub struct AniList;

impl MetadataProvider for AniList {
    fn id(&self) -> &'static str {
        "anilist"
    }

    fn name(&self) -> &'static str {
        "AniList"
    }

    // 90 requêtes par minute, réduites à 30 au 31/10/2026 : on vise ce plus bas.
    fn hosts(&self) -> Vec<HostRule> {
        vec![
            HostRule { host: "graphql.anilist.co", subdomains: false, min_interval: Duration::from_millis(2100) },
            HostRule { host: "anilist.co", subdomains: true, min_interval: Duration::from_millis(200) },
        ]
    }

    fn site(&self) -> &'static str {
        "anilist.co"
    }

    fn search_series(&self, query: &SeriesQuery, http: &GuardedClient) -> Result<Vec<ProviderSeries>> {
        let document = format!(
            "query ($search: String) {{ Page(perPage: 10) {{ media(search: $search, type: MANGA) {{ {FIELDS} }} }} }}"
        );
        let body = request(http, &document, json!({ "search": query.title }))?;
        Ok(body["data"]["Page"]["media"]
            .as_array()
            .map(|items| items.iter().filter_map(parse_media).collect())
            .unwrap_or_default())
    }

    fn get_series(&self, external_id: &str, http: &GuardedClient) -> Result<ProviderSeries> {
        let id: i64 = safe_id(external_id)?.parse().map_err(|_| Error::Invalid("identifiant AniList invalide".into()))?;
        let document = format!("query ($id: Int) {{ Media(id: $id, type: MANGA) {{ {FIELDS} }} }}");
        let body = request(http, &document, json!({ "id": id }))?;
        parse_media(&body["data"]["Media"]).ok_or_else(|| Error::Network("réponse illisible d'AniList".into()))
    }
}

fn request(http: &GuardedClient, query: &str, variables: Value) -> Result<Value> {
    let payload = json!({ "query": query, "variables": variables }).to_string();
    let body = parse_json(&http.post_json(API, payload, MAX_JSON_BYTES)?)?;
    if let Some(message) = body["errors"][0]["message"].as_str() {
        return Err(Error::Network(format!("AniList a refusé la requête : {message}")));
    }
    Ok(body)
}

fn parse_media(media: &Value) -> Option<ProviderSeries> {
    let id = media["id"].as_i64()?;
    // Romans et light novels ne sont pas des œuvres que Komago peut contenir.
    if media["format"] == "NOVEL" {
        return None;
    }
    let names = &media["title"];
    let english = names["english"].as_str();
    let romaji = names["romaji"].as_str();
    let native = names["native"].as_str();
    let title = english.or(romaji).or(native)?.to_string();

    let mut alt_titles = Vec::new();
    let mut add = |text: Option<&str>, language: &str| {
        if let Some(text) = text.filter(|t| *t != title) {
            if !alt_titles.iter().any(|a: &AltTitle| a.title == text) {
                alt_titles.push(AltTitle { title: text.to_string(), language: language.to_string() });
            }
        }
    };
    add(romaji, "ja-ro");
    add(native, "");
    add(english, "en");
    for synonym in media["synonyms"].as_array().into_iter().flatten() {
        add(synonym.as_str(), "");
    }

    let (mut authors, mut artists) = (Vec::new(), Vec::new());
    for edge in media["staff"]["edges"].as_array().into_iter().flatten() {
        let (Some(role), Some(name)) = (edge["role"].as_str(), edge["node"]["name"]["full"].as_str()) else {
            continue;
        };
        // Traducteurs et lettreurs des éditions locales ne sont pas les auteurs.
        if role.contains('(') {
            continue;
        }
        if role.contains("Story") && !authors.iter().any(|a| a == name) {
            authors.push(name.to_string());
        }
        if role.contains("Art") && !artists.iter().any(|a| a == name) {
            artists.push(name.to_string());
        }
    }

    let tags = media["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tag| tag["isMediaSpoiler"] != true)
        .filter_map(|tag| tag["name"].as_str().map(str::to_string))
        .collect();
    let genres = media["genres"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(str::to_string)).collect();

    let country = media["countryOfOrigin"].as_str();
    let kind = match country {
        Some("JP") => Some("manga"),
        Some("KR") => Some("manhwa"),
        Some("CN" | "TW") => Some("manhua"),
        _ => None,
    };
    let original_language = match country {
        Some("JP") => Some("ja"),
        Some("KR") => Some("ko"),
        Some("CN" | "TW") => Some("zh"),
        _ => None,
    };
    let status = match media["status"].as_str() {
        Some("FINISHED") => Some("completed"),
        Some("RELEASING") => Some("ongoing"),
        Some("HIATUS") => Some("hiatus"),
        Some("CANCELLED") => Some("cancelled"),
        _ => None,
    };

    Some(ProviderSeries {
        external_id: id.to_string(),
        url: media["siteUrl"].as_str().map(str::to_string),
        title,
        alt_titles,
        description: media["description"].as_str().map(clean_description).filter(|d| !d.is_empty()),
        cover_url: media["coverImage"]["extraLarge"].as_str().or(media["coverImage"]["large"].as_str()).map(str::to_string),
        authors,
        artists,
        genres,
        tags,
        status: status.map(str::to_string),
        kind: kind.map(str::to_string),
        original_language: original_language.map(str::to_string),
        publisher: None,
        published_from: date(&media["startDate"]),
        published_to: date(&media["endDate"]),
        payload: media.clone(),
    })
}

/// Le synopsis contient parfois du HTML malgré `asHtml: false`.
fn clean_description(text: &str) -> String {
    let with_breaks = text.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
    TAGS.replace_all(&with_breaks, "")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .trim()
        .to_string()
}

fn date(value: &Value) -> Option<String> {
    let year = value["year"].as_i64()?;
    Some(match (value["month"].as_i64(), value["day"].as_i64()) {
        (Some(month), Some(day)) => format!("{year:04}-{month:02}-{day:02}"),
        (Some(month), None) => format!("{year:04}-{month:02}"),
        _ => format!("{year:04}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::http::test_support::{rule, FakeTransport};

    // Forme relevée sur l'API réelle le 31/10/2026, textes raccourcis.
    const SEARCH: &str = r#"{"data":{"Page":{"media":[{
        "id":30642,"siteUrl":"https://anilist.co/manga/30642","format":"MANGA","status":"FINISHED","countryOfOrigin":"JP",
        "title":{"romaji":"Vinland Saga","english":"Vinland Saga","native":"ヴィンランド・サガ"},
        "synonyms":["Saga Winlandzka","빈란드 사가"],
        "description":"A viking<br>story &amp; <i>more</i>.",
        "coverImage":{"extraLarge":"https://s4.anilist.co/file/x.jpg"},
        "genres":["Action","Drama"],
        "tags":[{"name":"Rehabilitation","isMediaSpoiler":true},{"name":"Historical","isMediaSpoiler":false}],
        "startDate":{"year":2005,"month":4,"day":27},"endDate":{"year":2025,"month":7,"day":null},
        "staff":{"edges":[
          {"role":"Story & Art","node":{"name":{"full":"Makoto Yukimura"}}},
          {"role":"Lettering (English)","node":{"name":{"full":"Scott Brown"}}},
          {"role":"Translator (French)","node":{"name":{"full":"X"}}}]}
      },{"id":2,"format":"NOVEL","title":{"english":"Roman"}}]}}}"#;

    fn client(fake: &FakeTransport) -> GuardedClient {
        GuardedClient::new(Box::new(fake.clone()), vec![rule("graphql.anilist.co", false)], true)
    }

    #[test]
    fn search_maps_the_real_response_shape() {
        let fake = FakeTransport::with(vec![(200, SEARCH.as_bytes().to_vec())]);
        let query = SeriesQuery { title: "Vinland \"Saga\"".into(), ..Default::default() };
        let results = AniList.search_series(&query, &client(&fake)).unwrap();

        assert_eq!(results.len(), 1, "les romans sont écartés");
        let media = &results[0];
        assert_eq!(media.title, "Vinland Saga");
        assert_eq!(media.external_id, "30642");
        assert_eq!(media.description.as_deref(), Some("A viking\nstory & more."));
        assert_eq!(media.authors, ["Makoto Yukimura"]);
        assert_eq!(media.artists, ["Makoto Yukimura"], "ni lettreur ni traducteur");
        assert_eq!(media.tags, ["Historical"], "pas de tag qui divulgâche");
        assert_eq!(media.genres, ["Action", "Drama"]);
        assert_eq!(media.status.as_deref(), Some("completed"));
        assert_eq!(media.kind.as_deref(), Some("manga"));
        assert_eq!(media.published_from.as_deref(), Some("2005-04-27"));
        assert_eq!(media.published_to.as_deref(), Some("2025-07"));
        let alts: Vec<_> = media.alt_titles.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(alts, ["ヴィンランド・サガ", "Saga Winlandzka", "빈란드 사가"]);

        // La saisie part en variable GraphQL, jamais dans le texte de la requête.
        let sent: Value = serde_json::from_str(fake.requests.lock().unwrap()[0].json_body.as_ref().unwrap()).unwrap();
        assert_eq!(sent["variables"]["search"], "Vinland \"Saga\"");
        assert!(!sent["query"].as_str().unwrap().contains("Vinland"));
    }

    #[test]
    fn graphql_errors_are_reported() {
        let body = br#"{"data":null,"errors":[{"message":"Too Many Requests.","status":429}]}"#;
        let fake = FakeTransport::with(vec![(200, body.to_vec())]);
        let error = AniList.search_series(&SeriesQuery::default(), &client(&fake)).unwrap_err();
        assert!(error.to_string().contains("Too Many Requests"));
    }

    #[test]
    fn non_numeric_ids_are_refused() {
        let fake = FakeTransport::default();
        assert!(AniList.get_series("abc", &client(&fake)).is_err());
        assert!(fake.requests.lock().unwrap().is_empty());
    }
}
