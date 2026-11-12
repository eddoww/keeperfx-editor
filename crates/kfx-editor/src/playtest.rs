//! Play-testing: the game's command line for a level, launching it as a detached process, and
//! reading `keeperfx.log` afterwards.
//!
//! * Level selection (main.cpp:1804-1816, src/game_loop.c:846-866): `-level N` starts level N of the
//!   campaign named by `-campaign NAME`. `NAME` may carry a folder prefix (`campgns/`,
//!   `levels/`, `multiplayer/`, `cmpgn_prefix` in src/config_campaigns.c:58) that picks the list it
//!   is looked up in (`prepare_campaign_file_name`, :1272); `.cfg` is added when missing. The
//!   level files are read from the campaign's `LEVELS_LOCATION`. Map packs find their levels
//!   through `.lof` / `.lif` files (`find_and_load_lof_files`, src/config_campaigns.c:1327), so a
//!   loose map has to be put into a pack first: [`copy_into_personal`] copies it into
//!   `levels/personal` (the "Personal levels" pack, `levels/personal.cfg`) with a `.lof`.
//! * Options used here (main.cpp:1746-1897): `-nointro`, `-skipheartzoom`, `-nosound`,
//!   `-1player` (computer keepers for the other players of a multiplayer map), `-alex` (the
//!   game's easter-egg / cheat flag), `-cfg KEY=VALUE` (a `keeperfx.cfg` line, applied after the
//!   file, src/config_keeperfx.c:1301): windowed play is `INGAME_RES=<W>x<H>w32` (the `w` makes a
//!   windowed mode, `LbRegisterVideoModeString`, src/bflib_video.c:905-912).
//! * Script errors are logged as `Error: <func>(line N): <message>` (`SCRPTERRLOG`,
//!   src/globals.h:126) and Lua errors as `Lua error in <func>: <file>:N: <message>`
//!   (src/lua_base.c:118).
use crate::settings::PlaySettings;
use kfx_formats::campaign::{resolve, PackKind};
use std::path::{Path, PathBuf};

/// The game executable in an install: the native `keeperfx` on Linux / other Unix systems when
/// it is there, else `keeperfx.exe`.
pub fn detect_exe(install: &Path) -> Option<PathBuf> {
    let native = install.join("keeperfx");
    let exe = kfx_map::gamedata::find_ci(install, "keeperfx.exe");
    if cfg!(windows) {
        return exe;
    }
    if native.is_file() {
        return Some(native);
    }
    exe
}

/// Split an argument string on whitespace; `"..."` keeps spaces.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Where a level is played from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayTarget {
    /// A campaign / map pack whose levels folder holds the map: the `-campaign` value
    /// (`campgns/keeporig`, `levels/personal`, `multiplayer/classic`).
    Pack {
        campaign: String,
        kind: PackKind,
        name: String,
        multiplayer: bool,
        listed: bool,
    },
    /// Outside every campaign / pack folder.
    Loose,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The campaign / map pack a level of `folder` is played with (campaigns first, then packs that
/// list the level).
pub fn play_target(install: &Path, folder: &Path, level: u32) -> PlayTarget {
    let mut best: Option<(u8, PlayTarget)> = None;
    for kind in [
        PackKind::Campaign,
        PackKind::Mappack,
        PackKind::MultiplayerMappack,
    ] {
        let Some(dir) = kfx_gfx::install::resolve(install, kind.folder()).filter(|d| d.is_dir())
        else {
            continue;
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            let Ok(r) = resolve(install, &cfg) else {
                continue;
            };
            if !r.levels_dir.as_deref().is_some_and(|d| same_dir(d, folder)) {
                continue;
            }
            let listed = r.levels.iter().any(|l| l.number == level);
            let rank = match (kind, listed) {
                (PackKind::Campaign, true) => 0,
                (_, true) => 1,
                (PackKind::Campaign, false) => 2,
                _ => 3,
            };
            if best.as_ref().is_some_and(|(b, _)| *b <= rank) {
                continue;
            }
            let stem = cfg
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let prefix = match kind {
                PackKind::Campaign => "campgns",
                PackKind::Mappack => "levels",
                PackKind::MultiplayerMappack => "multiplayer",
            };
            best = Some((
                rank,
                PlayTarget::Pack {
                    campaign: format!("{prefix}/{stem}"),
                    kind,
                    name: r.name.clone(),
                    multiplayer: kind == PackKind::MultiplayerMappack,
                    listed,
                },
            ));
        }
    }
    best.map(|(_, t)| t).unwrap_or(PlayTarget::Loose)
}

/// A process to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Working directory: the game folder (the game reads its files relative to it).
    pub cwd: PathBuf,
}

