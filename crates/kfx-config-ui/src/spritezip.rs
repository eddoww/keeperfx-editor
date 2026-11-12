//! Writing custom sprite / icon zips (`mapNNNNN.zip`, campaign zips) that the game's
//! `load_file_sprites` (`src/custom_sprites.c:232-309`) reads.
//!
//! `sprites.json`: an array of `{ "name", "rotatable", "td": [[{"file": ...}, ...] per
//! direction], "fp": [...], "td_offset_x" ... }` (`process_sprite_from_list`,
//! src/custom_sprites.c:1470-1530; frame keys `read_png_data` :1050-1085). The game requires as many
//! first-person as top-down frames, 1 direction (or 5 when rotatable: front, front-right, right,
//! back-right, back), and images smaller than 255x255. `icons.json`: `{ "name", "file": [png,
//! ...] }` (`process_icon_from_list` :2038-2105). Files are stored uncompressed (the wiki asks for
//! no compression to keep load times short).
use kfx_gfx::Rgba;
use serde_json::{json, Value};
use std::io::{Cursor, Read, Write};

/// Direction names used for generated file names.
pub const DIRECTIONS: [&str; 5] = ["front", "rf", "right", "rb", "back"];

/// A sprite (animation) to add.
#[derive(Clone, Debug, Default)]
pub struct NewSprite {
    pub name: String,
    pub rotatable: bool,
    /// Top-down frames per direction (1 or 5 directions).
    pub td: Vec<Vec<Rgba>>,
    /// First-person frames per direction; empty = reuse the top-down frames.
    pub fp: Vec<Vec<Rgba>>,
    /// `td_offset_x/y` (anchor inside the image; default bottom centre).
    pub td_offset: Option<(i32, i32)>,
    pub fp_offset: Option<(i32, i32)>,
}

/// An icon (GUI image) to add; several frames for e.g. enabled / disabled.
#[derive(Clone, Debug, Default)]
pub struct NewIcon {
    pub name: String,
    pub frames: Vec<Rgba>,
}

/// Problems that would make the game reject the sprite.
pub fn check_sprite(s: &NewSprite) -> Vec<String> {
    let mut out = vec![];
    if s.name.trim().is_empty() {
        out.push("the sprite needs a name".into());
    }
    if s.name.chars().any(|c| c.is_ascii_lowercase()) {
        out.push("names are conventionally upper case".into());
    }
    let dirs = if s.rotatable { 5 } else { 1 };
    if s.td.len() != dirs {
        out.push(format!(
            "{} top-down directions given, {dirs} needed",
            s.td.len()
        ));
    }
    let n0 = s.td.first().map_or(0, |d| d.len());
    if n0 == 0 {
        out.push("no top-down frames".into());
    }
    if s.td.iter().any(|d| d.len() != n0) {
        out.push("every direction needs the same number of frames".into());
    }
    let fp: &Vec<Vec<Rgba>> = if s.fp.is_empty() { &s.td } else { &s.fp };
    let (tdn, fpn): (usize, usize) = (
        s.td.iter().map(Vec::len).sum(),
        fp.iter().map(Vec::len).sum(),
    );
    if tdn != fpn {
        out.push(format!(
            "the game needs as many first-person as top-down frames ({fpn} vs {tdn})"
        ));
    }
    for img in s.td.iter().chain(fp.iter()).flatten() {
        if img.w >= 255 || img.h >= 255 {
            out.push(format!(
                "frames must be smaller than 255x255 ({}x{})",
                img.w, img.h
            ));
            break;
        }
    }
    out
}

fn png(img: &Rgba) -> Result<Vec<u8>, String> {
    img.to_png().map_err(|e| e.to_string())
}

fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut out = vec![];
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let mut b = vec![];
        f.read_to_end(&mut b).map_err(|e| e.to_string())?;
        out.push((f.name().to_string(), b));
    }
    Ok(out)
}

fn merge_list(list: &mut Vec<Value>, item: Value) {
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    list.retain(|v| {
        v.get("name")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            != Some(name.clone())
    });
    list.push(item);
}

