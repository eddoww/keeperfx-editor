//! Runs the real game on a map and returns its log.
//!
//! * [`Rig::new`] makes a private working copy of the install under
//!   `<scratch>/kfx-map-gametest/<name>` (`kfx_formats::testutil::scratch_dir`;
//!   `$KFX_GAME_RIG_ROOT/<name>` when that is set): the read-only folders are symlinked, the binary
//!   and `keeperfx.cfg` are copied (windowed 640x480, no cursor capture, no edge panning),
//!   `levels/personal` (the "Personal levels" map pack, `levels/personal.cfg`) is a fresh empty
//!   folder. Nothing is ever written to the install.
//! * [`Rig::install_level`] puts a saved map into that pack with a `.lif` (so the pack loads) and
//!   a `.lof` (`KIND = FREE`, `MAPSIZE`).
//! * [`Rig::run`] starts `./keeperfx -nointro -nosound -campaign personal -level N` with SDL's
//!   offscreen video driver and dummy audio (no window on the desktop), sends SIGTERM after the
//!   given time (SIGKILL 5 s later if needed) and parses `keeperfx.log`. The game has no option to
//!   quit after N turns (`src/main.cpp:1720-2040`), so the turn reached is read from the
//!   `packetfile:{"turn":N ...}` line the game's crash handler logs on SIGTERM.
//!
//! Only one game runs at a time: an in-process mutex plus a lock directory
//! (`/tmp/kfx-map-gametest.lock`, holding the owner pid; a stale one is taken over).
// Included by several test binaries (also of other crates); each uses a subset.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static RUN_LOCK: Mutex<()> = Mutex::new(());
const LOCK_DIR: &str = "/tmp/kfx-map-gametest.lock";

/// Severity prefix of a log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Error,
    Warning,
    Sync,
    Other,
}

/// One parsed `keeperfx.log` line: `Error: [turn] func(line N): message` and variants.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogLine {
    pub level: Level,
    pub func: Option<String>,
    pub msg: String,
    pub raw: String,
}

impl LogLine {
    pub fn parse(raw: &str) -> LogLine {
        let (level, rest) = if let Some(r) = raw.strip_prefix("Error: ") {
            (Level::Error, r)
        } else if let Some(r) = raw.strip_prefix("Warning: ") {
            (Level::Warning, r)
        } else if let Some(r) = raw.strip_prefix("Sync: ") {
            (Level::Sync, r)
        } else {
            (Level::Other, raw)
        };
        // optional "[N] " player prefix
        let rest = match rest.strip_prefix('[') {
            Some(r) => match r.find("] ") {
                Some(i) if r[..i].bytes().all(|b| b.is_ascii_digit()) => &r[i + 2..],
                _ => rest,
            },
            None => rest,
        };
        // optional "func: " or "func(line N): "
        let (func, msg) = match rest.find(": ") {
            Some(i) if !rest[..i].contains(' ') || rest[..i].contains("(line ") => {
                let f = &rest[..i];
                let f = f.split('(').next().unwrap_or(f);
                (Some(f.to_string()), rest[i + 2..].to_string())
            }
            _ => (None, rest.to_string()),
        };
        LogLine {
            level,
            func,
            msg,
            raw: raw.to_string(),
        }
    }

    /// Lines that vary between runs of the same map or come from stopping the game.
    pub fn is_noise(&self) -> bool {
        let r = &self.raw;
        r.starts_with("LOG CREATED")
            || r.starts_with("[#")
            || r.starts_with("./keeperfx(")
            || r.starts_with('/')
            || r.contains("Moon phase")
            || r.contains("New moon")
            || r.contains("Full moon")
            || r.contains("level_load_time_phase")
            || r.contains("Failure signal: Termination")
            || r.contains("Signal code:")
            || r.contains("Fault address:")
            || r.contains("packetfile:")
            || r.contains("Context RIP")
            || r.contains("Starting packet saving")
            || r.contains("High scores table bad")
            // GL sprite atlas slot numbers depend on what the map shows
            || r.contains("PackSprite(handle=")
            // the music track depends on the level number; the install has no music files
            || r.contains("play_music_track: No music file found")
    }
}

/// Result of one game run.
#[derive(Clone, Debug, Default)]
pub struct GameRun {
    pub lines: Vec<LogLine>,
    /// `Started level N from ...` was logged.
    pub started: bool,
    /// Game turn when the game was stopped (from the `packetfile` line).
    pub turn: Option<u32>,
    /// The game exited by itself before the timeout.
    pub exited_early: bool,
    /// A fatal signal other than our SIGTERM.
    pub crashed: bool,
    pub stdout: String,
}

