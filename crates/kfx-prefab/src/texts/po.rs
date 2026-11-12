//! gettext `.po` exchange: one file per target language.
//!
//! Each row is one entry: `msgctxt` is `scope:id` (`level:1:42`, `campaign:1013`) so equal
//! source texts stay apart, `msgid` the source language text, `msgstr` the translation, `#:`
//! lines the places that use the id, `#, fuzzy` marks translations that are outdated (the source
//! changed after they were made). Rows without a source text are left out. Import finds rows by
//! `msgctxt` (else by an unambiguous `msgid`), ignores empty `msgstr`, and keeps `fuzzy` entries
//! outdated. The header carries `X-KFX-Language`, the game's own language code.
use super::project::*;
use kfx_formats::strings::Lang;

/// ISO 639 code of a game language (for the `Language:` header).
pub fn iso_code(l: Lang) -> &'static str {
    match l {
        Lang::English => "en",
        Lang::French => "fr",
        Lang::German => "de",
        Lang::Italian => "it",
        Lang::Spanish => "es",
        Lang::Swedish => "sv",
        Lang::Polish => "pl",
        Lang::Dutch => "nl",
        Lang::Hungarian => "hu",
        Lang::Korean => "ko",
        Lang::Danish => "da",
        Lang::Norwegian => "no",
        Lang::Czech => "cs",
        Lang::Arabic => "ar",
        Lang::Russian => "ru",
        Lang::Japanese => "ja",
        Lang::ChineseInt => "zh_CN",
        Lang::ChineseTra => "zh_TW",
        Lang::Portuguese => "pt",
        Lang::Hindi => "hi",
        Lang::Bengali => "bn",
        Lang::Javanese => "jv",
        Lang::Latin => "la",
        Lang::Ukrainian => "uk",
    }
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

/// `msgid "..."` with gettext's multi-line form for texts with line breaks.
fn field(out: &mut String, key: &str, text: &str) {
    if text.contains('\n') {
        out.push_str(&format!("{key} \"\"\n"));
        let mut rest = text;
        while !rest.is_empty() {
            let (line, tail) = match rest.find('\n') {
                Some(i) => (&rest[..=i], &rest[i + 1..]),
                None => (rest, ""),
            };
            out.push_str(&format!("\"{}\"\n", esc(line)));
            rest = tail;
        }
    } else {
        out.push_str(&format!("{key} \"{}\"\n", esc(text)));
    }
}

fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut o = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next()? {
            'n' => o.push('\n'),
            't' => o.push('\t'),
            'r' => o.push('\r'),
            '"' => o.push('"'),
            '\\' => o.push('\\'),
            c => o.push(c),
        }
    }
    Some(o)
}

/// One parsed `.po` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoEntry {
    pub ctxt: Option<String>,
    pub id: String,
    pub text: String,
    pub fuzzy: bool,
}

/// Parse `.po` text (the header entry, with an empty `msgid`, is returned too).
pub fn parse(text: &str) -> Result<Vec<PoEntry>, String> {
    #[derive(PartialEq)]
    enum Cur {
        None,
        Ctxt,
        Id,
        Str,
    }
    let mut out: Vec<PoEntry> = vec![];
    let mut e = PoEntry::default();
    let mut started = false;
    let mut cur = Cur::None;
    let mut fuzzy = false;
    let flush = |e: &mut PoEntry, started: &mut bool, fuzzy: &mut bool, out: &mut Vec<PoEntry>| {
        if *started {
            out.push(std::mem::take(e));
        }
        *started = false;
        *fuzzy = false;
    };
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut e, &mut started, &mut fuzzy, &mut out);
            cur = Cur::None;
            continue;
        }
        if let Some(flags) = line.strip_prefix("#,") {
            if started && cur == Cur::Str {
                flush(&mut e, &mut started, &mut fuzzy, &mut out);
            }
            fuzzy |= flags.split(',').any(|f| f.trim() == "fuzzy");
            continue;
        }
        if line.starts_with('#') {
            if started && cur == Cur::Str {
                flush(&mut e, &mut started, &mut fuzzy, &mut out);
            }
            continue;
        }
        let (key, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) if !k.starts_with('"') => (k, r),
            _ => ("", line),
        };
        let val = || unquote(rest).ok_or_else(|| format!("line {}: bad string", n + 1));
        match key {
            "msgctxt" => {
                if started && cur == Cur::Str {
                    flush(&mut e, &mut started, &mut fuzzy, &mut out);
                }
                started = true;
                e.fuzzy = fuzzy;
                e.ctxt = Some(val()?);
                cur = Cur::Ctxt;
            }
            "msgid" => {
                if started && cur == Cur::Str {
                    flush(&mut e, &mut started, &mut fuzzy, &mut out);
                }
                started = true;
                e.fuzzy = fuzzy;
                e.id = val()?;
                cur = Cur::Id;
            }
            "msgstr" => {
                e.text = val()?;
                cur = Cur::Str;
            }
            "" => {
                let v = unquote(line).ok_or_else(|| format!("line {}: bad continuation", n + 1))?;
                match cur {
                    Cur::Ctxt => e.ctxt.get_or_insert_with(String::new).push_str(&v),
                    Cur::Id => e.id.push_str(&v),
                    Cur::Str => e.text.push_str(&v),
                    Cur::None => return Err(format!("line {}: string outside an entry", n + 1)),
                }
            }
            _ => {} // msgid_plural, msgstr[n]: not used
        }
    }
    flush(&mut e, &mut started, &mut fuzzy, &mut out);
    Ok(out)
}