/// Build a zip: the entries of `existing` (if any) plus the new sprites and icons; same-named
/// sprites / icons in the JSON lists are replaced.
pub fn write_zip(
    existing: Option<&[u8]>,
    sprites: &[NewSprite],
    icons: &[NewIcon],
) -> Result<Vec<u8>, String> {
    let mut files: Vec<(String, Vec<u8>)> = match existing {
        Some(b) => read_zip(b)?,
        None => vec![],
    };
    let take_json =
        |files: &mut Vec<(String, Vec<u8>)>, name: &str| -> Result<Vec<Value>, String> {
            match files.iter().position(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Some(i) => {
                    let (_, b) = files.remove(i);
                    let t = String::from_utf8_lossy(&b);
                    let v: Value = serde_json::from_str(t.trim_start_matches('\u{feff}'))
                        .map_err(|e| format!("{name}: {e}"))?;
                    Ok(v.as_array().cloned().unwrap_or_default())
                }
                None => Ok(vec![]),
            }
        };
    let mut sj = take_json(&mut files, "sprites.json")?;
    let mut ij = take_json(&mut files, "icons.json")?;
    let put = |files: &mut Vec<(String, Vec<u8>)>, name: String, b: Vec<u8>| {
        files.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
        files.push((name, b));
    };
    for s in sprites {
        let problems: Vec<String> = check_sprite(s)
            .into_iter()
            .filter(|p| !p.contains("conventionally"))
            .collect();
        if !problems.is_empty() {
            return Err(format!("{}: {}", s.name, problems.join("; ")));
        }
        let base = s.name.to_ascii_lowercase();
        let list = |frames: &Vec<Vec<Rgba>>,
                    view: &str,
                    files: &mut Vec<(String, Vec<u8>)>|
         -> Result<Value, String> {
            let mut dirs = vec![];
            for (d, fr) in frames.iter().enumerate() {
                let mut items = vec![];
                for (i, img) in fr.iter().enumerate() {
                    let f = format!("{base}/{view}_{}_{:02}.png", DIRECTIONS[d.min(4)], i);
                    put(files, f.clone(), png(img)?);
                    items.push(json!({ "file": f }));
                }
                dirs.push(Value::Array(items));
            }
            Ok(Value::Array(dirs))
        };
        let td = list(&s.td, "td", &mut files)?;
        let fp = if s.fp.is_empty() {
            list(&s.td, "fp", &mut files)?
        } else {
            list(&s.fp, "fp", &mut files)?
        };
        let mut item = json!({ "name": s.name, "rotatable": s.rotatable, "td": td, "fp": fp });
        if let Some((x, y)) = s.td_offset {
            item["td_offset_x"] = json!(x);
            item["td_offset_y"] = json!(y);
        }
        if let Some((x, y)) = s.fp_offset {
            item["fp_offset_x"] = json!(x);
            item["fp_offset_y"] = json!(y);
        }
        merge_list(&mut sj, item);
    }
    for ic in icons {
        if ic.frames.is_empty() || ic.name.trim().is_empty() {
            return Err(format!(
                "icon '{}' needs a name and at least one image",
                ic.name
            ));
        }
        let base = ic.name.to_ascii_lowercase();
        let mut names = vec![];
        for (i, img) in ic.frames.iter().enumerate() {
            let f = format!("icons/{base}_{i}.png");
            put(&mut files, f.clone(), png(img)?);
            names.push(Value::String(f));
        }
        merge_list(&mut ij, json!({ "name": ic.name, "file": names }));
    }
    if !sj.is_empty() {
        files.push((
            "sprites.json".into(),
            serde_json::to_vec_pretty(&Value::Array(sj)).map_err(|e| e.to_string())?,
        ));
    }
    if !ij.is_empty() {
        files.push((
            "icons.json".into(),
            serde_json::to_vec_pretty(&Value::Array(ij)).map_err(|e| e.to_string())?,
        ));
    }
    let mut out = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (n, b) in &files {
            w.start_file(n.as_str(), opts).map_err(|e| e.to_string())?;
            w.write_all(b).map_err(|e| e.to_string())?;
        }
        w.finish().map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

/// Entries of a zip with their sizes (for the browser).
pub fn list_zip(bytes: &[u8]) -> Result<Vec<(String, usize)>, String> {
    Ok(read_zip(bytes)?
        .into_iter()
        .map(|(n, b)| (n, b.len()))
        .collect())
}

/// The per-map zip name (`map%05lu.zip`, src/custom_sprites.c:378).
pub fn map_zip_name(level: u32) -> String {
    format!("map{level:05}.zip")
}
