//! Small line diff for the raw views (override files are short).

/// One diff line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Same(String),
    Removed(String),
    Added(String),
}

/// Line diff of `a` -> `b`: common prefix / suffix trimmed, LCS on the rest (falls back to
/// "all removed, all added" when the middle is very large).
pub fn line_diff(a: &str, b: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    let am = &a[pre..a.len() - suf];
    let bm = &b[pre..b.len() - suf];
    let mut out: Vec<DiffLine> = a[..pre]
        .iter()
        .map(|s| DiffLine::Same(s.to_string()))
        .collect();
    if am.len() * bm.len() > 4_000_000 {
        out.extend(am.iter().map(|s| DiffLine::Removed(s.to_string())));
        out.extend(bm.iter().map(|s| DiffLine::Added(s.to_string())));
    } else {
        let (n, m) = (am.len(), bm.len());
        let mut l = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                l[i * (m + 1) + j] = if am[i] == bm[j] {
                    l[(i + 1) * (m + 1) + j + 1] + 1
                } else {
                    l[(i + 1) * (m + 1) + j].max(l[i * (m + 1) + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(DiffLine::Same(am[i].to_string()));
                i += 1;
                j += 1;
            } else if i < n && (j == m || l[(i + 1) * (m + 1) + j] >= l[i * (m + 1) + j + 1]) {
                out.push(DiffLine::Removed(am[i].to_string()));
                i += 1;
            } else {
                out.push(DiffLine::Added(bm[j].to_string()));
                j += 1;
            }
        }
    }
    out.extend(
        a[a.len() - suf..]
            .iter()
            .map(|s| DiffLine::Same(s.to_string())),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple() {
        let d = line_diff("a\nb\nc\n", "a\nx\nc\nd\n");
        assert_eq!(
            d,
            vec![
                DiffLine::Same("a".into()),
                DiffLine::Removed("b".into()),
                DiffLine::Added("x".into()),
                DiffLine::Same("c".into()),
                DiffLine::Added("d".into())
            ]
        );
    }
}