impl TextProject {
    /// The `.po` of `lang` (translations of the pinned source language).
    pub fn to_po(&self, lang: Lang) -> String {
        let src = self.source();
        let mut out = String::new();
        out.push_str("# Level texts exported by the KeeperFX editor.\nmsgid \"\"\nmsgstr \"\"\n");
        out.push_str("\"Project-Id-Version: KeeperFX level texts\\n\"\n");
        out.push_str(&format!("\"Language: {}\\n\"\n", iso_code(lang)));
        out.push_str(&format!("\"X-KFX-Language: {}\\n\"\n", lang.code()));
        out.push_str(&format!("\"X-KFX-Source-Language: {}\\n\"\n", src.code()));
        out.push_str(
            "\"MIME-Version: 1.0\\n\"\n\"Content-Type: text/plain; \
                charset=UTF-8\\n\"\n\"Content-Transfer-Encoding: 8bit\\n\"\n",
        );
        for (ri, row) in self.rows.iter().enumerate() {
            let source = self.source_text(ri);
            if source.is_empty() {
                continue;
            }
            let c = self.cell(ri, lang);
            out.push('\n');
            let mut places: Vec<String> = row
                .refs
                .iter()
                .map(|r| {
                    if r.line > 0 {
                        format!("{}:{}", r.file, r.line)
                    } else {
                        r.file.clone()
                    }
                })
                .collect();
            places.dedup();
            for r in &row.refs {
                out.push_str(&format!("#. {} ({})\n", r.command, r.file));
            }
            out.push_str(&format!("#: {}\n", places.join(" ")));
            if c.state == CellState::Outdated {
                out.push_str("#, fuzzy\n");
            }
            out.push_str(&format!("msgctxt \"{}:{}\"\n", row.scope.key(), row.id));
            field(&mut out, "msgid", &source);
            let t = if lang == src
                || c.state == CellState::Missing
                || c.state == CellState::Inherited
            {
                String::new()
            } else {
                c.text
            };
            field(
                &mut out,
                "msgstr",
                &if lang == src { self.source_text(ri) } else { t },
            );
        }
        out
    }

    /// Import a `.po` for `lang`.
    pub fn import_po(&mut self, lang: Lang, text: &str) -> Result<ImportReport, String> {
        let entries = parse(text)?;
        let mut rep = ImportReport::default();
        for e in entries {
            if e.id.is_empty() {
                continue; // the header
            }
            // msgctxt = scope:id  (`level:1:42`, `campaign:1013`)
            let found = e.ctxt.as_deref().and_then(|c| {
                let (scope, id) = c.rsplit_once(':')?;
                Some((Scope::parse(scope)?, id.trim().parse::<i64>().ok()?))
            });
            let ri = match found {
                Some((s, id)) => self.row_index(s, id),
                None => {
                    let hits: Vec<usize> = (0..self.rows.len())
                        .filter(|&i| self.source_text(i) == e.id)
                        .collect();
                    if hits.len() == 1 {
                        Some(hits[0])
                    } else {
                        None
                    }
                }
            };
            let Some(ri) = ri else {
                rep.unknown
                    .push(format!("{:?}", e.ctxt.as_deref().unwrap_or(&e.id)));
                continue;
            };
            self.import_cell(&mut rep, ri, lang, &e.text, e.fuzzy);
        }
        Ok(rep)
    }
}
