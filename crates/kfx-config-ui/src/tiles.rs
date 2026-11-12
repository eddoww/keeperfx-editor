//! Block texture sets (`tmapaNNN.dat` / `tmapbNNN.dat`): PNG export, PNG import with palette
//! quantisation (optional Floyd-Steinberg dithering), DAT import, per-tile paste and export,
//! texture packs (a `filelist_tmapXNNN.txt` and its PNGs, the layout of KeeperFX's own texture
//! sources, `pkg_gfx.mk:241`), where an override is written and what lies below it (revert).
//!
//! A tmap file is 256 x 2176 raw palette indices (`kfx_gfx::textures`). Overrides are looked
//! up by the game as `LEVELS_LOCATION/mapNNNNN.tmapXNNN.dat`, then the campaign's
//! `CONFIGS_LOCATION/tmapXNNN.dat`, then `data/` (`prepare_letter_one_file_path`,
//! src/engine_textures.c:155-193; see `kfx_gfx::textures::TextureSearch`).
use crate::fs::{find_ci, ConfigFs};
use crate::layers::{Layer, LayerKind, LayerStack};
use kfx_gfx::textures::{
    Part, TextureSet, ATLAS_H, ATLAS_W, FILE_BYTES, TILES_PER_FILE, TILES_PER_ROW, TILE_DIM,
};
use kfx_gfx::{Palette, Rgba};
use std::path::{Component, Path, PathBuf};

/// Nearest palette colour search with a small cache.
pub struct Quantizer {
    table: [[u8; 4]; 256],
    cache: std::collections::HashMap<[u8; 3], u8>,
    /// Indices that may be used (all by default).
    pub allowed: Vec<u8>,
}

impl Quantizer {
    pub fn new(pal: &Palette) -> Self {
        Quantizer {
            table: pal.table(),
            cache: Default::default(),
            allowed: (0..=255).collect(),
        }
    }
    /// Index of the palette colour closest to `rgb` (squared RGB distance, weighted for the eye).
    pub fn nearest(&mut self, rgb: [u8; 3]) -> u8 {
        if let Some(&i) = self.cache.get(&rgb) {
            return i;
        }
        let mut best = (u32::MAX, 0u8);
        for &i in &self.allowed {
            let c = self.table[i as usize];
            let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2) as u32;
            let dist = 2 * d(c[0], rgb[0]) + 4 * d(c[1], rgb[1]) + 3 * d(c[2], rgb[2]);
            if dist < best.0 {
                best = (dist, i);
            }
        }
        self.cache.insert(rgb, best.1);
        best.1
    }
    pub fn color(&self, i: u8) -> [u8; 4] {
        self.table[i as usize]
    }

    /// Quantise an image to palette indices (row-major), dithering when asked.
    pub fn quantize(&mut self, img: &Rgba, dither: bool) -> Vec<u8> {
        let (w, h) = (img.w as usize, img.h as usize);
        let mut out = vec![0u8; w * h];
        if !dither {
            for y in 0..h {
                for x in 0..w {
                    let p = img.pixel(x as u32, y as u32);
                    out[y * w + x] = self.nearest([p[0], p[1], p[2]]);
                }
            }
            return out;
        }
        let mut buf: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let p = img.pixel((i % w) as u32, (i / w) as u32);
                [p[0] as f32, p[1] as f32, p[2] as f32]
            })
            .collect();
        for y in 0..h {
            for x in 0..w {
                let o = buf[y * w + x];
                let q = [
                    o[0].clamp(0.0, 255.0) as u8,
                    o[1].clamp(0.0, 255.0) as u8,
                    o[2].clamp(0.0, 255.0) as u8,
                ];
                let i = self.nearest(q);
                out[y * w + x] = i;
                let c = self.table[i as usize];
                let e = [o[0] - c[0] as f32, o[1] - c[1] as f32, o[2] - c[2] as f32];
                let mut spread = |dx: isize, dy: isize, f: f32| {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        let p = &mut buf[ny as usize * w + nx as usize];
                        for k in 0..3 {
                            p[k] += e[k] * f;
                        }
                    }
                };
                spread(1, 0, 7.0 / 16.0);
                spread(-1, 1, 3.0 / 16.0);
                spread(0, 1, 5.0 / 16.0);
                spread(1, 1, 1.0 / 16.0);
            }
        }
        out
    }
}

