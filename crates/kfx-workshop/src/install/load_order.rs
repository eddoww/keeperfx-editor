//! Editing `mods/load_order.cfg`: the mod lists of its `[after_base]`, `[after_campaign]` and
//! `[after_map]` blocks, keeping the file's line endings and other lines.

pub(super) fn nl_of(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn block_range(lines: &[&str], stage: &str) -> Option<(usize, usize)> {
    let head = lines
        .iter()
        .position(|l| l.trim().eq_ignore_ascii_case(&format!("[{stage}]")))?;
    let end = lines[head + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |p| head + 1 + p);
    Some((head, end))
}

/// Mods listed (uncommented) in `stage`.
pub fn load_order_mods(text: &str, stage: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some((h, e)) = block_range(&lines, stage) else {
        return vec![];
    };
    lines[h + 1..e]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Is `name` in `stage`, possibly commented out (`;name`)?
pub(super) fn listed_anywhere(text: &str, stage: &str, name: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let Some((h, e)) = block_range(&lines, stage) else {
        return false;
    };
    lines[h + 1..e]
        .iter()
        .any(|l| l.trim().trim_start_matches([';', '#']).trim() == name)
}

/// `text` with `name` added to the end of `[stage]` (the block is created when missing).
pub fn load_order_add(text: &str, stage: &str, name: &str) -> String {
    let nl = nl_of(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    match block_range(&lines, stage) {
        Some((h, e)) => {
            let mut at = h + 1;
            for (k, l) in lines.iter().enumerate().take(e).skip(h + 1) {
                if !l.trim().is_empty() {
                    at = k + 1;
                }
            }
            out.insert(at, name.to_string());
        }
        None => {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(format!("[{stage}]"));
            out.push(name.to_string());
        }
    }
    let mut s = out.join(nl);
    s.push_str(nl);
    s
}

/// `text` without the line that is exactly `name` in `stage`.
pub fn load_order_remove(text: &str, stage: &str, name: &str) -> String {
    let nl = nl_of(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut drop = None;
    if let Some((h, e)) = block_range(&lines, stage) {
        drop = (h + 1..e).rev().find(|&k| lines[k].trim() == name);
    }
    let out: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(k, _)| Some(*k) != drop)
        .map(|(_, l)| *l)
        .collect();
    let mut s = out.join(nl);
    if !s.is_empty() {
        s.push_str(nl);
    }
    s
}

pub(super) fn fresh_load_order() -> String {
    "; KeeperFX Mods Order Configuration file version 1.0.\n; \
        Created by the KeeperFX editor Workshop installer.\n; Put a ; in front of a mod \
        to disable it. Mods loaded later win.\n\n[after_base]\n\n[after_campaign]\n\n\
        [after_map]\n"
        .to_string()
}
