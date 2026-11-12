//! Reading what Lua prints: the game writes `print` output to `keeperfx.log` in its folder.
//! [`LogTail`] follows that file; [`parse_things`] reads the `KFXLIVE THING` lines that
//! [`crate::actions::DUMP_HELPER`] prints. This is how an "in-game positions" overlay gets its
//! data: the API itself has no call that returns things (every reply is limited to 1024 bytes).
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Prefix of the lines the helper prints.
pub const MARKER: &str = "KFXLIVE ";

pub struct LogTail {
    path: PathBuf,
    pos: u64,
    partial: String,
}

impl LogTail {
    /// Follow `path`, starting at its current end (what was logged before is ignored).
    pub fn new(path: impl AsRef<Path>) -> LogTail {
        let path = path.as_ref().to_path_buf();
        let pos = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        LogTail {
            path,
            pos,
            partial: String::new(),
        }
    }

    /// Follow `path` from its start.
    pub fn from_start(path: impl AsRef<Path>) -> LogTail {
        LogTail {
            path: path.as_ref().to_path_buf(),
            pos: 0,
            partial: String::new(),
        }
    }

    /// The complete new lines since the last call. A shorter file (the game started again) restarts
    /// at 0.
    pub fn poll(&mut self) -> Vec<String> {
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return vec![];
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.pos {
            self.pos = 0;
            self.partial.clear();
        }
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return vec![];
        }
        let mut buf = vec![];
        if f.read_to_end(&mut buf).is_err() {
            return vec![];
        }
        self.pos += buf.len() as u64;
        self.partial.push_str(&String::from_utf8_lossy(&buf));
        let mut out = vec![];
        while let Some(i) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=i).collect();
            out.push(line.trim_end_matches(['\n', '\r']).to_string());
        }
        out
    }
}

/// One thing of the game as the helper printed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveThing {
    pub class: String,
    pub model: String,
    pub owner: i32,
    /// Subtile position.
    pub stl_x: i32,
    pub stl_y: i32,
}

/// Parse `KFXLIVE THING <class> <model> <owner> <x> <y>` lines (anywhere in a log line: the game
/// prefixes its own timestamp / turn text).
pub fn parse_things(lines: &[String]) -> Vec<LiveThing> {
    lines
        .iter()
        .filter_map(|l| {
            let i = l.find(MARKER)?;
            let mut it = l[i + MARKER.len()..].split_whitespace();
            if it.next()? != "THING" {
                return None;
            }
            Some(LiveThing {
                class: it.next()?.to_string(),
                model: it.next()?.to_string(),
                owner: it.next()?.parse().ok()?,
                stl_x: it.next()?.parse().ok()?,
                stl_y: it.next()?.parse().ok()?,
            })
        })
        .collect()
}