impl GameRun {
    pub fn problems(&self) -> Vec<&LogLine> {
        self.lines
            .iter()
            .filter(|l| matches!(l.level, Level::Error | Level::Warning) && !l.is_noise())
            .collect()
    }
    /// Error / warning lines of `self` not present in `base` (multiset difference, noise removed).
    pub fn new_problems_vs(&self, base: &GameRun) -> Vec<String> {
        // compared without the "[player] " prefix, which varies between runs
        let key = |l: &LogLine| (l.level, l.func.clone(), l.msg.clone());
        let mut pool: Vec<_> = base.problems().iter().map(|l| key(l)).collect();
        let mut out = vec![];
        for l in self.problems() {
            match pool.iter().position(|p| *p == key(l)) {
                Some(i) => {
                    pool.swap_remove(i);
                }
                None => out.push(l.raw.clone()),
            }
        }
        out
    }
    pub fn summary(&self) -> String {
        format!(
            "started={} turn={:?} exited_early={} crashed={} problems={}",
            self.started,
            self.turn,
            self.exited_early,
            self.crashed,
            self.problems().len()
        )
    }
}

/// A private working copy of the install.
pub struct Rig {
    pub dir: PathBuf,
    pub install: PathBuf,
}

/// Top-level entries copied (not symlinked) or created fresh.
const PRIVATE: &[&str] = &[
    "keeperfx",
    "keeperfx.cfg",
    "keeperfx.log",
    "levels",
    "replays",
    "save",
    "scrshots",
];

fn io<T>(r: std::io::Result<T>, what: &dyn std::fmt::Display) -> T {
    r.unwrap_or_else(|e| panic!("{what}: {e}"))
}

impl Rig {
    pub fn new(install: &Path, name: &str) -> Rig {
        // `KFX_GAME_RIG_ROOT` puts the rigs elsewhere (the lock directory does not move)
        let root = std::env::var_os("KFX_GAME_RIG_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| kfx_formats::testutil::scratch_dir("kfx-map-gametest"));
        let dir = root.join(name);
        if dir.exists() {
            io(std::fs::remove_dir_all(&dir), &dir.display());
        }
        io(std::fs::create_dir_all(&dir), &dir.display());
        for e in io(std::fs::read_dir(install), &install.display()).flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if PRIVATE.iter().any(|p| p.eq_ignore_ascii_case(&n))
                || n.to_ascii_lowercase().starts_with("keeperfx_hvlog")
            {
                continue;
            }
            io(std::os::unix::fs::symlink(e.path(), dir.join(&n)), &n);
        }
        io(
            std::fs::copy(install.join("keeperfx"), dir.join("keeperfx")),
            &"copy keeperfx",
        );
        for d in ["replays", "save", "scrshots", "levels"] {
            io(std::fs::create_dir_all(dir.join(d)), &d);
        }
        let levels = kfx_formats::testutil::resolve_ci(install, "levels").expect("levels folder");
        for e in io(std::fs::read_dir(&levels), &levels.display()).flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.eq_ignore_ascii_case("personal") {
                continue;
            }
            io(
                std::os::unix::fs::symlink(e.path(), dir.join("levels").join(&n)),
                &n,
            );
        }
        io(
            std::fs::create_dir_all(dir.join("levels/personal")),
            &"personal",
        );
        let cfg = io(
            std::fs::read_to_string(install.join("keeperfx.cfg")),
            &"keeperfx.cfg",
        );
        let mut out = String::new();
        let mut seen = std::collections::HashSet::new();
        let set = [
            ("FRONTEND_RES", "640x480w32 640x480w32 640x480w32"),
            ("INGAME_RES", "640x480w32"),
            ("CAPTURE_CURSOR", "OFF"),
            ("CURSOR_EDGE_CAMERA_PANNING", "OFF"),
            ("ATMOSPHERIC_SOUNDS", "OFF"),
        ];
        for line in cfg.lines() {
            let key = line.split('=').next().unwrap_or("").trim();
            match set.iter().find(|(k, _)| *k == key) {
                Some((k, v)) => {
                    out.push_str(&format!("{k}={v}\n"));
                    seen.insert(*k);
                }
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        for (k, v) in set {
            if !seen.contains(k) {
                out.push_str(&format!("{k}={v}\n"));
            }
        }
        io(
            std::fs::write(dir.join("keeperfx.cfg"), out),
            &"keeperfx.cfg",
        );
        Rig {
            dir,
            install: install.to_path_buf(),
        }
    }

    /// `levels/personal` of the working copy.
    pub fn pack_dir(&self) -> PathBuf {
        self.dir.join("levels/personal")
    }

    /// Copy the files of level `src_level` in `src` into the pack as level `n` (lower-case
    /// names), replacing any previous level `n`. `script` replaces the `.txt` when given.
    pub fn install_level(
        &self,
        n: u32,
        src: &Path,
        src_level: u32,
        w: u16,
        h: u16,
        script: Option<&str>,
    ) {
        let pack = self.pack_dir();
        let prefix = format!("map{n:05}.");
        for e in io(std::fs::read_dir(&pack), &pack.display()).flatten() {
            if e.file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(&prefix)
            {
                io(std::fs::remove_file(e.path()), &e.path().display());
            }
        }
        let sprefix = format!("map{src_level:05}.");
        for e in io(std::fs::read_dir(src), &src.display()).flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(ext) = name.strip_prefix(&sprefix) else {
                continue;
            };
            if ext.contains('.') || ext == "lif" || ext == "lof" || !e.path().is_file() {
                continue; // per-map config overrides (mapNNNNN.rules.cfg ...) are not copied
            }
            io(
                std::fs::copy(e.path(), pack.join(format!("{prefix}{ext}"))),
                &e.path().display(),
            );
        }
        if let Some(s) = script {
            io(
                std::fs::write(pack.join(format!("{prefix}txt")), s),
                &"script",
            );
        }
        io(
            std::fs::write(
                pack.join(format!("{prefix}lif")),
                format!("{n}, Editor test {n}\n"),
            ),
            &"lif",
        );
        io(
            std::fs::write(
                pack.join(format!("{prefix}lof")),
                format!(
                    "; KeeperFX Level Overview File (LOF)\nNAME_TEXT = Editor test {n}\nKIND = \
                    FREE\nMAPSIZE = {w} {h}\n"
                ),
            ),
            &"lof",
        );
    }

