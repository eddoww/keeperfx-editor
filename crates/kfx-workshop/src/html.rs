//! Tolerant parsers for the Workshop's HTML pages.
//!
//! The site has no stable public API for browsing and for the file list, so the pages are the
//! contract. The parser tokenises the page (skipping `<script>`/`<style>`) and then looks for
//! landmarks that have been stable across the 767 item pages surveyed:
//!
//! * browse page: every `div.workshop-grid-item` is a card (`a[href=/workshop/item/<id>/<slug>]`,
//!   `img` thumbnail, a category label `span`, `.user-username`, `data-workshop-rating-*`
//!   attributes); `ul.pagination` links carry `page=N`.
//! * item page: `h2.workshop-item-title`; `#workshop-images`; the info table rows (`tr` with two
//!   `td` cells: label, value) including `data-workshop-rating-score` spans; the Downloads table
//!   (rows whose first cell links to `/workshop/download/<item>/<file>/<name>`, struck through
//!   with "(Broken)" when broken); `h5` "Description" / "Installation Instructions" followed by a
//!   `div.markdown-body`.
//!
//! Anything missing yields an empty field rather than an error; only a page with neither a title
//! nor a file is rejected.

use crate::error::{Error, Result};
use crate::model::*;
use crate::util::parse_size;

