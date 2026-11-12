//! Script fragments of a prefab and their placeholders.
//!
//! A fragment is DK script and / or Lua text that refers to the prefab's action points and hero
//! gates by key instead of by number, and to its player slots instead of players:
//!
//! * `{{ap.KEY}}`: the number the action point `KEY` got where the prefab was applied;
//! * `{{gate.KEY}}`: the number of the hero gate `KEY` (a DK script writes gates as negative
//!   locations: `-{{gate.KEY}}`);
//! * `{{player.A}}`: the player slot `A`..`H` as the target map names it (`PLAYER0`..`PLAYER3`,
//!   `PLAYER_GOOD`, `PLAYER_NEUTRAL`...).
//!
//! [`rewrite`] fills them in; [`placeholderise`] does the reverse for a captured fragment, using
//! `kfx_script`'s own reference scan so only real action point / hero gate arguments are touched.
use std::collections::BTreeMap;

/// Which language a fragment is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    Dk,
    Lua,
}

/// Values for the placeholders.
#[derive(Clone, Debug, Default)]
pub struct Substitutions {
    pub ap: BTreeMap<String, i64>,
    pub gate: BTreeMap<String, i64>,
    /// Slot letter -> text (`PLAYER0`).
    pub player: BTreeMap<String, String>,
}

/// A placeholder found in a fragment.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placeholder {
    /// `ap`, `gate` or `player`.
    pub kind: String,
    pub key: String,
}

/// Script text of player `n` (`PLAYER0`..`PLAYER3`, `PLAYER_GOOD`, `PLAYER_NEUTRAL`; the extra
/// colours have no script name before `PLAYER6`..).
pub fn player_text(n: u8) -> String {
    match n {
        4 => "PLAYER_GOOD".into(),
        5 => "PLAYER_NEUTRAL".into(),
        n => format!("PLAYER{n}"),
    }
}

fn scan(s: &str, mut f: impl FnMut(usize, usize, &str, &str)) {
    let mut from = 0;
    while let Some(p) = s[from..].find("{{") {
        let start = from + p;
        let Some(e) = s[start..].find("}}") else {
            break;
        };
        let end = start + e + 2;
        let inner = &s[start + 2..end - 2];
        if let Some((kind, key)) = inner.split_once('.') {
            if matches!(kind, "ap" | "gate" | "player")
                && !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                f(start, end, kind, key);
            }
        }
        from = end;
    }
}

/// The placeholders of a fragment, sorted, without duplicates.
pub fn placeholders(fragment: &str) -> Vec<Placeholder> {
    let mut v = vec![];
    scan(fragment, |_, _, k, key| {
        v.push(Placeholder {
            kind: k.into(),
            key: key.into(),
        })
    });
    v.sort();
    v.dedup();
    v
}

/// Replace the placeholders; returns the text and the placeholders that had no value (left as
/// they are).
pub fn rewrite(fragment: &str, subs: &Substitutions) -> (String, Vec<Placeholder>) {
    let mut out = String::with_capacity(fragment.len());
    let mut missing = vec![];
    let mut last = 0;
    scan(fragment, |s, e, kind, key| {
        let v = match kind {
            "ap" => subs.ap.get(key).map(|n| n.to_string()),
            "gate" => subs.gate.get(key).map(|n| n.to_string()),
            _ => subs.player.get(key).cloned(),
        };
        out.push_str(&fragment[last..s]);
        match v {
            Some(v) => out.push_str(&v),
            None => {
                out.push_str(&fragment[s..e]);
                missing.push(Placeholder {
                    kind: kind.into(),
                    key: key.into(),
                });
            }
        }
        last = e;
    });
    out.push_str(&fragment[last..]);
    missing.sort();
    missing.dedup();
    (out, missing)
}

