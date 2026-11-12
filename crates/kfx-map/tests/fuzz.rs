//! Mutation fuzzing (bounded, deterministic): mutated real files through every parser and
//! through load -> validate -> save -> legacy save -> transform, asserting no panic. Also:
//! whatever the FX writers produce parses again, is stable, and is accepted by the game's TOML
//! reader (`kfx_formats::game_toml`); a map the editor saved loads again without unreadable parts.
//!
//! `KFX_FUZZ_ITERS` sets the iterations per target (default 400), `KFX_FUZZ_SEED` the seed,
//! `KFX_FUZZ_SECS` the time budget per test (default 60 s; the loops stop early when it is spent).
//! Needs the test install (skipped otherwise).
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{campaign, cfg, game_toml, level_fx, levelinfo, rnc, strings, testutil};
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn env_num(k: &str, d: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}

fn iters() -> usize {
    env_num("KFX_FUZZ_ITERS", 400) as usize
}

fn rng(salt: u64) -> Rng {
    Rng(
        env_num("KFX_FUZZ_SEED", 0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0x2545_F491_4F6C_DD1D)
            | 1,
    )
}

struct Budget(Instant, Duration);
impl Budget {
    fn new() -> Self {
        Budget(
            Instant::now(),
            Duration::from_secs(env_num("KFX_FUZZ_SECS", 60)),
        )
    }
    fn spent(&self) -> bool {
        self.0.elapsed() > self.1
    }
}

fn mutate(r: &mut Rng, b: &[u8], text: bool) -> Vec<u8> {
    let mut v = b.to_vec();
    for _ in 0..1 + r.below(4) {
        match r.below(if text { 9 } else { 6 }) {
            0 if !v.is_empty() => {
                let i = r.below(v.len());
                v[i] ^= 1 << r.below(8);
            }
            1 if !v.is_empty() => {
                let i = r.below(v.len());
                v[i] = r.next() as u8;
            }
            2 => {
                let n = r.below(v.len() + 1);
                v.truncate(n);
            }
            3 => {
                let i = r.below(v.len() + 1);
                for _ in 0..r.below(8) {
                    v.insert(i, r.next() as u8);
                }
            }
            4 if v.len() >= 4 => {
                // a big count / size word
                let i = r.below(v.len() - 3);
                v[i..i + 4].copy_from_slice(&[0xFF, 0xFF, 0xFF, r.next() as u8 & 0x7F]);
            }
            5 if !v.is_empty() => {
                let i = r.below(v.len());
                let j = r.below(v.len());
                let n = r.below(64).min(v.len() - i.max(j));
                let s: Vec<u8> = v[i..i + n].to_vec();
                v[j..j + n].copy_from_slice(&s);
            }
            6 => {
                // interesting tokens
                let toks: [&[u8]; 20] = [
                    b"-1",
                    b"99999999999",
                    b"[",
                    b"]",
                    b"\"",
                    b"'",
                    b"\n",
                    b"=",
                    b"[thing0]",
                    b"\\",
                    b"\xff\xfe",
                    b"-2147483648",
                    b"[[thing]]",
                    b"9223372036854775807",
                    b"{",
                    b".",
                    b"\"\"\"",
                    b"'''",
                    b"\"Foo Bar\" = 1\n",
                    b"\n[foo]\n[foo.bar.baz]\nx = 1\n",
                ];
                let t = toks[r.below(toks.len())];
                let i = r.below(v.len() + 1);
                v.splice(i..i, t.iter().copied());
            }
            7 => {
                // a number replaced by an extreme
                let ext: [&[u8]; 7] = [
                    b"-1",
                    b"0",
                    b"65536",
                    b"-9223372036854775808",
                    b"2147483648",
                    b"1000000",
                    b"-0",
                ];
                if let Some(i) = (0..v.len())
                    .map(|_| r.below(v.len().max(1)))
                    .find(|&i| v.get(i).is_some_and(|c| c.is_ascii_digit()))
                {
                    let mut j = i;
                    while j < v.len() && v[j].is_ascii_digit() {
                        j += 1;
                    }
                    let e = ext[r.below(ext.len())];
                    v.splice(i..j, e.iter().copied());
                }
            }
            _ => {
                // a line duplicated
                let s = String::from_utf8_lossy(&v).to_string();
                let lines: Vec<&str> = s.lines().collect();
                if !lines.is_empty() {
                    let l = format!("{}\n", lines[r.below(lines.len())]);
                    let i = r.below(v.len() + 1);
                    v.splice(i..i, l.bytes());
                }
            }
        }
    }
    v
}

fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Collected failures; the input of the first one per kind is saved for a reproduction.
struct Failures(Vec<String>, PathBuf);
impl Failures {
    fn new(name: &str) -> Self {
        Failures(
            vec![],
            kfx_formats::testutil::scratch_dir(&format!(
                "kfx-map/kfx-fuzz-{name}-{}",
                std::process::id()
            )),
        )
    }
    fn add(&mut self, what: &str, data: &[u8], msg: String) {
        let first = msg.lines().next().unwrap_or("").to_string();
        if self
            .0
            .iter()
            .any(|f| f.starts_with(&format!("{what}: {first}")))
        {
            return;
        }
        let _ = std::fs::create_dir_all(&self.1);
        let p = self.1.join(format!("{what}-{}.bin", self.0.len()));
        let _ = std::fs::write(&p, data);
        self.0
            .push(format!("{what}: {first} (input {})", p.display()));
    }
    fn check(self) {
        assert!(
            self.0.is_empty(),
            "{} fuzz failures:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn quiet<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    catch_unwind(AssertUnwindSafe(f))
}

/// The binary level files, RNC and the FX / cfg-like / string parsers.
#[test]
fn parsers_survive_mutations() {
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let budget = Budget::new();
    let n = iters();
    let mut fail = Failures::new("parsers");
    let mut r = rng(1);
    let d85 = MapDims::new(85, 85);
    let keep = install.join("campgns/keeporig");
    let dir =
        kfx_formats::testutil::scratch_dir(&format!("kfx-map/kfx-fuzz-bin-{}", std::process::id()));
    for ext in level_bin::EXTENSIONS {
        let Some(p) =
            level_bin::find_file(&keep, 6, ext).or_else(|| level_bin::find_file(&keep, 1, ext))
        else {
            continue;
        };
        let raw = std::fs::read(&p).unwrap();
        let data = if rnc::is_rnc(&raw) {
            rnc::unpack(&raw).unwrap()
        } else {
            raw.clone()
        };
        for _ in 0..n / 4 {
            if budget.spent() {
                break;
            }
            let m = mutate(&mut r, &data, false);
            let res = quiet(|| {
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("map00001.{ext}")), &m).unwrap();
                let _ = level_bin::LevelBin::load_with_dims(&dir, 1, Some(d85));
            });
            if let Err(e) = res {
                fail.add(ext, &m, panic_msg(e));
            }
        }
        if rnc::is_rnc(&raw) {
            // the packed body mutated, with the packed CRC fixed so the decoder runs
            for _ in 0..n {
                if budget.spent() {
                    break;
                }
                let mut m = raw.clone();
                let body = m.len() - 18;
                for _ in 0..1 + r.below(3) {
                    let i = 18 + r.below(body);
                    m[i] ^= 1 << r.below(8);
                }
                let packed = u32::from_be_bytes(m[8..12].try_into().unwrap()) as usize;
                if 18 + packed <= m.len() {
                    let c = rnc::crc(&m[18..18 + packed]);
                    m[14..16].copy_from_slice(&c.to_be_bytes());
                }
                if let Err(e) = quiet(|| rnc::unpack(&m)) {
                    fail.add("rnc", &m, panic_msg(e));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    // FX text: no panic; what is written parses again, is stable and loads in the game
    let camp = install.join("campgns/burdnimp");
    for ext in ["tngfx", "aptfx", "lgtfx"] {
        let data = std::fs::read(level_bin::find_file(&camp, 220, ext).unwrap()).unwrap();
        for _ in 0..n {
            if budget.spent() {
                break;
            }
            let m = mutate(&mut r, &data, true);
            let res = quiet(|| match ext {
                "tngfx" => level_fx::TngFx::from_bytes(&m)
                    .ok()
                    .map(|f| (f.to_text(), f.to_bytes())),
                "aptfx" => level_fx::AptFx::from_bytes(&m)
                    .ok()
                    .map(|f| (f.to_text(), f.to_bytes())),
                _ => level_fx::LgtFx::from_bytes(&m)
                    .ok()
                    .map(|f| (f.to_text(), f.to_bytes())),
            });
            match res {
                Err(e) => fail.add(ext, &m, panic_msg(e)),
                Ok(Some((t, bytes))) => {
                    if let Err(e) = game_toml::check(&t) {
                        fail.add(
                            &format!("{ext}-game-rejects"),
                            &m,
                            format!("our output is rejected by the game's reader: {e}"),
                        );
                    }
                    let again = quiet(|| match ext {
                        "tngfx" => level_fx::TngFx::from_bytes(&bytes).map(|f| f.to_text()),
                        "aptfx" => level_fx::AptFx::from_bytes(&bytes).map(|f| f.to_text()),
                        _ => level_fx::LgtFx::from_bytes(&bytes).map(|f| f.to_text()),
                    });
                    match again {
                        Err(e) => fail.add(&format!("{ext}-reparse-panic"), &m, panic_msg(e)),
                        Ok(Err(e)) => fail.add(
                            &format!("{ext}-reparse"),
                            &m,
                            format!("our output does not parse: {e}"),
                        ),
                        Ok(Ok(t2)) if t2 != t => fail.add(
                            &format!("{ext}-unstable"),
                            &m,
                            "the second write differs".into(),
                        ),
                        _ => {}
                    }
                }
                Ok(None) => {}
            }
        }
    }
    // cfg-like text formats and string tables: no panic
    let lof = std::fs::read(level_bin::find_file(&camp, 220, "lof").unwrap()).unwrap();
    let ccfg = std::fs::read(install.join("campgns/keeporig.cfg")).unwrap_or_default();
    let objcfg = std::fs::read(install.join("fxdata/objects.cfg")).unwrap_or_default();
    let lif = std::fs::read_dir(install.join("levels/classic"))
        .unwrap()
        .flatten()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".lif")
        })
        .map(|e| std::fs::read(e.path()).unwrap())
        .unwrap_or_default();
    for (what, data) in [
        ("lof", &lof),
        ("campaign", &ccfg),
        ("cfg", &objcfg),
        ("lif", &lif),
    ] {
        for _ in 0..n {
            if budget.spent() {
                break;
            }
            let m = mutate(&mut r, data, true);
            let res = quiet(|| match what {
                "lof" => {
                    let l = levelinfo::Lof::parse(&m);
                    let _ = l.info().mapsize_or_default();
                    let _ = l.to_bytes();
                }
                "lif" => {
                    let _ = levelinfo::Lif::parse(&m).to_bytes();
                }
                "campaign" => {
                    let c = campaign::Campaign::parse(&m);
                    let _ = c.common();
                    let _ = c.map_block_numbers();
                    let _ = c.to_bytes();
                }
                _ => {
                    let _ = cfg::CfgDoc::from_bytes(&m).to_bytes();
                }
            });
            if let Err(e) = res {
                fail.add(what, &m, panic_msg(e));
            }
        }
    }
    if let Some(p) = testutil::resolve_ci(&install, "fxdata/gtext_eng.dat") {
        let data = std::fs::read(p).unwrap();
        for _ in 0..n / 4 {
            if budget.spent() {
                break;
            }
            let m = mutate(&mut r, &data, false);
            if let Err(e) = quiet(|| {
                strings::StringTable::parse(&m, strings::Lang::from_code("eng").unwrap()).to_bytes()
            }) {
                fail.add("strings", &m, panic_msg(e));
            }
        }
    }
    fail.check();
}

fn copy_lower(src: &[PathBuf], dst: &Path) {
    let _ = std::fs::remove_dir_all(dst);
    std::fs::create_dir_all(dst).unwrap();
    for p in src {
        std::fs::copy(
            p,
            dst.join(
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            ),
        )
        .unwrap();
    }
}

/// One file of a whole map mutated: load, validate, save, legacy save, slab objects,
/// transform. No panic; a map the editor saved loads again and is readable.
#[test]
fn map_pipeline_survives_mutations() {
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let gd = GameData::load(&ConfigLayers::for_level(&install, None, None, None, None)).unwrap();
    let budget = Budget::new();
    let mut fail = Failures::new("map");
    let mut r = rng(2);
    let dims = MapDims::new(85, 85);
    for (rel, n) in [("multiplayer/classic", 55u32), ("campgns/keeporig", 6)] {
        let src = install.join(rel);
        let names: Vec<PathBuf> = std::fs::read_dir(&src)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with(&format!("map{n:05}."))
            })
            .collect();
        let dir = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-map/kfx-fuzz-map-{}",
            std::process::id()
        ));
        let out = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-map/kfx-fuzz-out-{}",
            std::process::id()
        ));
        for it in 0..iters() / 2 {
            if budget.spent() {
                break;
            }
            copy_lower(&names, &dir);
            let victim = &names[r.below(names.len())];
            let vname = victim
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            let text = vname.ends_with("fx") || vname.ends_with("lof") || vname.ends_with("toml");
            let m = mutate(&mut r, &std::fs::read(victim).unwrap(), text);
            std::fs::write(dir.join(&vname), &m).unwrap();
            let res = quiet(|| -> Option<String> {
                let mut d = MapDoc::load(&dir, n, dims, &gd).ok()?;
                let _ = kfx_map::validate::validate(&d, &gd);
                let _ = std::fs::remove_dir_all(&out);
                let saved = d.save(&gd, &out, SaveColumns::Current).is_ok();
                if saved {
                    match MapDoc::load(&out, n, dims, &gd) {
                        Err(e) => return Some(format!("the saved map does not load: {e}")),
                        Ok(b) if !b.unreadable.is_empty() => {
                            return Some(format!(
                                "the saved map has unreadable parts: {:?}",
                                b.unreadable.iter().map(|u| &u.error).collect::<Vec<_>>()
                            ))
                        }
                        Ok(_) => {}
                    }
                }
                if d.read_only_reason().is_some() && saved {
                    return Some("a read-only map was saved".into());
                }
                let _ = d.save_legacy(&gd, &out, SaveColumns::Current);
                let all = kfx_map::autotile::SlabRect {
                    x0: 0,
                    y0: 0,
                    x1: 84,
                    y1: 84,
                };
                let _ = d.apply_slab_objects(&gd, all);
                if it % 8 == 0 {
                    let _ = d.transform_map(&gd, kfx_map::transform::Transform::Rot90);
                }
                None
            });
            match res {
                Err(e) => fail.add(&format!("mapdoc-{vname}"), &m, panic_msg(e)),
                Ok(Some(msg)) => fail.add(&format!("mapdoc-{vname}"), &m, msg),
                Ok(None) => {}
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out);
    }
    fail.check();
}
