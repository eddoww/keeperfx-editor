//! CSV exchange of a project's texts (RFC 4180: comma separated, `"` quoting, CR LF line ends;
//! reading also accepts LF).
//!
//! Columns: `scope` (`level:N` or `campaign`), `id`, `refs` (where the id is used), then one
//! column per language, named by its three letter code (`ENG`, `FRE`...). On import the rows
//! are found by `scope` + `id`; an empty language cell means "nothing supplied" and changes
//! nothing; a cell equal to the current text changes nothing.
use super::project::*;
use kfx_formats::strings::Lang;

pub(crate) fn quote(s: &str) -> String {
    if s.contains(['"', ',', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Parse CSV text into records of fields.
pub fn parse(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut chars = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .peekable();
    let mut in_q = false;
    let mut any = false;
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_q = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                in_q = true;
                any = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                any = true;
            }
            '\r' => {}
            '\n' => {
                if any || !field.is_empty() || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                any = false;
            }
            c => {
                field.push(c);
                any = true;
            }
        }
    }
    if in_q {
        return Err("unterminated quoted field".into());
    }
    if any || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

impl TextProject {
    /// The CSV of every row for `langs` (the source language first when listed).
    pub fn to_csv(&self, langs: &[Lang]) -> String {
        let mut out = String::from("scope,id,refs");
        for l in langs {
            out.push(',');
            out.push_str(l.code());
        }
        out.push_str("\r\n");
        for (ri, row) in self.rows.iter().enumerate() {
            let refs: Vec<String> = row
                .refs
                .iter()
                .map(|r| format!("{}:{} {}", r.file, r.line, r.command))
                .collect();
            out.push_str(&format!(
                "{},{},{}",
                row.scope.key(),
                row.id,
                quote(&refs.join("; "))
            ));
            for &l in langs {
                out.push(',');
                out.push_str(&quote(&self.cell(ri, l).text));
            }
            out.push_str("\r\n");
        }
        out
    }

    /// Import a CSV made by [`TextProject::to_csv`] (possibly edited by a translator).
    pub fn import_csv(&mut self, text: &str) -> Result<ImportReport, String> {
        let records = parse(text)?;
        let Some(header) = records.first() else {
            return Err("empty file".into());
        };
        let col = |name: &str| {
            header
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))
        };
        let (Some(c_scope), Some(c_id)) = (col("scope"), col("id")) else {
            return Err("the header needs `scope` and `id` columns".into());
        };
        let lang_cols: Vec<(usize, Lang)> = header
            .iter()
            .enumerate()
            .filter_map(|(i, h)| Lang::from_code(h).map(|l| (i, l)))
            .collect();
        if lang_cols.is_empty() {
            return Err("no language column (ENG, FRE, ...) in the header".into());
        }
        let mut rep = ImportReport::default();
        for rec in &records[1..] {
            let (Some(scope), Some(id)) = (
                rec.get(c_scope).and_then(|s| Scope::parse(s)),
                rec.get(c_id).and_then(|s| s.trim().parse::<i64>().ok()),
            ) else {
                rep.unknown.push(format!("unreadable row {rec:?}"));
                continue;
            };
            let Some(ri) = self.row_index(scope, id) else {
                rep.unknown.push(format!("{} id {id}", scope.key()));
                continue;
            };
            for &(ci, lang) in &lang_cols {
                let Some(t) = rec.get(ci) else { continue };
                self.import_cell(&mut rep, ri, lang, t, false);
            }
        }
        Ok(rep)
    }
}
