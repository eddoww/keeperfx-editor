//! Small helpers.

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<u8> = a.bytes().collect();
    let b: Vec<u8> = b.bytes().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let c = if a[i - 1].eq_ignore_ascii_case(&b[j - 1]) {
                0
            } else {
                1
            };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// First sentence of a description (up to 100 characters), for one-line lists.
pub fn first_sentence(doc: &str) -> String {
    let line = doc.trim().lines().next().unwrap_or("").trim();
    let end = line.find(". ").map(|i| i + 1).unwrap_or(line.len());
    let t = &line[..end];
    if t.chars().count() > 100 {
        format!("{}...", t.chars().take(97).collect::<String>())
    } else {
        t.to_string()
    }
}
