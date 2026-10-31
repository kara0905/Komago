use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;

/// Forme de comparaison d'un titre : sans accents, sans casse ni ponctuation.
pub fn normalize_title(title: &str) -> String {
    let folded: String = title
        .nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Clé de tri : titre normalisé, sans article initial.
pub fn sort_title(title: &str) -> String {
    let normalized = normalize_title(title);
    for article in ["the ", "a ", "an ", "le ", "la ", "les ", "l "] {
        if let Some(rest) = normalized.strip_prefix(article) {
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    normalized
}

/// Tri naturel : `page2` avant `page10`, sans tenir compte de la casse.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let first = take_digits(&mut left);
                let second = take_digits(&mut right);
                let order = first.len().cmp(&second.len()).then_with(|| first.cmp(&second));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left.next();
                right.next();
            }
        }
    }
}

fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(c) = chars.peek().filter(|c| c.is_ascii_digit()) {
        digits.push(*c);
        chars.next();
    }
    let trimmed = digits.trim_start_matches('0');
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_by(|a, b| natural_cmp(a, b));
        names
    }

    #[test]
    fn natural_order_handles_numbers_case_and_padding() {
        assert_eq!(sorted(vec!["p10.jpg", "p2.jpg", "p1.jpg"]), ["p1.jpg", "p2.jpg", "p10.jpg"]);
        assert_eq!(sorted(vec!["B.jpg", "a.jpg"]), ["a.jpg", "B.jpg"]);
        assert_eq!(sorted(vec!["010.png", "9.png", "001.png"]), ["001.png", "9.png", "010.png"]);
        assert_eq!(sorted(vec!["ch2/1.jpg", "ch10/1.jpg", "ch1/2.jpg"]), ["ch1/2.jpg", "ch2/1.jpg", "ch10/1.jpg"]);
    }

    #[test]
    fn natural_order_is_total_for_equal_numbers() {
        assert_ne!(natural_cmp("01.jpg", "1.jpg"), Ordering::Equal);
    }

    #[test]
    fn titles_compare_without_accents_or_punctuation() {
        assert_eq!(normalize_title("Éclipse : Totale!"), "eclipse totale");
        assert_eq!(normalize_title("ONE-PIECE"), "one piece");
        assert_eq!(normalize_title("進撃の巨人"), "進撃の巨人");
    }

    #[test]
    fn sort_title_ignores_leading_articles() {
        assert_eq!(sort_title("The Walking Dead"), "walking dead");
        assert_eq!(sort_title("L'Attaque"), "attaque");
        assert_eq!(sort_title("Les"), "les");
    }
}