impl Launch {
    /// The command as one line (for the UI and logs).
    pub fn display(&self) -> String {
        let q = |s: &str| {
            if s.contains(' ') || s.is_empty() {
                format!("\"{s}\"")
            } else {
                s.to_string()
            }
        };
        let mut v = vec![q(&self.program.display().to_string())];
        v.extend(self.args.iter().map(|a| q(a)));
        format!(
            "cd {} && {}",
            q(&self.cwd.display().to_string()),
            v.join(" ")
        )
    }
}

/// The game's own arguments for playing `level` of `campaign`.
pub fn game_args(p: &PlaySettings, campaign: &str, level: u32, multiplayer: bool) -> Vec<String> {
    let mut a = vec![
        "-campaign".to_string(),
        campaign.to_string(),
        "-level".to_string(),
        level.to_string(),
    ];
    if p.no_intro {
        a.push("-nointro".into());
    }
    if p.skip_heart_zoom {
        a.push("-skipheartzoom".into());
    }
    if p.cheats {
        a.push("-alex".into());
    }
    if p.no_sound {
        a.push("-nosound".into());
    }
    if multiplayer && p.one_player_mp {
        a.push("-1player".into());
    }
    for (on, flag) in [
        (p.alt_input, "-altinput"),
        (p.show_game_turns, "-show_game_turns"),
        (p.frame_step, "-framestep"),
        (p.timer, "-timer"),
        (p.dbg_pathfind, "-dbgpathfind"),
        (p.dbg_shots, "-dbgshots"),
        (p.no_mods, "-nomods"),
    ] {
        if on {
            a.push(flag.into());
        }
    }
    if p.windowed {
        let res = p.window_res.trim();
        let res = if res.split('x').count() == 2
            && res.split('x').all(|n| n.trim().parse::<u32>().is_ok())
        {
            res
        } else {
            "1280x800"
        };
        a.push("-cfg".into());
        a.push(format!("INGAME_RES={res}w32"));
        a.push("-cfg".into());
        a.push(format!("FRONTEND_RES={res}w32 {res}w32 {res}w32"));
    }
    a.extend(split_args(&p.extra_args));
    a
}

/// The full launch: wrapper (or `wine` for a `.exe` outside Windows), executable, arguments,
/// working directory = the executable's folder.
pub fn build_launch(
    p: &PlaySettings,
    exe: &Path,
    campaign: &str,
    level: u32,
    multiplayer: bool,
) -> Launch {
    let cwd = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut wrapper = split_args(&p.wrapper);
    let is_exe = exe
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    if wrapper.is_empty() && is_exe && !cfg!(windows) {
        wrapper.push("wine".into());
    }
    let game = game_args(p, campaign, level, multiplayer);
    match wrapper.split_first() {
        Some((prog, rest)) => {
            let mut args: Vec<String> = rest.to_vec();
            args.push(exe.display().to_string());
            args.extend(game);
            Launch {
                program: PathBuf::from(prog),
                args,
                cwd,
            }
        }
        None => Launch {
            program: exe.to_path_buf(),
            args: game,
            cwd,
        },
    }
}