#[derive(Clone, Debug)]
pub(crate) enum Tok {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    Close(String),
    Text(String),
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(end) = rest.as_bytes()[..rest.len().min(12)]
            .iter()
            .position(|&b| b == b';')
        else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let rep: Option<String> = match ent {
            "amp" => Some("&".into()),
            "lt" => Some("<".into()),
            "gt" => Some(">".into()),
            "quot" => Some("\"".into()),
            "apos" | "#039" | "#39" => Some("'".into()),
            "nbsp" => Some(" ".into()),
            "bull" => Some("\u{2022}".into()),
            "hellip" => Some("\u{2026}".into()),
            "ndash" => Some("\u{2013}".into()),
            "mdash" => Some("\u{2014}".into()),
            _ => {
                if let Some(n) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    u32::from_str_radix(n, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(String::from)
                } else if let Some(n) = ent.strip_prefix('#') {
                    n.parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .map(String::from)
                } else {
                    None
                }
            }
        };
        match rep {
            Some(r) => {
                out.push_str(&r);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub(crate) fn tokenize(html: &str) -> Vec<Tok> {
    let b = html.as_bytes();
    let mut toks = vec![];
    let mut i = 0;
    let mut text_start = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        // comment
        if html[i..].starts_with("<!--") {
            push_text(&mut toks, &html[text_start..i]);
            i = html[i..].find("-->").map_or(b.len(), |e| i + e + 3);
            text_start = i;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if !(next.is_ascii_alphabetic() || next == b'/' || next == b'!') {
            i += 1;
            continue;
        }
        push_text(&mut toks, &html[text_start..i]);
        // find the end of the tag, respecting quotes
        let mut j = i + 1;
        let mut q = 0u8;
        while j < b.len() {
            let c = b[j];
            if q != 0 {
                if c == q {
                    q = 0;
                }
            } else if c == b'"' || c == b'\'' {
                q = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        let raw = &html[i + 1..j.min(b.len())];
        i = (j + 1).min(b.len());
        text_start = i;
        if raw.starts_with('!') || raw.starts_with('?') {
            continue;
        }
        if let Some(name) = raw.strip_prefix('/') {
            toks.push(Tok::Close(name.trim().to_ascii_lowercase()));
            continue;
        }
        let (name, attrs, sc) = parse_tag(raw);
        let skip_body = name == "script" || name == "style";
        toks.push(Tok::Open {
            name: name.clone(),
            attrs,
            self_closing: sc,
        });
        if skip_body && !sc {
            let close = format!("</{name}");
            let end = find_ci(&html[i..], &close).map_or(b.len(), |e| i + e);
            i = html[end.min(b.len())..]
                .find('>')
                .map_or(b.len(), |e| end + e + 1);
            text_start = i;
            toks.push(Tok::Close(name));
        }
    }
    push_text(&mut toks, &html[text_start.min(html.len())..]);
    toks
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&k| h[k..k + n.len()].eq_ignore_ascii_case(n))
}

fn push_text(toks: &mut Vec<Tok>, t: &str) {
    if !t.trim().is_empty() {
        toks.push(Tok::Text(decode_entities(t)));
    }
}

fn parse_tag(raw: &str) -> (String, Vec<(String, String)>, bool) {
    let raw = raw.trim();
    let sc = raw.ends_with('/');
    let raw = raw.trim_end_matches('/');
    let name_end = raw.find(|c: char| c.is_whitespace()).unwrap_or(raw.len());
    let name = raw[..name_end].to_ascii_lowercase();
    let mut attrs = vec![];
    let mut rest = raw[name_end..].trim_start();
    while !rest.is_empty() {
        let key_end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let key = rest[..key_end].to_ascii_lowercase();
        rest = rest[key_end..].trim_start();
        let mut val = String::new();
        if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let body = &r[1..];
                let end = body.find(q).unwrap_or(body.len());
                val = decode_entities(&body[..end]);
                rest = body[(end + 1).min(body.len())..].trim_start();
            } else {
                let end = r.find(char::is_whitespace).unwrap_or(r.len());
                val = decode_entities(&r[..end]);
                rest = r[end..].trim_start();
            }
        }
        if !key.is_empty() {
            attrs.push((key, val));
        }
    }
    (name, attrs, sc)
}

pub(crate) fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

pub(crate) fn has_class(attrs: &[(String, String)], class: &str) -> bool {
    attr(attrs, "class").is_some_and(|c| c.split_whitespace().any(|x| x == class))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text of the tokens `toks[from..]` up to (excluding) the close of the element opened at
/// `toks[from - 1]`... callers pass the range explicitly.
fn text_of(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        if let Tok::Text(x) = t {
            s.push_str(x);
            s.push(' ');
        }
    }
    collapse(&s)
}

/// Index one past the close tag matching the open tag at `i` (same name, depth counted).
fn element_end(toks: &[Tok], i: usize) -> usize {
    let Tok::Open {
        name, self_closing, ..
    } = &toks[i]
    else {
        return i + 1;
    };
    if *self_closing
        || matches!(
            name.as_str(),
            "img" | "br" | "hr" | "input" | "meta" | "link"
        )
    {
        return i + 1;
    }
    let mut depth = 0i32;
    for (k, t) in toks.iter().enumerate().skip(i) {
        match t {
            Tok::Open {
                name: n,
                self_closing: false,
                ..
            } if n == name => depth += 1,
            Tok::Close(n) if n == name => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

fn parse_item_href(href: &str) -> Option<(u32, String)> {
    let p = href.find("/workshop/item/")?;
    let rest = &href[p + "/workshop/item/".len()..];
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut it = rest.splitn(2, '/');
    let id: u32 = it.next()?.parse().ok()?;
    Some((id, it.next().unwrap_or("").trim_matches('/').to_string()))
}

fn to_f32(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// Parse a browse page.
pub fn parse_browse(html: &str) -> Result<Page> {
    let toks = tokenize(html);
    let mut items = vec![];
    let mut i = 0;
    while i < toks.len() {
        let is_card = matches!(&toks[i], Tok::Open { name, attrs, .. } if name == "div" && has_class(attrs, "workshop-grid-item"));
        if !is_card {
            i += 1;
            continue;
        }
        let end = element_end(&toks, i);
        if let Some(it) = parse_card(&toks[i..end]) {
            items.push(it);
        }
        i = end;
    }
    // pagination: the largest page=N seen in a link of ul.pagination
    let mut total = 1u32;
    let mut cur = 1u32;
    for (k, t) in toks.iter().enumerate() {
        if let Tok::Open { name, attrs, .. } = t {
            if name == "ul" && has_class(attrs, "pagination") {
                let end = element_end(&toks, k);
                let mut active = false;
                for t2 in &toks[k..end] {
                    if let Tok::Open { name, attrs, .. } = t2 {
                        if name == "li" {
                            active = has_class(attrs, "active");
                        }
                        if name == "a" {
                            if let Some(n) = attr(attrs, "href")
                                .and_then(|h| {
                                    h.split(['?', '&']).find_map(|kv| kv.strip_prefix("page="))
                                })
                                .and_then(|v| v.parse::<u32>().ok())
                            {
                                total = total.max(n);
                                if active {
                                    cur = n;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(Page {
        items,
        page: cur,
        total_pages: total,
    })
}

fn parse_card(toks: &[Tok]) -> Option<ItemSummary> {
    let mut it = ItemSummary::default();
    let mut found = false;
    let mut cat_done = false;
    for (k, t) in toks.iter().enumerate() {
        match t {
            Tok::Open { name, attrs, .. } if name == "a" => {
                if let Some((id, slug)) = attr(attrs, "href").and_then(parse_item_href) {
                    if !found {
                        it.id = id;
                        it.slug = slug;
                        found = true;
                    }
                    if let Some(title) = attr(attrs, "title").filter(|t| !t.trim().is_empty()) {
                        if it.title.is_empty() {
                            it.title = title.trim().to_string();
                        }
                    }
                    if it.title.is_empty() {
                        let e = element_end(toks, k);
                        it.title = text_of(&toks[k..e]);
                    }
                }
            }
            Tok::Open { name, attrs, .. } if name == "img" => {
                if let Some(src) = attr(attrs, "src") {
                    if it.thumbnail.is_none() && src.contains("/workshop/image/") {
                        it.thumbnail = Some(src.to_string());
                    }
                }
            }
            Tok::Open { name, attrs, .. } if name == "span" => {
                if let Some(ty) = attr(attrs, "data-workshop-rating-type") {
                    let v = attr(attrs, "data-workshop-rating-score").and_then(to_f32);
                    match ty {
                        "quality" => it.rating = v,
                        "difficulty" => it.difficulty = v,
                        _ => {}
                    }
                } else if has_class(attrs, "user-username") {
                    let e = element_end(toks, k);
                    it.submitter = text_of(&toks[k..e]);
                } else if !cat_done {
                    let e = element_end(toks, k);
                    let txt = text_of(&toks[k..e]);
                    if let Some(c) = Category::from_label(&txt) {
                        it.category = Some(c);
                        cat_done = true;
                    }
                }
            }
            _ => {}
        }
    }
    found.then_some(it)
}

/// Plain-text rendering of a markdown-rendered description block.
fn render_block(toks: &[Tok]) -> String {
    let mut out = String::new();
    let mut href_stack: Vec<Option<String>> = vec![];
    for t in toks {
        match t {
            Tok::Text(s) => {
                let s = collapse(s);
                if !s.is_empty() {
                    if !out.is_empty() && !out.ends_with(['\n', ' ']) {
                        out.push(' ');
                    }
                    out.push_str(&s);
                }
            }
            Tok::Open { name, attrs, .. } => match name.as_str() {
                "br" => out.push('\n'),
                "li" => out.push_str("\n- "),
                "p" | "div" | "ul" | "ol" | "pre" | "blockquote" | "h1" | "h2" | "h3" | "h4"
                | "h5" | "h6" => {
                    if !out.is_empty() && !out.ends_with("\n\n") {
                        out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
                    }
                }
                "a" => href_stack.push(attr(attrs, "href").map(str::to_string)),
                _ => {}
            },
            Tok::Close(name) => {
                if name == "a" {
                    if let Some(Some(h)) = href_stack.pop() {
                        if h.starts_with("http") && !out.ends_with(&h) {
                            out.push_str(&format!(" ({h})"));
                        }
                    }
                }
            }
        }
    }
    out.trim().to_string()
}

/// Parse an item page.
pub fn parse_item(html: &str) -> Result<ItemDetail> {
    let toks = tokenize(html);
    let mut d = ItemDetail {
        source: "html".into(),
        ..Default::default()
    };
    let mut i = 0;
    let mut in_images = false;
    let mut images_end = 0;
    while i < toks.len() {
        let t = &toks[i];
        if i >= images_end {
            in_images = false;
        }
        match t {
            Tok::Open { name, attrs, .. }
                if name == "h2"
                    && has_class(attrs, "workshop-item-title")
                    && d.title.is_empty() =>
            {
                let e = element_end(&toks, i);
                d.title = text_of(&toks[i..e]);
            }
            Tok::Open { name, attrs, .. }
                if name == "link" && attr(attrs, "rel") == Some("canonical") && d.id == 0 =>
            {
                if let Some((id, slug)) = attr(attrs, "href").and_then(parse_item_href) {
                    d.id = id;
                    d.slug = slug;
                }
            }
            Tok::Open { name, attrs, .. }
                if name == "meta" && attr(attrs, "property") == Some("og:url") && d.id == 0 =>
            {
                if let Some((id, slug)) = attr(attrs, "content").and_then(parse_item_href) {
                    d.id = id;
                    d.slug = slug;
                }
            }
            Tok::Open { attrs, .. } if attr(attrs, "id") == Some("workshop-images") => {
                in_images = true;
                images_end = element_end(&toks, i);
            }
            Tok::Open { name, attrs, .. } if in_images && (name == "a" || name == "img") => {
                let url = if name == "a" {
                    attr(attrs, "href")
                } else {
                    attr(attrs, "src")
                };
                if let Some(u) = url {
                    if u.contains("/workshop/image/") && !d.images.iter().any(|x| x == u) {
                        d.images.push(u.to_string());
                    }
                }
            }
            Tok::Open { name, attrs, .. }
                if name == "a"
                    && d.submitter.is_empty()
                    && attr(attrs, "href").is_some_and(|h| h.contains("/workshop/user/")) =>
            {
                let e = element_end(&toks, i);
                let mut n = String::new();
                for k in i..e {
                    if let Tok::Open { attrs, .. } = &toks[k] {
                        if has_class(attrs, "user-username") {
                            n = text_of(&toks[k..element_end(&toks, k)]);
                        }
                    }
                }
                d.submitter = n;
            }
            Tok::Open { name, .. } if name == "tr" => {
                let e = element_end(&toks, i);
                parse_row(&toks[i..e], &mut d);
                i = e;
                continue;
            }
            Tok::Open { name, .. } if name == "h5" => {
                let e = element_end(&toks, i);
                let head = text_of(&toks[i..e]).to_ascii_lowercase();
                let target = if head == "description" {
                    Some(0)
                } else if head.starts_with("installation") {
                    Some(1)
                } else {
                    None
                };
                if let Some(which) = target {
                    // the next div.markdown-body
                    if let Some(off) = toks[e..].iter().position(|t| matches!(t, Tok::Open { name, attrs, .. } if name == "div" && has_class(attrs, "markdown-body"))) {
                        let s = e + off;
                        let ee = element_end(&toks, s);
                        let text = render_block(&toks[s + 1..ee]);
                        if which == 0 {
                            d.description = text;
                        } else {
                            d.install_instructions = text;
                        }
                    }
                }
                i = e;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    // fallbacks: the main Download button when the table is missing
    if d.files.is_empty() {
        for (k, t) in toks.iter().enumerate() {
            if let Tok::Open { name, attrs, .. } = t {
                if name == "a" && has_class(attrs, "btn") {
                    if let Some(f) = attr(attrs, "href").and_then(file_from_url) {
                        let e = element_end(&toks, k);
                        let mut f = f;
                        f.size_label = d
                            .extra
                            .iter()
                            .find(|(l, _)| l == "Filesize")
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default();
                        f.size = parse_size(&f.size_label).unwrap_or(0);
                        f.date = d
                            .updated
                            .clone()
                            .or(d.submitted.clone())
                            .unwrap_or_default();
                        let _ = e;
                        d.files.push(f);
                        break;
                    }
                }
            }
        }
    }
    d.extra.retain(|(l, _)| l != "Filesize");
    if d.title.is_empty() && d.files.is_empty() {
        return Err(Error::parse(
            "item page",
            "neither a title nor a download was found",
        ));
    }
    Ok(d)
}

fn file_from_url(href: &str) -> Option<FileEntry> {
    let p = href.find("/workshop/download/")?;
    let rest = &href[p + "/workshop/download/".len()..];
    let mut it = rest.splitn(3, '/');
    let _item: u32 = it.next()?.parse().ok()?;
    let id: u32 = it.next()?.parse().ok()?;
    let name = it
        .next()
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_string();
    Some(FileEntry {
        id,
        name,
        url: href.to_string(),
        ..Default::default()
    })
}

fn parse_row(row: &[Tok], d: &mut ItemDetail) {
    // cells
    struct Cell<'a> {
        toks: &'a [Tok],
    }
    let mut cells: Vec<Cell> = vec![];
    let mut k = 1;
    while k < row.len() {
        if matches!(&row[k], Tok::Open { name, .. } if name == "td" || name == "th") {
            let e = element_end(row, k);
            cells.push(Cell {
                toks: &row[k + 1..e.saturating_sub(1).max(k + 1)],
            });
            k = e;
        } else {
            k += 1;
        }
    }
    if cells.len() < 2
        || row
            .iter()
            .any(|t| matches!(t, Tok::Open { name, .. } if name == "th"))
    {
        return; // header rows (`File | Mirrors | ...`) carry no data
    }
    // Downloads table row
    let link = cells[0].toks.iter().find_map(|t| match t {
        Tok::Open { name, attrs, .. } if name == "a" => attr(attrs, "href").and_then(file_from_url),
        _ => None,
    });
    if let Some(mut f) = link {
        let first = text_of(cells[0].toks);
        f.broken = first.to_ascii_lowercase().contains("(broken)") || cells[0].toks.iter().any(|t| matches!(t, Tok::Open { name, .. } if name == "s" || name == "strike" || name == "del"));
        // the link text is the real name; the URL's last part is percent-encoded
        f.name = cells[0]
            .toks
            .iter()
            .find_map(|t| match t {
                Tok::Text(s) if !collapse(s).eq_ignore_ascii_case("(broken)") => Some(collapse(s)),
                _ => None,
            })
            .unwrap_or_else(|| crate::util::url_decode(&f.name));
        for t in cells[1].toks {
            if let Tok::Open { name, attrs, .. } = t {
                if name == "a" {
                    if let Some(h) = attr(attrs, "href") {
                        f.mirrors.push(h.to_string());
                    }
                }
            }
        }
        if cells.len() >= 5 {
            f.size_label = text_of(cells[2].toks);
            f.size = parse_size(&f.size_label).unwrap_or(0);
            f.downloads = text_of(cells[3].toks).replace(',', "").parse().unwrap_or(0);
            f.date = text_of(cells[4].toks);
        } else if cells.len() >= 3 {
            f.size_label = text_of(cells[2].toks);
            f.size = parse_size(&f.size_label).unwrap_or(0);
        }
        if !d.files.iter().any(|x| x.id == f.id) {
            d.files.push(f);
        }
        return;
    }
    // Info table row: label, value
    let label = text_of(cells[0].toks);
    let value = text_of(cells[1].toks);
    if label.is_empty() {
        return;
    }
    let spans: Vec<&[(String, String)]> = cells[1]
        .toks
        .iter()
        .filter_map(|t| match t {
            Tok::Open { name, attrs, .. }
                if name == "span" && attr(attrs, "data-workshop-rating-type").is_some() =>
            {
                Some(attrs.as_slice())
            }
            _ => None,
        })
        .collect();
    let score = spans
        .first()
        .and_then(|a| attr(a, "data-workshop-rating-score"))
        .and_then(to_f32);
    // "8 ratings" / "1 rating"
    let words: Vec<&str> = value.split_whitespace().collect();
    let count: Option<u32> = words
        .windows(2)
        .find(|p| p[1].starts_with("rating"))
        .and_then(|p| p[0].parse().ok());
    match label.as_str() {
        "Category" => d.category = Category::from_label(&value),
        "Rating" => {
            d.rating = score.or_else(|| value.split_whitespace().next().and_then(to_f32));
            d.rating_count = count;
        }
        "Difficulty" => {
            d.difficulty = score.or_else(|| value.split_whitespace().next().and_then(to_f32));
            d.difficulty_count = count;
        }
        "Original creation date" => d.original_date = Some(value),
        "Submission date" => d.submitted = Some(value),
        "Last updated" => d.updated = Some(value),
        "Min. game version" => d.min_game_version = Some(value),
        "Original author" => d.original_author = Some(value),
        "Downloads" => d.downloads = value.replace(',', "").parse().unwrap_or(0),
        "Submitted by" => {}
        _ => d.extra.push((label, value)),
    }
}

/// Parse the (possibly stale) JSON item answer `/api/v1/workshop/item/<id>`.
pub fn parse_item_api(json: &str, id: u32) -> Result<ItemDetail> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| Error::parse("item json", e.to_string()))?;
    let it = v
        .get("workshop_item")
        .ok_or_else(|| Error::parse("item json", "no workshop_item"))?;
    let mut d = ItemDetail {
        id,
        source: "api".into(),
        ..Default::default()
    };
    d.title = it
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    for f in it
        .get("files")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        let url = f
            .get("url")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        d.files.push(FileEntry {
            id: f.get("id").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            name: f
                .get("filename")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            date: f
                .get("timestamp")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            size: f.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            size_label: crate::util::format_size(
                f.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            ),
            url,
            ..Default::default()
        });
    }
    Ok(d)
}

/// Parse the JSON search answer `/api/v1/workshop/search?q=` (richer than the HTML cards).
pub fn parse_search_api(json: &str) -> Result<Vec<ItemSummary>> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| Error::parse("search json", e.to_string()))?;
    let arr = v
        .get("workshop_items")
        .and_then(|x| x.as_array())
        .ok_or_else(|| {
            Error::parse(
                "search json",
                v.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("no workshop_items")
                    .to_string(),
            )
        })?;
    let mut out = vec![];
    for it in arr {
        let id = it.get("id").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        if id == 0 {
            continue;
        }
        let name = it
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let date = |k: &str| {
            it.get(k)
                .and_then(|d| d.get("date"))
                .and_then(|d| d.as_str())
                .map(|s| s.split(' ').next().unwrap_or(s).to_string())
        };
        out.push(ItemSummary {
            id,
            slug: crate::util::slug(&name).replace('_', "-"),
            title: name,
            category: it
                .get("category")
                .and_then(|c| c.as_u64())
                .and_then(|c| Category::from_id(c as u32)),
            submitter: it
                .get("submitter")
                .and_then(|s| s.get("username"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            thumbnail: it
                .get("thumbnail")
                .and_then(|t| t.as_str())
                .map(|t| format!("/workshop/image/{id}/{t}")),
            rating: it
                .get("ratingScore")
                .and_then(|x| x.as_f64())
                .map(|x| x as f32),
            difficulty: it
                .get("difficultyRatingScore")
                .and_then(|x| x.as_f64())
                .map(|x| x as f32),
            downloads: it
                .get("downloadCount")
                .and_then(|x| x.as_u64())
                .map(|x| x as u32),
            updated: date("updatedTimestamp"),
            description: it
                .get("description")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            min_game_build: it
                .get("minGameBuild")
                .and_then(|x| x.as_i64())
                .map(|x| x as i32),
        });
    }
    Ok(out)
}