/// Turn the action point / hero gate numbers and `PLAYERn` words of a captured fragment into
/// placeholders. `aps` / `gates` map the numbers the map had to the prefab keys, `players` maps a
/// player number to its slot letter. Numbers without an entry stay as they are.
pub fn placeholderise(
    fragment: &str,
    kind: ScriptKind,
    aps: &[(i64, String)],
    gates: &[(i64, String)],
    players: &[(u8, char)],
) -> String {
    use kfx_script::RefKind;
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let refs = match kind {
        ScriptKind::Dk => kfx_script::references(&kfx_script::Document::parse(fragment.as_bytes())),
        ScriptKind::Lua => {
            let doc = kfx_script::lua::LuaDoc::parse(fragment);
            kfx_script::lua::references(&doc, &kfx_script::lua::ApiIndex::stock())
        }
    };
    for r in refs {
        let (list, tag) = match r.kind {
            RefKind::ActionPoint => (aps, "ap"),
            RefKind::HeroGate => (gates, "gate"),
            _ => continue,
        };
        if let Some((_, key)) = list.iter().find(|(n, _)| *n == r.value as i64) {
            let text = &fragment[r.span.start..r.span.end.min(fragment.len())];
            let neg = text.trim_start().starts_with('-');
            edits.push((
                r.span.start,
                r.span.end,
                format!("{}{{{{{tag}.{key}}}}}", if neg { "-" } else { "" }),
            ));
        }
    }
    // PLAYERn words
    let b = fragment.as_bytes();
    let mut i = 0;
    while i + 7 <= b.len() {
        if &b[i..i + 6] == b"PLAYER" && b[i + 6].is_ascii_digit() {
            let before = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            let after = i + 7 == b.len() || !(b[i + 7].is_ascii_alphanumeric() || b[i + 7] == b'_');
            if before && after {
                let n = b[i + 6] - b'0';
                if let Some((_, slot)) = players.iter().find(|(p, _)| *p == n) {
                    edits.push((i, i + 7, format!("{{{{player.{slot}}}}}")));
                }
                i += 7;
                continue;
            }
        }
        i += 1;
    }
    edits.sort();
    edits.dedup_by(|b, a| b.0 < a.1);
    let mut out = fragment.to_string();
    for (s, e, t) in edits.into_iter().rev() {
        if e <= out.len() && s <= e {
            out.replace_range(s..e, &t);
        }
    }
    out
}

/// Append `fragment` to `existing` script text under a comment line naming it. A blank line
/// separates the two; the line ending follows the existing text.
pub fn append_fragment(existing: &str, fragment: &str, label: &str, kind: ScriptKind) -> String {
    let eol = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let comment = match kind {
        ScriptKind::Dk => format!("REM --- prefab: {label} ---"),
        ScriptKind::Lua => format!("-- prefab: {label}"),
    };
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(eol);
    }
    if !out.is_empty() {
        out.push_str(eol);
    }
    out.push_str(&comment);
    out.push_str(eol);
    for l in fragment.trim_end().lines() {
        out.push_str(l);
        out.push_str(eol);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_and_scan() {
        let f = "IF_ACTION_POINT({{ap.door}}, {{player.A}})\n\
            ADD_PARTY_TO_LEVEL(PLAYER_GOOD, X, -{{gate.g}}, 1, 1)\n{{ap.zzz}}";
        let mut s = Substitutions::default();
        s.ap.insert("door".into(), 7);
        s.gate.insert("g".into(), 3);
        s.player.insert("A".into(), "PLAYER1".into());
        let (t, missing) = rewrite(f, &s);
        assert_eq!(
            t,
            "IF_ACTION_POINT(7, PLAYER1)\nADD_PARTY_TO_LEVEL(PLAYER_GOOD, X, -3, 1, 1)\n{{ap.zzz}}"
        );
        assert_eq!(
            missing,
            vec![Placeholder {
                kind: "ap".into(),
                key: "zzz".into()
            }]
        );
        assert_eq!(placeholders(f).len(), 4);
    }

    #[test]
    fn placeholderise_dk() {
        let src = "IF_ACTION_POINT(5, PLAYER0)\n\
            \tADD_PARTY_TO_LEVEL(PLAYER_GOOD, AMBUSH, -2, 1, 1)\nENDIF\n";
        let out = placeholderise(
            src,
            ScriptKind::Dk,
            &[(5, "ap5".into())],
            &[(2, "gate2".into())],
            &[(0, 'A')],
        );
        assert_eq!(
            out,
            "IF_ACTION_POINT({{ap.ap5}}, {{player.A}})\n\
                \tADD_PARTY_TO_LEVEL(PLAYER_GOOD, AMBUSH, -{{gate.gate2}}, 1, 1)\nENDIF\n"
        );
    }
}