/// Indexed pixels to RGBA.
pub fn indexed_to_rgba(idx: &[u8], w: u32, h: u32, pal: &Palette) -> Rgba {
    let mut img = Rgba::new(w, h);
    for (i, &p) in idx.iter().enumerate().take((w * h) as usize) {
        img.put(i as u32 % w, i as u32 / w, pal.rgba(p));
    }
    img
}

/// A whole tmap file (256 x 2176) as PNG.
pub fn atlas_png(set: &TextureSet, part: Part, pal: &Palette) -> Result<Vec<u8>, String> {
    set.atlas_rgba(part, pal)
        .to_png()
        .map_err(|e| e.to_string())
}

/// Resize with nearest neighbour (for pasting images of other sizes onto a tile).
pub fn resize_nearest(img: &Rgba, w: u32, h: u32) -> Rgba {
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.put(x, y, img.pixel(x * img.w / w, y * img.h / h));
        }
    }
    out
}

/// A tmap file's bytes from a 256 x 2176 PNG (other sizes are an error).
pub fn import_atlas_png(png: &[u8], pal: &Palette, dither: bool) -> Result<Vec<u8>, String> {
    let img = Rgba::from_png(png).map_err(|e| e.to_string())?;
    if img.w as usize != ATLAS_W || img.h as usize != ATLAS_H {
        return Err(format!(
            "the image is {}x{}, a texture file is {ATLAS_W}x{ATLAS_H}",
            img.w, img.h
        ));
    }
    let mut q = Quantizer::new(pal);
    let v = q.quantize(&img, dither);
    debug_assert_eq!(v.len(), FILE_BYTES);
    Ok(v)
}

/// Paste an image (resized to 32x32 when needed) onto tile `tile` (0..544) of a tmap file.
pub fn paste_tile(
    atlas: &mut [u8],
    tile: usize,
    img: &Rgba,
    q: &mut Quantizer,
    dither: bool,
) -> Result<(), String> {
    if tile >= TILES_PER_FILE || atlas.len() < FILE_BYTES {
        return Err("tile out of range".into());
    }
    let img = if img.w as usize == TILE_DIM && img.h as usize == TILE_DIM {
        img.clone()
    } else {
        resize_nearest(img, TILE_DIM as u32, TILE_DIM as u32)
    };
    let idx = q.quantize(&img, dither);
    let (col, row) = (tile % TILES_PER_ROW, tile / TILES_PER_ROW);
    for y in 0..TILE_DIM {
        let o = (row * TILE_DIM + y) * ATLAS_W + col * TILE_DIM;
        atlas[o..o + TILE_DIM].copy_from_slice(&idx[y * TILE_DIM..(y + 1) * TILE_DIM]);
    }
    Ok(())
}

/// Where layer `layer` keeps texture file `part` of set `index`: `mapNNNNN.tmapXNNN.dat` in the
/// levels folder for a map, `tmapXNNN.dat` in `CONFIGS_LOCATION` for a campaign, the mod's
/// `data/` for a mod. `None` for the base (the install's `data/` is not written) or a layer
/// without folder.
pub fn tmap_override_path(layer: &Layer, part: Part, index: u32) -> Option<PathBuf> {
    let letter = if part == Part::A { 'a' } else { 'b' };
    let name = format!("tmap{letter}{index:03}.dat");
    match &layer.kind {
        LayerKind::Base => None,
        LayerKind::Map(n) => Some(layer.dir.as_ref()?.join(format!("map{n:05}.{name}"))),
        LayerKind::Campaign => Some(layer.dir.as_ref()?.join(name)),
        LayerKind::Mod(_) => Some(layer.dir.as_ref()?.parent()?.join("data").join(name)),
    }
}

/// A tmap file's bytes from a raw `.dat` (the game's own check: at least 1024 bytes; shorter
/// files leave the rest at `MISSING_FILL`, longer ones are cut, `TextureSet::from_bytes`).
pub fn import_atlas_dat(dat: &[u8]) -> Result<Vec<u8>, String> {
    let set = TextureSet::from_bytes(0, dat, None).map_err(|e| e.to_string())?;
    Ok(set.atlas_indexed(Part::A).to_vec())
}