/// Start the game without waiting for it. A background thread reaps the process when it ends
/// and reports through `done` (so that the editor can refresh the game log).
pub fn spawn_detached(
    l: &Launch,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<u32, String> {
    use std::process::{Command, Stdio};
    let mut c = Command::new(&l.program);
    c.args(&l.args)
        .current_dir(&l.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = c
        .spawn()
        .map_err(|e| format!("{}: {e}", l.program.display()))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
        done.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    Ok(pid)
}

/// The next level number without files in `folder` (from `from`).
pub fn next_free_level(folder: &Path, from: u32) -> u32 {
    let mut n = from.max(1);
    while !crate::document::level_files(folder, n).is_empty() {
        n += 1;
    }
    n
}

/// Copy the files of `level` in `src` into `levels/personal` of the install as level `dest`
/// (lower-case names, an existing level `dest` is not touched: choose a free number), with a
/// `.lof` naming it and giving its size (`KIND = FREE`). Returns the folder.
pub fn copy_into_personal(
    install: &Path,
    src: &Path,
    level: u32,
    dest: u32,
    name: &str,
    size: (u16, u16),
) -> Result<PathBuf, String> {
    let pack = kfx_gfx::install::resolve(install, "levels/personal")
        .unwrap_or_else(|| install.join("levels").join("personal"));
    std::fs::create_dir_all(&pack).map_err(|e| format!("{}: {e}", pack.display()))?;
    if !crate::document::level_files(&pack, dest).is_empty() {
        return Err(format!("map{dest:05} already exists in {}", pack.display()));
    }
    let sp = format!("map{level:05}.");
    for f in crate::document::level_files(src, level) {
        let n = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let Some(ext) = n.strip_prefix(&sp) else {
            continue;
        };
        if ext == "lof" || ext == "lif" {
            continue;
        }
        let to = pack.join(format!("map{dest:05}.{ext}"));
        std::fs::copy(&f, &to).map_err(|e| format!("{} -> {}: {e}", f.display(), to.display()))?;
    }
    let lof = format!(
        "; KeeperFX Level Overview File (LOF)\nNAME_TEXT = {}\nKIND = FREE\nMAPSIZE = {} {}\n",
        name.replace(['\n', '\r'], " "),
        size.0,
        size.1
    );
    let p = pack.join(format!("map{dest:05}.lof"));
    std::fs::write(&p, lof).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(pack)
}

// ------------------------------------------------------------------------------------- log

/// Severity of a game log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error,
    Warning,
}

/// Which script a log line points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    Dk,
    Lua,
}

/// An error or warning of `keeperfx.log`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
    /// 1-based line number in the log file.
    pub log_line: usize,
    /// The script line the message is about.
    pub script: Option<(ScriptKind, usize)>,
}

/// Errors and warnings of a game log, with script lines linked: `func(line N): ...` is a line of
/// the level's DK script (`SCRPTERRLOG` / `SCRPTWRNLOG`, src/globals.h:126-127; `text_line_number`
/// is the line of `mapNNNNN.txt` being parsed), `....lua:N:` a line of a Lua file (only
/// `mapNNNNN.lua` files are linked).
pub fn parse_log(text: &str) -> Vec<LogEntry> {
    let mut out = vec![];
    for (i, raw) in text.lines().enumerate() {
        let (level, rest) = if let Some(r) = raw.strip_prefix("Error: ") {
            (LogLevel::Error, r)
        } else if let Some(r) = raw.strip_prefix("Warning: ") {
            (LogLevel::Warning, r)
        } else {
            continue;
        };
        out.push(LogEntry {
            level,
            text: rest.to_string(),
            log_line: i + 1,
            script: script_link(rest),
        });
    }
    out
}

fn script_link(s: &str) -> Option<(ScriptKind, usize)> {
    let digits = |r: &str| -> Option<usize> {
        let n: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        n.parse().ok()
    };
    if let Some(i) = s.to_ascii_lowercase().find(".lua:") {
        let before = &s[..i];
        let file = before
            .rsplit(['/', '\\', ' ', '"', '\''])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if file.starts_with("map")
            && file[3..].bytes().all(|b| b.is_ascii_digit())
            && file.len() > 3
        {
            return digits(&s[i + 5..]).map(|n| (ScriptKind::Lua, n));
        }
        return None;
    }
    let i = s.find("(line ")?;
    // only "func(line N): " forms
    let n = digits(&s[i + 6..])?;
    s[i + 6..]
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .starts_with("):")
        .then_some((ScriptKind::Dk, n))
}

