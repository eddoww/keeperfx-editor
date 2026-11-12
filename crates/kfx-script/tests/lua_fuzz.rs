//! Robustness: real Lua files truncated at every 97th byte and at random points, plus random
//! single-token damage. No panics, bounded time per document.
use kfx_formats::testutil;
use kfx_script::lua::diagnostics::diagnostics;
use kfx_script::lua::doc::LuaDoc;
use kfx_script::lua::index::ApiIndex;
use kfx_script::lua::lexer::{lex, reassemble};
use kfx_script::lua::nav::{definition, format, outline, positions, references, LuaFormatOptions};
use kfx_script::lua::services::{completions, highlight, hover, signature_help};
use kfx_script::NullProvider;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p
            .extension()
            .map_or(false, |x| x.eq_ignore_ascii_case("lua"))
        {
            out.push(p);
        }
    }
}

fn files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = testutil::install_dir() {
        walk(&d.join("fxdata"), &mut v);
        walk(&d.join("campgns"), &mut v);
        walk(&d.join("levels"), &mut v);
    }
    v.sort();
    v
}

fn boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Run every service on `src` with the cursor at `off`; returns the elapsed time.
fn exercise(src: &str, off: usize, ix: &ApiIndex) -> Duration {
    let t = Instant::now();
    let lexed = lex(src);
    assert_eq!(
        reassemble(src, &lexed.tokens),
        src,
        "lossless lexing of a truncated file"
    );
    let d = LuaDoc::parse(src);
    let diag = diagnostics(&d, ix, &NullProvider);
    for x in &diag {
        assert!(
            x.span.start <= x.span.end && x.span.end <= src.len(),
            "span out of range: {x:?}"
        );
        assert!(
            src.is_char_boundary(x.span.start) && src.is_char_boundary(x.span.end),
            "span not on a char boundary: {x:?}"
        );
    }
    for (s, _) in highlight(&d, ix) {
        assert!(s.end <= src.len());
    }
    let _ = outline(&d);
    let _ = references(&d, ix);
    let _ = positions(&d, ix);
    let _ = format(&d, &LuaFormatOptions::default());
    let off = boundary(src, off);
    let _ = completions(&d, off, ix, &NullProvider);
    let _ = signature_help(&d, off, ix);
    let _ = hover(&d, off, ix);
    let _ = definition(&d, off, ix);
    t.elapsed()
}

#[test]
fn truncation_fuzz() {
    let fs = files();
    if fs.is_empty() {
        eprintln!("skip: no install");
        return;
    }
    let ix = ApiIndex::stock();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let (mut runs, mut worst) = (0usize, Duration::ZERO);
    let total = Instant::now();
    for f in &fs {
        let full = String::from_utf8_lossy(&std::fs::read(f).unwrap()).into_owned();
        // every 97th byte
        let mut cuts: Vec<usize> = (0..=full.len()).step_by(97).collect();
        // random points
        for _ in 0..60 {
            cuts.push((rng.next() % (full.len() as u64 + 1)) as usize);
        }
        for c in cuts {
            let c = boundary(&full, c);
            let part = &full[..c];
            let dt = exercise(part, c, &ix);
            worst = worst.max(dt);
            runs += 1;
            assert!(
                dt < Duration::from_secs(2),
                "{} cut at {c} took {dt:?}",
                f.display()
            );
        }
        // random single-token damage: delete one token, or duplicate one
        let lexed = lex(&full);
        for _ in 0..25 {
            let toks: Vec<_> = lexed
                .tokens
                .iter()
                .filter(|t| !t.is_trivia() && t.span.len() > 0)
                .collect();
            if toks.is_empty() {
                break;
            }
            let t = toks[(rng.next() % toks.len() as u64) as usize];
            let mut s = String::with_capacity(full.len() + 16);
            s.push_str(&full[..t.span.start]);
            if rng.next() % 2 == 0 {
                s.push_str(&full[t.span.end..]);
            } else {
                s.push_str(&full[t.span.start..t.span.end]);
                s.push_str(&full[t.span.start..]);
            }
            let dt = exercise(&s, t.span.start, &ix);
            worst = worst.max(dt);
            runs += 1;
            assert!(
                dt < Duration::from_secs(2),
                "{} damaged at {} took {dt:?}",
                f.display(),
                t.span.start
            );
        }
    }
    println!(
        "fuzz: {} files, {} documents exercised, worst single document {:?}, total {:?}",
        fs.len(),
        runs,
        worst,
        total.elapsed()
    );
}

#[test]
fn byte_noise_fuzz() {
    // arbitrary bytes (lossily decoded) must never panic the lexer, parser or services
    let ix = ApiIndex::stock();
    let mut rng = Rng(0xDEADBEEFCAFEF00D);
    let alphabet: Vec<&str> = vec![
        "function",
        "end",
        "if",
        "then",
        "else",
        "elseif",
        "for",
        "in",
        "do",
        "while",
        "repeat",
        "until",
        "local",
        "return",
        "(",
        ")",
        "{",
        "}",
        "[",
        "]",
        ",",
        ";",
        ":",
        ".",
        "..",
        "...",
        "=",
        "==",
        "~=",
        "+",
        "-",
        "*",
        "/",
        "\"",
        "'",
        "[[",
        "]]",
        "--",
        "---@param",
        "::",
        "goto",
        "x",
        "y1",
        "PLAYER0",
        "CreateEffect",
        "1",
        "0x",
        "3.",
        "\n",
        " ",
        "é",
        "\t",
        "\\",
    ];
    for _ in 0..400 {
        let n = (rng.next() % 60) as usize;
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(alphabet[(rng.next() % alphabet.len() as u64) as usize]);
            if rng.next() % 3 == 0 {
                s.push(' ');
            }
        }
        let off = (rng.next() % (s.len() as u64 + 1)) as usize;
        let dt = exercise(&s, off, &ix);
        assert!(dt < Duration::from_secs(2), "{s:?} took {dt:?}");
    }
}
