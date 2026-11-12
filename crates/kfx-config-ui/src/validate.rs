//! Field validation against the schema, following the game's value parsers: `value_default`
//! (number clamped to `[min, max]`, else a name of the field's table, src/config.c:437),
//! `value_flagsfield` (number, `none`, or flag names, src/config.c:522), name lookups of other
//! configs.
use crate::schema::{ArgSpec, KeySpec, Schema};
use kfx_formats::cfg::split_tokens;
use kfx_map::gamedata::parse_number;

/// A problem with a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Hard errors make the game ignore or zero the value; warnings mean it is clamped / odd.
    pub error: bool,
    pub msg: String,
}

fn err(m: String) -> Issue {
    Issue {
        error: true,
        msg: m,
    }
}
fn warn(m: String) -> Issue {
    Issue {
        error: false,
        msg: m,
    }
}

fn enum_has(schema: &Schema, table: &str, tok: &str) -> Option<bool> {
    let t = schema.enum_table(table).filter(|t| !t.is_empty())?;
    Some(t.iter().any(|e| e.name.eq_ignore_ascii_case(tok)))
}

fn ref_has(names: &mut dyn FnMut(&str) -> Vec<(i64, String)>, r: &str, tok: &str) -> Option<bool> {
    let list = names(r);
    if list.is_empty() {
        return None;
    }
    Some(
        tok.eq_ignore_ascii_case("NULL")
            || tok.eq_ignore_ascii_case("none")
            || list.iter().any(|(_, n)| n.eq_ignore_ascii_case(tok)),
    )
}

fn check_token(
    arg: &ArgSpec,
    tok: &str,
    schema: &Schema,
    names: &mut dyn FnMut(&str) -> Vec<(i64, String)>,
    out: &mut Vec<Issue>,
) {
    let num = parse_number(tok);
    let label = arg.label.clone().unwrap_or_default();
    let what = if label.is_empty() {
        String::new()
    } else {
        format!("{label}: ")
    };
    match arg.ty.as_str() {
        "int" | "coord" => match num {
            Some(n) => {
                if let Some(min) = arg.min_i64() {
                    if n < min {
                        out.push(warn(format!(
                            "{what}{n} is below the minimum {min} (the game clamps it)"
                        )));
                    }
                }
                if let Some(max) = arg.max_i64() {
                    if n > max {
                        out.push(warn(format!(
                            "{what}{n} is above the maximum {max} (the game clamps it)"
                        )));
                    }
                }
            }
            None => {
                let ok = arg
                    .enum_table
                    .as_deref()
                    .and_then(|t| enum_has(schema, t, tok))
                    .unwrap_or(false);
                if !ok {
                    out.push(err(format!(
                        "{what}'{tok}' is not a number (the game reads 0)"
                    )));
                }
            }
        },
        "enum" | "name" | "ref" => {
            if num.is_none() {
                let known = match arg.ref_kind() {
                    Some(r) => ref_has(names, &r, tok).map(|ok| (ok, r)),
                    None => arg
                        .enum_table
                        .as_deref()
                        .and_then(|t| enum_has(schema, t, tok))
                        .map(|ok| (ok, "value".to_string())),
                };
                if let Some((false, what_kind)) = known {
                    out.push(err(format!("{what}unknown {what_kind} '{tok}'")));
                }
            }
        }
        "bool" => {
            if !matches!(
                tok.to_ascii_lowercase().as_str(),
                "0" | "1" | "true" | "false"
            ) {
                out.push(warn(format!("{what}'{tok}' is not 0/1")));
            }
        }
        "flags" | "longflags" => {
            if num.is_none() && !tok.eq_ignore_ascii_case("none") {
                if let Some(false) = arg
                    .enum_table
                    .as_deref()
                    .and_then(|t| enum_has(schema, t, tok))
                {
                    out.push(err(format!("{what}unknown flag '{tok}'")));
                }
            }
        }
        _ => {}
    }
}

/// Problems of `value` for `key`. `names(kind)` lists the names of a referenced config.
pub fn validate(
    key: &KeySpec,
    value: &str,
    schema: &Schema,
    names: &mut dyn FnMut(&str) -> Vec<(i64, String)>,
) -> Vec<Issue> {
    let mut out = vec![];
    let toks = split_tokens(value);
    if key.args.is_empty() {
        return out;
    }
    let first = &key.args[0];
    if key.whole_line && key.args.len() == 1 && matches!(first.ty.as_str(), "flags" | "longflags") {
        for (i, t) in toks.iter().enumerate() {
            if i == 1 && (t == "0" || t == "1") {
                continue;
            }
            check_token(first, t, schema, names, &mut out);
        }
        return out;
    }
    if key.whole_line {
        return out;
    }
    let full = key.confidence.as_deref() == Some("full");
    let variadic = key.args.last().is_some_and(|a| a.variadic);
    if !key.repeat && !variadic && toks.len() > key.args.len() && full {
        out.push(warn(format!(
            "{} values, the game reads the first {}",
            toks.len(),
            key.args.len()
        )));
    }
    if full && toks.len() < key.args.len() && !toks.is_empty() {
        out.push(warn(format!(
            "{} values, expected {}",
            toks.len(),
            key.args.len()
        )));
    }
    for (i, t) in toks.iter().enumerate() {
        let arg = key.args.get(i).or(if key.repeat || variadic {
            key.args.last()
        } else {
            None
        });
        if let Some(a) = arg {
            check_token(a, t, schema, names, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_names() {
        let k = KeySpec {
            name: "X".into(),
            args: vec![ArgSpec {
                ty: "int".into(),
                min: Some(0.0),
                max: Some(10.0),
                ..Default::default()
            }],
            confidence: Some("full".into()),
            ..Default::default()
        };
        let s = Schema::default();
        let mut names = |_: &str| vec![];
        assert!(validate(&k, "5", &s, &mut names).is_empty());
        assert_eq!(validate(&k, "11", &s, &mut names).len(), 1);
        assert!(validate(&k, "abc", &s, &mut names)[0].error);
        let r = KeySpec {
            name: "C".into(),
            args: vec![ArgSpec {
                ty: "ref".into(),
                reference: Some("creature".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut cr = |_: &str| vec![(1, "DRAGON".to_string())];
        assert!(validate(&r, "dragon", &s, &mut cr).is_empty());
        assert!(validate(&r, "DRAGONN", &s, &mut cr)[0].error);
        let e = KeySpec {
            name: "P".into(),
            args: vec![ArgSpec {
                ty: "enum".into(),
                enum_table: Some("creature_desc".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(validate(&e, "DRAGON", &s, &mut cr).is_empty());
    }
}
