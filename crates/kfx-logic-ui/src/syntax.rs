//! Script text helpers: canonical command text, argument kinds, normalised command keys.
use crate::model::Cmd;
use kfx_script::overlay::OverlayArg;
use kfx_script::tables::ARGS_COUNT;
use kfx_script::{ArgKind, ArgTok, Command, Document, Overlay, Tables};

/// Latin-1 decode (every byte is one char; lossless).
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Latin-1 encode (chars above U+00FF become `?`).
pub fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            if (c as u32) < 256 {
                c as u32 as u8
            } else {
                b'?'
            }
        })
        .collect()
}

/// Kind, `extended` flag and "optional" flag of argument `slot` of command `name`
/// (letters of `command_desc`, `src/lvl_script_commands.c:7020`, refined by the overlay).
pub fn arg_kind(name: &str, slot: usize, version: i64) -> (ArgKind, bool, bool) {
    let t = Tables::get();
    let d = t.find(version, name).or_else(|| t.find_full(name));
    let mut letter = 0u8;
    let mut ext = false;
    if let Some(d) = d {
        let mut src = 0usize;
        let mut dst = 0usize;
        while src < ARGS_COUNT {
            let c = d.letter(src);
            if c == b'!' {
                src += 1;
                continue;
            }
            if dst == slot {
                letter = c;
                ext = d.letter(src + 1) == b'!';
                break;
            }
            if d.letter(src + 1) == b'+' {
                letter = c;
                break;
            }
            dst += 1;
            src += 1;
        }
    }
    let optional = letter.is_ascii_lowercase();
    let lk = ArgKind::from_letter(letter);
    let k = match Overlay::get().arg(name, slot) {
        Some(o)
            if matches!(o.kind, ArgKind::Text | ArgKind::Number)
                && !matches!(lk, ArgKind::Text | ArgKind::Number) =>
        {
            lk
        }
        Some(o) => o.kind,
        None => lk,
    };
    (k, ext, optional)
}

/// Raw type letter of a slot (`O` for operators), 0 past the end.
pub fn slot_letter(name: &str, slot: usize, version: i64) -> u8 {
    let t = Tables::get();
    let Some(d) = t.find(version, name).or_else(|| t.find_full(name)) else {
        return 0;
    };
    let mut src = 0usize;
    let mut dst = 0usize;
    while src < ARGS_COUNT {
        let c = d.letter(src);
        if c == b'!' {
            src += 1;
            continue;
        }
        if dst == slot {
            return c.to_ascii_uppercase();
        }
        if d.letter(src + 1) == b'+' {
            return c.to_ascii_uppercase();
        }
        dst += 1;
        src += 1;
    }
    0
}

/// Number of declared argument slots (repeating commands: the declared ones).
pub fn arity(name: &str, version: i64) -> usize {
    let t = Tables::get();
    t.find(version, name)
        .or_else(|| t.find_full(name))
        .map(|d| d.arity())
        .unwrap_or(0)
}

/// Required argument count (`count_required_parameters`, `src/lvl_script.c:404`).
pub fn required(name: &str, version: i64) -> usize {
    let t = Tables::get();
    t.find(version, name)
        .or_else(|| t.find_full(name))
        .map(|d| d.required())
        .unwrap_or(0)
}

/// Overlay name of a slot (or `argN`).
pub fn arg_name(name: &str, slot: usize) -> String {
    Overlay::get()
        .arg(name, slot)
        .map(|o: &OverlayArg| o.name.clone())
        .unwrap_or_else(|| format!("arg{}", slot + 1))
}

/// One-line doc of a command from the overlay.
pub fn command_doc(name: &str) -> String {
    Overlay::get()
        .cmd(name)
        .map(|c| c.doc.clone())
        .unwrap_or_default()
}

/// Canonical text of a command: `NAME(a,b,c)`, operator slots written `a op b`, no parentheses
/// when there are no arguments.
pub fn canonical(cmd: &Cmd, version: i64) -> String {
    let mut args: Vec<&str> = cmd.args.iter().map(|s| s.as_str()).collect();
    while args.last().map(|a| a.is_empty()).unwrap_or(false) {
        args.pop();
    }
    if args.is_empty() {
        return cmd.name.clone();
    }
    let mut s = String::new();
    s.push_str(&cmd.name);
    s.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            if slot_letter(&cmd.name, i, version) == b'O' {
                s.push(' ');
            } else if slot_letter(&cmd.name, i - 1, version) == b'O' {
                s.push(' ');
            } else {
                s.push(',');
            }
        }
        s.push_str(a);
    }
    s.push(')');
    s
}

/// The source text of each argument of a parsed command (function calls kept whole).
pub fn arg_texts(doc: &Document, c: &Command) -> Vec<String> {
    c.args
        .iter()
        .map(|a| {
            if a.func.is_some() {
                doc.text_lossy(a.span)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join("")
            } else if a.tok == ArgTok::Str {
                format!("\"{}\"", a.text)
            } else {
                doc.text_lossy(a.text_span)
            }
        })
        .collect()
}

/// Normalised key of a parsed command for semantic comparison: name and arguments as the game
/// reads them (identifiers upper case, strings verbatim, function calls whole).
pub fn command_key(doc: &Document, c: &Command) -> String {
    let mut s = c.name.clone();
    if !c.syntax_ok || c.desc.is_none() {
        s.push_str("!invalid");
    }
    s.push('(');
    for (i, a) in c.args.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let t = if a.func.is_some() {
            doc.text_lossy(a.span)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("")
                .to_ascii_uppercase()
        } else {
            match a.tok {
                ArgTok::Str => format!("\"{}\"", a.text),
                _ => a.text.clone(),
            }
        };
        s.push_str(&t);
    }
    s.push(')');
    s
}

/// Parse one line (in a script of level version `version`) and return its command key
/// (`None` when the line is not one command).
pub fn line_key(line: &str, version: i64) -> Option<String> {
    let text = if version > 0 {
        format!("LEVEL_VERSION({version})\n{line}\n")
    } else {
        format!("{line}\n")
    };
    let d = Document::parse(&latin1_bytes(&text));
    let l = d.lines.iter().filter(|l| l.cmd.is_some()).last()?;
    Some(command_key(&d, l.cmd.as_ref().unwrap()))
}

/// Value of a numeric argument (`strtol` rules, `0x` and leading zeros accepted like the game).
pub fn num(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (neg, b) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let v = if let Some(h) = b.strip_prefix("0x").or_else(|| b.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()?
    } else if b.len() > 1 && b.starts_with('0') {
        i64::from_str_radix(&b[1..], 8).ok()?
    } else {
        b.parse().ok()?
    };
    Some(if neg { -v } else { v })
}

/// The `IF*` commands that open a condition block (`src/lvl_script_conditions.c`).
pub fn is_if(name: &str) -> bool {
    kfx_script::validate::is_if_command(name)
}

/// A string literal argument for `text`.
pub fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}