    /// Write `map<n>.<ext>` into the pack.
    pub fn write_level_file(&self, n: u32, ext: &str, text: &str) {
        io(
            std::fs::write(self.pack_dir().join(format!("map{n:05}.{ext}")), text),
            &ext,
        );
    }

    /// Install the Lua state dump ([`dump_lua`]) as `map<n>.lua`.
    pub fn install_dump(&self, n: u32, w: u16, h: u16) {
        self.write_level_file(n, "lua", &dump_lua(w, h));
    }

    /// Run level `n` of the pack for `secs` seconds and return the parsed log.
    pub fn run(&self, n: u32, secs: u64) -> GameRun {
        let _g = RUN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _l = DirLock::acquire(Duration::from_secs(600));
        let log = self.dir.join("keeperfx.log");
        let _ = std::fs::remove_file(&log);
        let out = io(
            std::fs::File::create(self.dir.join("stdout.txt")),
            &"stdout.txt",
        );
        let err = io(out.try_clone(), &"stdout.txt");
        let child = Command::new("./keeperfx")
            .args([
                "-nointro",
                "-nosound",
                "-campaign",
                "personal",
                "-level",
                &n.to_string(),
            ])
            .current_dir(&self.dir)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("SDL_VIDEODRIVER", "offscreen")
            .env("SDL_AUDIODRIVER", "dummy")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn();
        let child = io(child, &"spawn keeperfx");
        let mut guard = ChildGuard(Some(child));
        let start = Instant::now();
        let mut exited_early = false;
        loop {
            let c = guard.0.as_mut().unwrap();
            if let Ok(Some(_)) = c.try_wait() {
                exited_early = true;
                break;
            }
            if start.elapsed() >= Duration::from_secs(secs) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !exited_early {
            guard.terminate();
        }
        drop(guard);
        let text = std::fs::read(&log)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let lines: Vec<LogLine> = text.lines().map(LogLine::parse).collect();
        let started = lines
            .iter()
            .any(|l| l.raw.starts_with(&format!("Started level {n} ")));
        let turn = lines.iter().find_map(|l| {
            let i = l.raw.find("\"turn\":")?;
            l.raw[i + 7..]
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        });
        let crashed = lines
            .iter()
            .any(|l| l.raw.contains("Failure signal") && !l.raw.contains("Termination"));
        let stdout = std::fs::read_to_string(self.dir.join("stdout.txt")).unwrap_or_default();
        GameRun {
            lines,
            started,
            turn,
            exited_early,
            crashed,
            stdout,
        }
    }
}

/// Kills the game when dropped (also on panic): SIGTERM, then SIGKILL after 5 s.
struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn terminate(&mut self) {
        let Some(mut c) = self.0.take() else { return };
        let _ = Command::new("kill")
            .args(["-TERM", &c.id().to_string()])
            .status();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = c.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = c.kill();
        let _ = c.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct DirLock;

impl DirLock {
    fn acquire(timeout: Duration) -> DirLock {
        let t = Instant::now();
        loop {
            match std::fs::create_dir(LOCK_DIR) {
                Ok(()) => {
                    let _ = std::fs::write(
                        Path::new(LOCK_DIR).join("pid"),
                        std::process::id().to_string(),
                    );
                    return DirLock;
                }
                Err(_) => {
                    let pid = std::fs::read_to_string(Path::new(LOCK_DIR).join("pid"))
                        .unwrap_or_default();
                    let alive =
                        !pid.trim().is_empty() && Path::new("/proc").join(pid.trim()).exists();
                    if !alive && t.elapsed() > Duration::from_secs(2) {
                        let _ = std::fs::remove_dir_all(LOCK_DIR);
                        continue;
                    }
                    assert!(t.elapsed() < timeout, "another game run holds {LOCK_DIR}");
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(LOCK_DIR);
    }
}

/// A level Lua script that, at game turn 2, prints the game's view of the map into the log:
/// every slab's kind and owner, every subtile's floor height (`GetFloorHeight`, in subtiles),
/// every object / effect generator / door / trap / creature. Lines start with `KFXDUMP`.
pub fn dump_lua(w: u16, h: u16) -> String {
    format!(
        r#"-- generated by the kfx-map game load test
local function kfx_dump()
  local W, H = {w}, {h}
  for y = 0, H - 1 do
    local row = {{}}
    for x = 0, W - 1 do
      local s = GetSlab(x, y)
      row[#row + 1] = tostring(s.kind) .. "/" .. tostring(s.owner.playerId)
    end
    print("KFXDUMP SLB " .. y .. " " .. table.concat(row, " "))
  end
  for y = 0, 3 * H - 1 do
    local row = {{}}
    for x = 0, 3 * W - 1 do
      row[#row + 1] = tostring(math.floor(GetFloorHeight(x, y) / 256))
    end
    print("KFXDUMP HGT " .. y .. " " .. table.concat(row, ""))
  end
  for _, cls in ipairs({{"Object", "EffectGen", "Door", "Trap", "Creature"}}) do
    local list = GetThingsOfClass(cls)
    print("KFXDUMP COUNT " .. cls .. " " .. #list)
    for _, t in ipairs(list) do
      print("KFXDUMP THING " .. cls .. " " .. tostring(t.model) .. " " .. tostring(t.owner.playerId) .. " " .. t.pos.stl_x .. " " .. t.pos.stl_y)
    end
  end
  print("KFXDUMP END")
end
RegisterTimerEvent(kfx_dump, 2, false)
"#
    )
}

/// The parsed `KFXDUMP` lines of a run.
#[derive(Clone, Debug, Default)]
pub struct Dump {
    /// Per slab (row-major): kind name and owner.
    pub slabs: Vec<(String, i64)>,
    /// Floor height (subtiles) per subtile row.
    pub heights: Vec<Vec<u8>>,
    pub counts: Vec<(String, usize)>,
    /// (class, model name, owner, stl x, stl y)
    pub things: Vec<(String, String, i64, i64, i64)>,
    pub complete: bool,
}

impl GameRun {
    pub fn dump(&self) -> Option<Dump> {
        let mut d = Dump::default();
        let mut any = false;
        for l in &self.lines {
            let Some(i) = l.raw.find("KFXDUMP ") else {
                continue;
            };
            any = true;
            let rest = &l.raw[i + 8..];
            let mut it = rest.split(' ');
            match it.next() {
                Some("SLB") => {
                    it.next();
                    for c in it {
                        let (k, o) = c.split_once('/').unwrap_or((c, "-1"));
                        d.slabs.push((k.to_string(), o.parse().unwrap_or(-1)));
                    }
                }
                Some("HGT") => {
                    it.next();
                    d.heights.push(
                        it.next()
                            .unwrap_or("")
                            .bytes()
                            .map(|b| b.wrapping_sub(b'0'))
                            .collect(),
                    );
                }
                Some("COUNT") => {
                    let c = it.next().unwrap_or("").to_string();
                    d.counts
                        .push((c, it.next().and_then(|v| v.parse().ok()).unwrap_or(0)));
                }
                Some("THING") => {
                    let v: Vec<&str> = it.collect();
                    if v.len() >= 5 {
                        let n = |s: &str| s.parse().unwrap_or(-1);
                        d.things.push((
                            v[0].to_string(),
                            v[1].to_string(),
                            n(v[2]),
                            n(v[3]),
                            n(v[4]),
                        ));
                    }
                }
                Some("END") => d.complete = true,
                _ => {}
            }
        }
        any.then_some(d)
    }
}