/// One tile (0..544) of a tmap file as RGBA.
pub fn tile_rgba(atlas: &[u8], tile: usize, pal: &Palette) -> Option<Rgba> {
    if tile >= TILES_PER_FILE || atlas.len() < FILE_BYTES {
        return None;
    }
    let (col, row) = (tile % TILES_PER_ROW, tile / TILES_PER_ROW);
    let mut idx = Vec::with_capacity(TILE_DIM * TILE_DIM);
    for y in 0..TILE_DIM {
        let o = (row * TILE_DIM + y) * ATLAS_W + col * TILE_DIM;
        idx.extend_from_slice(&atlas[o..o + TILE_DIM]);
    }
    Some(indexed_to_rgba(&idx, TILE_DIM as u32, TILE_DIM as u32, pal))
}

fn letter(part: Part) -> char {
    if part == Part::A {
        'a'
    } else {
        'b'
    }
}

/// `tmapXNNN.dat`.
pub fn tmap_file_name(part: Part, index: u32) -> String {
    format!("tmap{}{index:03}.dat", letter(part))
}

/// The folder and file-name prefix where layer `l` keeps texture files: the install's `data/`
/// (next to `fxdata`) for the base, `<mod>/data` for a mod, `CONFIGS_LOCATION` for a
/// campaign, the levels folder with `mapNNNNN.` for a map (src/engine_textures.c:155-193).
fn tmap_dir(fs: &dyn ConfigFs, l: &Layer) -> Option<(PathBuf, String)> {
    match &l.kind {
        LayerKind::Base => {
            let root = l.dir.as_ref()?.parent()?;
            Some((
                find_ci(fs, root, "data").unwrap_or_else(|| root.join("data")),
                String::new(),
            ))
        }
        _ => {
            let p = tmap_override_path(l, Part::A, 0)?;
            let prefix = match l.kind {
                LayerKind::Map(n) => format!("map{n:05}."),
                _ => String::new(),
            };
            Some((p.parent()?.to_path_buf(), prefix))
        }
    }
}

/// Layer `l`'s file for `part` of set `index`, if it exists.
pub fn layer_tmap(fs: &dyn ConfigFs, l: &Layer, part: Part, index: u32) -> Option<PathBuf> {
    let (d, prefix) = tmap_dir(fs, l)?;
    find_ci(fs, &d, &format!("{prefix}{}", tmap_file_name(part, index)))
}

/// The nearest layer below `below` (exclusive) that has texture file `part` of set `index`,
/// and the file. With `below` = the layer count this is the file the game loads (the search
/// walks map, mods, campaign, mods, `data/` — the reverse of the layer order,
/// `prepare_letter_one_file_path`, src/engine_textures.c:155-193); with `below` = the target it is
/// what applies once the target's override is removed.
pub fn lower_tmap(
    fs: &dyn ConfigFs,
    layers: &LayerStack,
    below: usize,
    part: Part,
    index: u32,
) -> Option<(usize, PathBuf)> {
    (0..below.min(layers.layers.len()))
        .rev()
        .find_map(|i| Some((i, layer_tmap(fs, &layers.layers[i], part, index)?)))
}

