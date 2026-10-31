//! Score de correspondance entre une série locale et une fiche externe. Il sert à
//! classer et à signaler ; aucune association n'est faite sans confirmation.

use strsim::{normalized_levenshtein, sorensen_dice};

use super::{ProviderSeries, SeriesQuery};
use crate::text::normalize_title;

const TITLE_WEIGHT: f64 = 0.7;
const AUTHOR_WEIGHT: f64 = 0.2;
const KIND_WEIGHT: f64 = 0.1;
const CERTAIN_SCORE: f64 = 0.9;
/// Deux fiches trop proches du même fournisseur restent à départager par l'utilisateur.
const CERTAIN_GAP: f64 = 0.1;

pub fn title_similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (normalize_title(a), normalize_title(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    // Jamais 1 sans égalité : « Berserk » n'est pas exactement « Berserk Deluxe ».
    (normalized_levenshtein(&a, &b).max(sorensen_dice(&a, &b))).min(0.97)
}

/// « Yukimura Makoto » et « Makoto Yukimura » sont la même personne.
fn name_key(name: &str) -> String {
    let normalized = normalize_title(name);
    let mut parts: Vec<_> = normalized.split(' ').collect();
    parts.sort_unstable();
    parts.join(" ")
}

fn author_similarity(local: &[String], remote: &[String]) -> f64 {
    let mut best: f64 = 0.0;
    for a in local {
        for b in remote {
            best = best.max(normalized_levenshtein(&name_key(a), &name_key(b)));
        }
    }
    best
}

pub fn score(query: &SeriesQuery, candidate: &ProviderSeries) -> f64 {
    let local_titles = std::iter::once(&query.title).chain(query.alt_titles.iter());
    let mut title: f64 = 0.0;
    for local in local_titles {
        title = title.max(title_similarity(local, &candidate.title));
        for alternate in &candidate.alt_titles {
            title = title.max(title_similarity(local, &alternate.title));
        }
    }

    // Seuls les indices connus des deux côtés comptent : l'absence d'auteur local
    // ne doit pas pénaliser une fiche.
    let mut total = title * TITLE_WEIGHT;
    let mut weight = TITLE_WEIGHT;
    let remote_people: Vec<String> = candidate.authors.iter().chain(&candidate.artists).cloned().collect();
    if !query.authors.is_empty() && !remote_people.is_empty() {
        total += author_similarity(&query.authors, &remote_people) * AUTHOR_WEIGHT;
        weight += AUTHOR_WEIGHT;
    }
    if let (Some(local), Some(remote)) = (&query.kind, &candidate.kind) {
        total += if local == remote { KIND_WEIGHT } else { 0.0 };
        weight += KIND_WEIGHT;
    }
    total / weight
}

/// Une correspondance n'est « très probable » que si elle est forte et nettement
/// devant les autres fiches du même fournisseur.
pub fn is_certain(score: f64, next_from_same_provider: Option<f64>) -> bool {
    score >= CERTAIN_SCORE && next_from_same_provider.is_none_or(|next| score - next >= CERTAIN_GAP)
}

#[cfg(test)]
mod tests {
    use super::super::AltTitle;
    use super::*;

    fn candidate(title: &str) -> ProviderSeries {
        ProviderSeries { title: title.into(), ..Default::default() }
    }

    fn query(title: &str) -> SeriesQuery {
        SeriesQuery { title: title.into(), ..Default::default() }
    }

    #[test]
    fn exact_titles_score_full_regardless_of_punctuation_and_accents() {
        assert_eq!(score(&query("Éclipse : Totale"), &candidate("eclipse totale")), 1.0);
    }

    #[test]
    fn alternate_titles_count_on_both_sides() {
        let mut remote = candidate("Attack on Titan");
        remote.alt_titles.push(AltTitle { title: "Shingeki no Kyojin".into(), language: "ja-ro".into() });
        assert_eq!(score(&query("Shingeki no Kyojin"), &remote), 1.0);

        let mut local = query("L'Attaque des Titans");
        local.alt_titles.push("Attack on Titan".into());
        assert_eq!(score(&local, &candidate("Attack on Titan")), 1.0);
    }

    #[test]
    fn different_works_score_low() {
        assert!(score(&query("Berserk"), &candidate("Vinland Saga")) < 0.5);
    }

    #[test]
    fn a_sequel_is_not_a_perfect_match() {
        assert!(score(&query("Berserk"), &candidate("Berserk: The Prototype")) < CERTAIN_SCORE);
    }

    #[test]
    fn matching_author_lifts_and_wrong_author_lowers() {
        let mut local = query("Monster");
        local.authors = vec!["Urasawa Naoki".into()];
        let mut right = candidate("Monster");
        right.authors = vec!["Naoki Urasawa".into()];
        let mut wrong = candidate("Monster");
        wrong.authors = vec!["Someone Else".into()];

        assert_eq!(score(&local, &right), 1.0, "l'ordre des noms ne compte pas");
        assert!(score(&local, &wrong) < CERTAIN_SCORE, "même titre, autre auteur : incertain");
    }

    #[test]
    fn unknown_signals_do_not_penalize() {
        let mut remote = candidate("Monster");
        remote.authors = vec!["Naoki Urasawa".into()];
        remote.kind = Some("manga".into());
        assert_eq!(score(&query("Monster"), &remote), 1.0);
    }

    #[test]
    fn kind_mismatch_lowers_the_score() {
        let mut local = query("Monster");
        local.kind = Some("comic".into());
        let mut remote = candidate("Monster");
        remote.kind = Some("manga".into());
        assert!(score(&local, &remote) < 1.0);
    }

    #[test]
    fn certainty_requires_a_strong_and_clear_lead() {
        assert!(is_certain(1.0, None));
        assert!(is_certain(1.0, Some(0.6)));
        assert!(!is_certain(1.0, Some(0.95)), "deux fiches presque identiques");
        assert!(!is_certain(0.8, None));
    }
}
