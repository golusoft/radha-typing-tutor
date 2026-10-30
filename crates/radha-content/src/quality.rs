//! Content quality checks shared by the importer and the bundle tests:
//! placeholder markers, repeated lines and near-duplicate passages.

use std::collections::HashSet;

/// Markers that mean a passage is not finished text. Whole-word and
/// case-insensitive for `TODO` / `TBD`; substrings otherwise.
#[must_use]
pub fn placeholder_markers(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut found = Vec::new();
    if lower.contains("lorem") {
        found.push("lorem");
    }
    // "placeholder" as a marker (`PLACEHOLDER`, `[placeholder]`,
    // "placeholder text"); the ordinary English word in a sentence
    // ("fictional placeholders for practice") is not one.
    let marker = text.contains("PLACEHOLDER")
        || [
            "[placeholder",
            "<placeholder",
            "{placeholder",
            "(placeholder",
        ]
        .iter()
        .any(|m| lower.contains(m))
        || [
            "placeholder text",
            "placeholder passage",
            "placeholder content",
        ]
        .iter()
        .any(|m| lower.contains(m));
    if marker {
        found.push("placeholder");
    }
    // An elision "..." (exactly three dots). A longer run is a key drill
    // for the full-stop key ("...." in a bottom-row lesson), not a gap.
    if text.split(|c: char| c != '.').any(|run| run.len() == 3) {
        found.push("...");
    }
    let words: HashSet<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    for marker in ["todo", "tbd"] {
        if words.contains(marker) {
            found.push(marker);
        }
    }
    if text.trim().is_empty() {
        found.push("empty");
    }
    found
}

/// The normalised words of `text`: lower case, letters and digits only.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// FNV-1a, stable across runs and platforms.
fn fnv(parts: &[String]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(b' ')) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// Sorted, de-duplicated hashes of every `k`-word window of `text`.
#[must_use]
pub fn shingles(text: &str, k: usize) -> Vec<u64> {
    let w = words(text);
    let mut out: Vec<u64> = if w.len() < k {
        vec![fnv(&w)]
    } else {
        w.windows(k).map(fnv).collect()
    };
    out.sort_unstable();
    out.dedup();
    out
}

/// Jaccard similarity of two sorted shingle sets.
#[must_use]
pub fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (mut i, mut j, mut both) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                both += 1;
                i += 1;
                j += 1;
            }
        }
    }
    both as f64 / (a.len() + b.len() - both) as f64
}

/// The most similar pair of `texts` by 5-word-shingle Jaccard:
/// `(similarity, i, j)`; `None` for fewer than two texts.
#[must_use]
pub fn most_similar_pair(texts: &[&str]) -> Option<(f64, usize, usize)> {
    let sets: Vec<Vec<u64>> = texts.iter().map(|t| shingles(t, 5)).collect();
    let mut best: Option<(f64, usize, usize)> = None;
    for i in 0..sets.len() {
        for j in i + 1..sets.len() {
            let s = jaccard(&sets[i], &sets[j]);
            if best.is_none_or(|(b, _, _)| s > b) {
                best = Some((s, i, j));
            }
        }
    }
    best
}

/// Lines (trimmed, non-empty) that occur more than once across `texts`,
/// with the indices of the texts holding them.
#[must_use]
pub fn repeated_lines(texts: &[&str]) -> Vec<(String, Vec<usize>)> {
    let mut seen: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, text) in texts.iter().enumerate() {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            seen.entry(line).or_default().push(i);
        }
    }
    let mut out: Vec<(String, Vec<usize>)> = seen
        .into_iter()
        .filter(|(_, at)| at.len() > 1)
        .map(|(l, at)| (l.to_owned(), at))
        .collect();
    out.sort();
    out
}

/// Sentences (split at `.`, `?`, `!` followed by a space or line end) of at
/// least `min_words` words that occur in more than one of `texts`.
#[must_use]
pub fn shared_sentences(texts: &[&str], min_words: usize) -> Vec<String> {
    let mut owner: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut shared: HashSet<String> = HashSet::new();
    for (i, text) in texts.iter().enumerate() {
        let mut local: HashSet<String> = HashSet::new();
        for s in sentences(text) {
            if s.split_whitespace().count() < min_words || !local.insert(s.clone()) {
                continue;
            }
            match owner.get(&s) {
                Some(&o) if o != i => {
                    shared.insert(s);
                }
                Some(_) => {}
                None => {
                    owner.insert(s, i);
                }
            }
        }
    }
    let mut out: Vec<String> = shared.into_iter().collect();
    out.sort();
    out
}

/// The sentences of `text`.
#[must_use]
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let end =
            matches!(c, '.' | '?' | '!') && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if end || c == '\n' {
            let s = cur.trim().to_owned();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        }
    }
    let s = cur.trim().to_owned();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_found() {
        assert!(placeholder_markers("A real sentence.").is_empty());
        assert_eq!(placeholder_markers("Lorem ipsum"), ["lorem"]);
        assert_eq!(placeholder_markers("wait..."), ["..."]);
        assert!(
            placeholder_markers(",,,, .... ////").is_empty(),
            "a full-stop key drill is not an elision"
        );
        assert_eq!(placeholder_markers("TODO: write"), ["todo"]);
        assert_eq!(placeholder_markers("[Placeholder] body"), ["placeholder"]);
        assert_eq!(
            placeholder_markers("placeholder text here"),
            ["placeholder"]
        );
        assert!(placeholder_markers("used neutral placeholders instead of real data").is_empty());
        assert!(placeholder_markers("Todos los dias").is_empty());
        assert_eq!(placeholder_markers("  "), ["empty"]);
    }

    #[test]
    fn similarity_and_repeats() {
        let a = "the clerk entered the date and the reference number in the register today";
        let b = "a completely different passage about railway parcels and their dispatch times";
        assert!(jaccard(&shingles(a, 5), &shingles(a, 5)) > 0.99);
        assert!(jaccard(&shingles(a, 5), &shingles(b, 5)) < 0.01);
        let (s, i, j) = most_similar_pair(&[a, b, a]).unwrap();
        assert!(s > 0.99 && (i, j) == (0, 2));
        assert_eq!(
            repeated_lines(&["x\ny", "y\nz"]),
            [("y".to_owned(), vec![0, 1])]
        );
        let shared = shared_sentences(
            &[
                "One two three four. Five six.",
                "One two three four. Seven.",
            ],
            3,
        );
        assert_eq!(shared, ["One two three four."]);
    }
}