/// `tmapXNNN.dat` texture set numbers some layer has a tmapa file for, sorted.
pub fn available_sets(fs: &dyn ConfigFs, layers: &LayerStack) -> Vec<u32> {
    let mut out = vec![];
    for l in &layers.layers {
        let Some((d, prefix)) = tmap_dir(fs, l) else {
            continue;
        };
        let head = format!("{prefix}tmapa");
        for f in fs.list_dir(&d).unwrap_or_default() {
            let lf = f.to_ascii_lowercase();
            let Some(num) = lf.strip_prefix(&head).and_then(|r| r.strip_suffix(".dat")) else {
                continue;
            };
            if num.len() == 3 && num.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(n) = num.parse::<u32>() {
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Does the layer hold any texture file of its own (any set, tmapa or tmapb)?
pub fn layer_has_tmaps(fs: &dyn ConfigFs, l: &Layer) -> bool {
    let Some((d, prefix)) = tmap_dir(fs, l) else {
        return false;
    };
    fs.list_dir(&d).unwrap_or_default().iter().any(|f| {
        let lf = f.to_ascii_lowercase();
        lf.strip_prefix(&prefix).is_some_and(|r| {
            (r.starts_with("tmapa") || r.starts_with("tmapb")) && r.ends_with(".dat")
        })
    })
}

/// Where a texture import goes when `target` is the target layer: the target itself, except for
/// a map target when the map has no file of its own for this part and the campaign layer already
/// holds texture files: then the campaign, so every level of the campaign shares the import.
/// `map_local` turns that rule off. Returns the layer index and whether the rule applied.
pub fn tmap_write_layer(
    fs: &dyn ConfigFs,
    layers: &LayerStack,
    target: usize,
    part: Part,
    index: u32,
    map_local: bool,
) -> (usize, bool) {
    let Some(t) = layers.layers.get(target) else {
        return (target, false);
    };
    if map_local || !matches!(t.kind, LayerKind::Map(_)) || layer_tmap(fs, t, part, index).is_some()
    {
        return (target, false);
    }
    match layers.index_of(&LayerKind::Campaign) {
        Some(c)
            if layer_has_tmaps(fs, &layers.layers[c])
                && tmap_override_path(&layers.layers[c], part, index).is_some() =>
        {
            (c, true)
        }
        _ => (target, false),
    }
}

/// The texture set the game loads for set `index` (all layers), with the source of each part
/// (`None` for a missing tmapb).
pub fn load_effective_set(
    fs: &dyn ConfigFs,
    layers: &LayerStack,
    index: u32,
) -> Result<(TextureSet, [Option<(usize, PathBuf)>; 2]), String> {
    let n = layers.layers.len();
    let src = [Part::A, Part::B].map(|p| lower_tmap(fs, layers, n, p, index));
    let read = |s: &Option<(usize, PathBuf)>| -> Result<Option<Vec<u8>>, String> {
        match s {
            Some((_, p)) => {
                crate::fs::read_unpacked(fs, p).map_err(|e| format!("{}: {e}", p.display()))
            }
            None => Ok(None),
        }
    };
    let a = read(&src[0])?
        .ok_or_else(|| format!("no {} in any layer", tmap_file_name(Part::A, index)))?;
    let b = read(&src[1])?;
    let set = TextureSet::from_bytes(index, &a, b.as_deref()).map_err(|e| e.to_string())?;
    Ok((set, src))
}

/// One line of a texture pack filelist: tile `n` (the line's order) is the `w` x `h` rectangle
/// at `x`, `y` of PNG `file` (a path relative to the filelist's folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackEntry {
    pub file: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// `filelist_tmapXNNN.txt`.
pub fn filelist_name(part: Part, index: u32) -> String {
    format!("filelist_tmap{}{index:03}.txt", letter(part))
}

/// A filelist with one 32x32 PNG per tile: `packNNN/tmapX/tileTTT.png`. The header names the
/// pack and its grid (`textures_pack_NNN`, 8 columns, 68 rows, 32x32 tiles).
pub fn default_filelist(part: Part, index: u32) -> String {
    let mut s = format!(
        "textures_pack_{index:03}\t{TILES_PER_ROW}\t{}\t{TILE_DIM}\t{TILE_DIM}\n",
        TILES_PER_FILE / TILES_PER_ROW
    );
    for t in 0..TILES_PER_FILE {
        s.push_str(&format!(
            "pack{index:03}/tmap{}/tile{t:03}.png\t0\t0\t{TILE_DIM}\t{TILE_DIM}\n",
            letter(part)
        ));
    }
    s
}

/// A relative path that stays inside its folder.
fn safe_relative(p: &str) -> bool {
    !p.is_empty()
        && !p.contains('\\')
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Parse a filelist (header `textures_pack_...`, then one tab-separated line per tile).
pub fn parse_filelist(text: &str) -> Result<Vec<PackEntry>, String> {
    let mut lines = text
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty());
    let head = lines.next().ok_or("empty filelist")?;
    if !head.trim_start().starts_with("textures_pack_") {
        return Err("not a texture pack filelist (no textures_pack_ header)".into());
    }
    let mut out = vec![];
    for (n, l) in lines.enumerate() {
        let f: Vec<&str> = l
            .split('\t')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .collect();
        if f.len() < 5 {
            return Err(format!("line {}: expected file x y w h", n + 2));
        }
        let num = |s: &str| {
            s.parse::<u32>()
                .map_err(|_| format!("line {}: {s} is not a number", n + 2))
        };
        let e = PackEntry {
            file: f[0].to_string(),
            x: num(f[1])?,
            y: num(f[2])?,
            w: num(f[3])?,
            h: num(f[4])?,
        };
        if !safe_relative(&e.file) {
            return Err(format!(
                "line {}: {} must be a path inside the pack folder",
                n + 2,
                e.file
            ));
        }
        if e.w == 0 || e.h == 0 || e.w as usize > TILE_DIM || e.h as usize > TILE_DIM {
            return Err(format!("line {}: tile size must be 1..32", n + 2));
        }
        out.push(e);
    }
    if out.len() > TILES_PER_FILE {
        return Err(format!(
            "{} tiles, a texture file has {TILES_PER_FILE}",
            out.len()
        ));
    }
    Ok(out)
}

fn read_text(fs: &dyn ConfigFs, p: &Path) -> Result<Option<String>, String> {
    Ok(fs
        .read(p)
        .map_err(|e| format!("{}: {e}", p.display()))?
        .map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// Export part `part` of a texture set as a pack in `dir`: the existing
/// `filelist_tmapXNNN.txt` there is followed (so a pack laid out by hand keeps its layout),
/// otherwise [`default_filelist`] is written. Returns the number of PNGs written.
pub fn export_pack(
    fs: &dyn ConfigFs,
    dir: &Path,
    set: &TextureSet,
    part: Part,
    pal: &Palette,
) -> Result<usize, String> {
    let list_path = dir.join(filelist_name(part, set.index));
    let text = match read_text(fs, &list_path)? {
        Some(t) => t,
        None => {
            let t = default_filelist(part, set.index);
            fs.write(&list_path, t.as_bytes())
                .map_err(|e| format!("{}: {e}", list_path.display()))?;
            t
        }
    };
    let entries = parse_filelist(&text)?;
    let atlas = set.atlas_indexed(part);
    // group the tiles per PNG
    let mut files: Vec<(String, Vec<(usize, &PackEntry)>)> = vec![];
    for (t, e) in entries.iter().enumerate() {
        match files.iter_mut().find(|f| f.0 == e.file) {
            Some(f) => f.1.push((t, e)),
            None => files.push((e.file.clone(), vec![(t, e)])),
        }
    }
    for (file, tiles) in &files {
        let w = tiles.iter().map(|(_, e)| e.x + e.w).max().unwrap_or(1);
        let h = tiles.iter().map(|(_, e)| e.y + e.h).max().unwrap_or(1);
        let mut img = Rgba::new(w, h);
        for (t, e) in tiles {
            let tile = tile_rgba(atlas, *t, pal).ok_or("tile out of range")?;
            let part = tile.crop(0, 0, e.w, e.h).ok_or("tile crop")?;
            img.blit(&part, e.x as i32, e.y as i32);
        }
        let png = img.to_png().map_err(|e| e.to_string())?;
        let p = dir.join(file);
        fs.write(&p, &png)
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(files.len())
}

/// Build a tmap file from the pack in `dir` (`filelist_tmapXNNN.txt` and its PNGs), starting
/// from `current` (tiles the filelist does not cover keep their pixels; a PNG smaller than a
/// tile's rectangle is an error).
pub fn import_pack(
    fs: &dyn ConfigFs,
    dir: &Path,
    part: Part,
    index: u32,
    current: &[u8],
    pal: &Palette,
    dither: bool,
) -> Result<Vec<u8>, String> {
    let list_path = dir.join(filelist_name(part, index));
    let text =
        read_text(fs, &list_path)?.ok_or_else(|| format!("{} not found", list_path.display()))?;
    let entries = parse_filelist(&text)?;
    let mut atlas = current.to_vec();
    atlas.resize(FILE_BYTES, kfx_gfx::textures::MISSING_FILL);
    let mut q = Quantizer::new(pal);
    let mut cache: std::collections::HashMap<String, Rgba> = Default::default();
    for (t, e) in entries.iter().enumerate() {
        if !cache.contains_key(&e.file) {
            let p = dir.join(&e.file);
            let b = fs
                .read(&p)
                .map_err(|er| format!("{}: {er}", p.display()))?
                .ok_or_else(|| format!("{} not found", p.display()))?;
            cache.insert(
                e.file.clone(),
                Rgba::from_png(&b).map_err(|er| format!("{}: {er}", p.display()))?,
            );
        }
        let img = &cache[&e.file];
        let r = img
            .crop(e.x, e.y, e.w, e.h)
            .ok_or_else(|| format!("{}: tile {t} is outside the image", e.file))?;
        let idx = q.quantize(&r, dither);
        let (col, row) = (t % TILES_PER_ROW, t / TILES_PER_ROW);
        for y in 0..e.h as usize {
            let o = (row * TILE_DIM + y) * ATLAS_W + col * TILE_DIM;
            atlas[o..o + e.w as usize]
                .copy_from_slice(&idx[y * e.w as usize..(y + 1) * e.w as usize]);
        }
    }
    Ok(atlas)
}

/// A hash of a pack's filelist and PNGs (`None` without filelist), to notice edits (live reload).
pub fn pack_fingerprint(fs: &dyn ConfigFs, dir: &Path, part: Part, index: u32) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let text = read_text(fs, &dir.join(filelist_name(part, index))).ok()??;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    let mut seen: Vec<String> = vec![];
    for e in parse_filelist(&text).ok()? {
        if !seen.contains(&e.file) {
            fs.read(&dir.join(&e.file)).ok().flatten().hash(&mut h);
            seen.push(e.file);
        }
    }
    Some(h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        let mut raw = vec![0u8; 768];
        for i in 0..256 {
            raw[i * 3] = (i % 64) as u8;
            raw[i * 3 + 1] = ((i / 4) % 64) as u8;
            raw[i * 3 + 2] = (i / 8) as u8 % 64;
        }
        Palette::from_bytes(&raw).unwrap()
    }

    #[test]
    fn quantize_exact_colours_round_trip() {
        let p = pal();
        let mut img = Rgba::new(16, 16);
        for i in 0..256u32 {
            img.put(i % 16, i / 16, p.rgba(i as u8));
        }
        let mut q = Quantizer::new(&p);
        let idx = q.quantize(&img, false);
        for i in 0..256 {
            // the palette may hold duplicates: the colour must match, not the index
            assert_eq!(p.rgba(idx[i]), p.rgba(i as u8));
        }
        let d = q.quantize(&img, true);
        for i in 0..256 {
            assert_eq!(
                p.rgba(d[i]),
                p.rgba(i as u8),
                "dithering exact colours adds no error"
            );
        }
    }

    #[test]
    fn paste_and_png() {
        let p = pal();
        let mut atlas = vec![0u8; FILE_BYTES];
        let mut img = Rgba::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                img.put(x, y, p.rgba(77));
            }
        }
        let mut q = Quantizer::new(&p);
        paste_tile(&mut atlas, 9, &img, &mut q, false).unwrap();
        // tile 9: column 1, row 1
        assert_eq!(p.rgba(atlas[(32 + 5) * ATLAS_W + 32 + 5]), p.rgba(77));
        assert_eq!(atlas[5 * ATLAS_W + 5], 0);
        let set = TextureSet::from_bytes(0, &atlas, None).unwrap();
        let png = atlas_png(&set, Part::A, &p).unwrap();
        let back = import_atlas_png(&png, &p, false).unwrap();
        for (a, b) in atlas.iter().zip(&back) {
            assert_eq!(p.rgba(*a), p.rgba(*b));
        }
    }

    #[test]
    fn pack_round_trip_and_lower_layer() {
        use crate::fs::MemFs;
        let p = pal();
        let mut atlas = vec![0u8; FILE_BYTES];
        for (i, b) in atlas.iter_mut().enumerate() {
            *b = ((i / ATLAS_W / TILE_DIM) * 3 % 256) as u8;
        }
        let set = TextureSet::from_bytes(4, &atlas, None).unwrap();
        let fs = MemFs::new();
        let dir = PathBuf::from("/pack");
        assert_eq!(
            export_pack(&fs, &dir, &set, Part::A, &p).unwrap(),
            TILES_PER_FILE
        );
        assert!(fs
            .read(&dir.join("filelist_tmapa004.txt"))
            .unwrap()
            .is_some());
        let f0 = pack_fingerprint(&fs, &dir, Part::A, 4).unwrap();
        let back = import_pack(&fs, &dir, Part::A, 4, &vec![0; FILE_BYTES], &p, false).unwrap();
        for (a, b) in atlas.iter().zip(&back) {
            assert_eq!(p.rgba(*a), p.rgba(*b));
        }
        // a pack laid out by hand: two tiles in one PNG
        let list = "textures_pack_004\t8\t68\t32\t32\n\
            pack/two.png\t0\t0\t32\t32\npack/two.png\t32\t0\t32\t32\n";
        fs.write(&dir.join("filelist_tmapb004.txt"), list.as_bytes())
            .unwrap();
        assert_eq!(export_pack(&fs, &dir, &set, Part::B, &p).unwrap(), 1);
        let two = Rgba::from_png(&fs.read(&dir.join("pack/two.png")).unwrap().unwrap()).unwrap();
        assert_eq!((two.w, two.h), (64, 32));
        // editing the PNG changes the fingerprint
        fs.write(
            &dir.join("pack004/tmapa/tile000.png"),
            &Rgba::new(32, 32).to_png().unwrap(),
        )
        .unwrap();
        assert_ne!(pack_fingerprint(&fs, &dir, Part::A, 4).unwrap(), f0);
        assert!(parse_filelist("textures_pack_0\n../x.png\t0\t0\t32\t32\n").is_err());
        assert!(parse_filelist("nope\n").is_err());
        // revert source: the map layer falls back to the campaign, then the install's data/
        let layers = LayerStack {
            layers: vec![
                Layer::base("/inst/fxdata", None),
                Layer::campaign(Some(PathBuf::from("/camp")), None),
                Layer::map("/lv", 3),
            ],
        };
        fs.write(Path::new("/inst/data/TMAPA004.DAT"), &[1; 1024])
            .unwrap();
        assert_eq!(
            lower_tmap(&fs, &layers, 2, Part::A, 4),
            Some((0, PathBuf::from("/inst/data/TMAPA004.DAT")))
        );
        fs.write(Path::new("/camp/tmapa004.dat"), &[2; 1024])
            .unwrap();
        assert_eq!(
            lower_tmap(&fs, &layers, 2, Part::A, 4),
            Some((1, PathBuf::from("/camp/tmapa004.dat")))
        );
        assert_eq!(lower_tmap(&fs, &layers, 2, Part::B, 4), None);
        // the effective set: a map override above wins
        fs.write(Path::new("/lv/map00003.tmapa004.dat"), &[3; 1024])
            .unwrap();
        let (set, src) = load_effective_set(&fs, &layers, 4).unwrap();
        assert_eq!(
            src[0],
            Some((2, PathBuf::from("/lv/map00003.tmapa004.dat")))
        );
        assert_eq!(src[1], None);
        assert_eq!(set.atlas_indexed(Part::A)[0], 3);
        assert_eq!(available_sets(&fs, &layers), vec![4]);
        fs.write(Path::new("/inst/data/tmapa007.dat"), &[1; 1024])
            .unwrap();
        assert_eq!(available_sets(&fs, &layers), vec![4, 7]);
        // the campaign has texture files and the map has none for this part: the campaign
        assert_eq!(
            tmap_write_layer(&fs, &layers, 2, Part::B, 4, false),
            (1, true)
        );
        assert_eq!(
            tmap_write_layer(&fs, &layers, 2, Part::B, 4, true),
            (2, false)
        );
        assert_eq!(
            tmap_write_layer(&fs, &layers, 2, Part::A, 4, false),
            (2, false),
            "the map has its own tmapa"
        );
        assert_eq!(
            tmap_write_layer(&fs, &layers, 1, Part::B, 4, false),
            (1, false)
        );
        assert_eq!(
            import_atlas_dat(&[7; 1024]).unwrap()[1024],
            kfx_gfx::textures::MISSING_FILL
        );
        assert!(import_atlas_dat(&[7; 10]).is_err());
    }
}
