// Shared by several test binaries; each uses a subset.
#![allow(dead_code)]
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A unique scratch folder under the test scratch root ([`kfx_formats::testutil::scratch_dir`]),
/// removed again when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Scratch {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = kfx_formats::testutil::scratch_dir("workshop")
            .join(format!("{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in files {
        if n.ends_with('/') {
            z.add_directory(*n, o).unwrap();
        } else {
            z.start_file(*n, o).unwrap();
            z.write_all(b).unwrap();
        }
    }
    z.finish().unwrap();
}

/// A fake level: all the files a level has, with distinct content per level number.
pub fn level_files(n: u32, with_lof: bool) -> Vec<(String, Vec<u8>)> {
    let mut v = vec![];
    for ext in [
        "slb", "own", "clm", "dat", "tng", "apt", "lgt", "wib", "inf", "txt",
    ] {
        v.push((
            format!("map{n:05}.{ext}"),
            format!("{ext} of level {n}").into_bytes(),
        ));
    }
    if with_lof {
        v.push((
            format!("map{n:05}.lof"),
            format!(
                "; KeeperFX Level Overview File (LOF)\nKIND = FREE\n\
                    NAME_TEXT = Level {n}\nPLAYERS = 1\n"
            )
            .into_bytes(),
        ));
    } else {
        v.push((
            format!("map{n:05}.lif"),
            format!("{n}, Level {n}\n").into_bytes(),
        ));
    }
    v
}

/// Every file and folder under `root`: relative path -> content (None for a folder).
pub fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().display().to_string();
            if p.is_dir() {
                out.insert(rel, None);
                walk(root, &p, out);
            } else {
                out.insert(rel, Some(std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(root, root, &mut m);
    m
}

/// A private install layout: the pack/campaign configs of the test install (`KFX_INSTALL`) when
/// there is one (no level data, no copyrighted media), else a hand-made equivalent.
pub fn private_install(root: &Path) {
    let src = kfx_formats::testutil::install_dir();
    let mk = |p: &str| std::fs::create_dir_all(root.join(p)).unwrap();
    for d in [
        "levels",
        "levels/personal",
        "levels/standard",
        "campgns",
        "campgns/keeporig",
        "multiplayer",
        "multiplayer/modern",
        "mods",
        "fxdata",
        "creatrs",
    ] {
        mk(d);
    }
    for f in [
        "levels/personal.cfg",
        "levels/standard.cfg",
        "campgns/keeporig.cfg",
        "multiplayer/modern.cfg",
        "mods/_load_order.cfg",
    ] {
        let bytes = src.as_ref().and_then(|s| std::fs::read(s.join(f)).ok());
        let bytes = bytes.unwrap_or_else(|| match f {
            "levels/personal.cfg" => b"; KeeperFX Mappack file\n\n[common]\nNAME = Personal \
                levels\nNAME_TEXT_ID = 974\nLEVELS_LOCATION = levels/personal\nHIGH_SCORES = \
                9999 scr_lgcy.dat\nHUMAN_PLAYER = RED\n\n[strings]\nENG = fxdata/gtext_eng.dat\n"
                .to_vec(),
            "levels/standard.cfg" => b"[common]\nNAME = Standard levels\nLEVELS_LOCATION = \
                levels/standard\n"
                .to_vec(),
            "campgns/keeporig.cfg" => b"[common]\nNAME = Original\nLEVELS_LOCATION = \
                campgns/keeporig\n"
                .to_vec(),
            "multiplayer/modern.cfg" => b"[common]\nNAME = Modern\nLEVELS_LOCATION = \
                multiplayer/modern\n"
                .to_vec(),
            _ => b"[after_base]\n\n[after_campaign]\n\n[after_map]\n".to_vec(),
        });
        std::fs::write(root.join(f), bytes).unwrap();
    }
}