/// `keeperfx.log` of a game folder (read as Latin-1).
pub fn read_log(game_dir: &Path) -> Option<String> {
    let p = kfx_map::gamedata::find_ci(game_dir, "keeperfx.log")?;
    std::fs::read(&p)
        .ok()
        .map(|b| kfx_formats::cfg::latin1_decode(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> PlaySettings {
        PlaySettings {
            no_intro: true,
            windowed: false,
            cheats: false,
            skip_heart_zoom: false,
            no_sound: false,
            one_player_mp: true,
            ..Default::default()
        }
    }

    #[test]
    fn args_split_with_quotes() {
        assert_eq!(
            split_args(r#"-fps 30  -cfg "INGAME_RES=800x600w32"  "" x"#),
            vec!["-fps", "30", "-cfg", "INGAME_RES=800x600w32", "", "x"]
        );
        assert!(split_args("  ").is_empty());
    }

    #[test]
    fn command_line_native_and_options() {
        let mut p = settings();
        let l = build_launch(
            &p,
            Path::new("/games/kfx/keeperfx"),
            "campgns/keeporig",
            7,
            false,
        );
        assert_eq!(l.program, PathBuf::from("/games/kfx/keeperfx"));
        assert_eq!(l.cwd, PathBuf::from("/games/kfx"));
        assert_eq!(
            l.args,
            ["-campaign", "campgns/keeporig", "-level", "7", "-nointro"]
        );
        p.windowed = true;
        p.window_res = "1024x768".into();
        p.cheats = true;
        p.skip_heart_zoom = true;
        p.no_sound = true;
        p.extra_args = "-fps 30".into();
        let l = build_launch(
            &p,
            Path::new("/g/keeperfx"),
            "multiplayer/classic",
            12,
            true,
        );
        assert_eq!(
            l.args,
            [
                "-campaign",
                "multiplayer/classic",
                "-level",
                "12",
                "-nointro",
                "-skipheartzoom",
                "-alex",
                "-nosound",
                "-1player",
                "-cfg",
                "INGAME_RES=1024x768w32",
                "-cfg",
                "FRONTEND_RES=1024x768w32 1024x768w32 1024x768w32",
                "-fps",
                "30"
            ]
        );
        // a bad resolution falls back
        p.window_res = "huge".into();
        assert!(game_args(&p, "x", 1, false).contains(&"INGAME_RES=1280x800w32".to_string()));
    }

    #[test]
    fn command_line_wrapper() {
        let mut p = settings();
        let l = build_launch(
            &p,
            Path::new("/g/keeperfx.exe"),
            "levels/personal",
            3,
            false,
        );
        if cfg!(windows) {
            assert_eq!(l.program, PathBuf::from("/g/keeperfx.exe"));
        } else {
            assert_eq!(l.program, PathBuf::from("wine"));
            assert_eq!(l.args[0], "/g/keeperfx.exe");
            assert_eq!(
                &l.args[1..5],
                ["-campaign", "levels/personal", "-level", "3"]
            );
        }
        p.wrapper = "env WINEDEBUG=-all wine".into();
        let l = build_launch(
            &p,
            Path::new("/g/keeperfx.exe"),
            "levels/personal",
            3,
            false,
        );
        assert_eq!(l.program, PathBuf::from("env"));
        assert_eq!(&l.args[..3], ["WINEDEBUG=-all", "wine", "/g/keeperfx.exe"]);
        assert!(l
            .display()
            .starts_with("cd /g && env WINEDEBUG=-all wine /g/keeperfx.exe -campaign"));
    }

    #[test]
    fn targets_of_shipped_levels() {
        let Some(root) = kfx_formats::testutil::install_dir() else {
            return;
        };
        let camp = kfx_formats::testutil::resolve_ci(&root, "campgns/keeporig").unwrap();
        match play_target(&root, &camp, 1) {
            PlayTarget::Pack {
                campaign,
                multiplayer,
                listed,
                ..
            } => {
                assert!(
                    campaign.eq_ignore_ascii_case("campgns/keeporig"),
                    "{campaign}"
                );
                assert!(!multiplayer && listed);
            }
            t => panic!("{t:?}"),
        }
        if let Some(mp) = kfx_formats::testutil::resolve_ci(&root, "levels/classic") {
            if let PlayTarget::Pack { campaign, .. } = play_target(&root, &mp, 1) {
                assert!(
                    campaign.to_ascii_lowercase().starts_with("levels/"),
                    "{campaign}"
                );
            }
        }
        assert_eq!(
            play_target(&root, &std::env::temp_dir(), 1),
            PlayTarget::Loose
        );
        assert!(detect_exe(&root).is_some());
    }

    #[test]
    fn log_lines_link_scripts() {
        let log = "LOG CREATED\nError: script_scan_line(line 12): Unrecognized command, \
        'FOO_BAR'\nWarning: [0] some_func: something\nError: CheckLua: Lua error in \
        OnGameStart: /srv/keeperfx/levels/personal/map00003.lua:7: attempt to call a nil \
        value\nError: Lua error in x: fxdata/lua/init.lua:5: y\nScript(line 15): fine\n";
        let e = parse_log(log);
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].script, Some((ScriptKind::Dk, 12)));
        assert_eq!(e[0].level, LogLevel::Error);
        assert_eq!(e[0].log_line, 2);
        assert_eq!(e[1].script, None);
        assert_eq!(e[1].level, LogLevel::Warning);
        assert_eq!(e[2].script, Some((ScriptKind::Lua, 7)));
        assert_eq!(e[3].script, None);
    }
}
