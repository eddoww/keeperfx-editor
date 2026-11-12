//! A simple built-in top-down colour renderer for prefabs (thumbnails, the ghost in the browser).
//!
//! It needs no game data: slab colours come from the slab names, owners are shown as an inner
//! border in the player colour, things as small icons (creatures round, objects square, traps
//! red, doors a bar, hero gates green diamonds, action points cyan diamonds, lights yellow).
//! Masked cells are transparent.
use crate::apply::OwnerMap;
use crate::model::*;

/// An RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(w: u32, h: u32) -> Image {
        Image {
            w,
            h,
            rgba: vec![0; (w * h * 4) as usize],
        }
    }
    pub fn put(&mut self, x: i32, y: i32, c: [u8; 3], a: u8) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        if a == 255 || self.rgba[i + 3] == 0 {
            self.rgba[i..i + 3].copy_from_slice(&c);
            self.rgba[i + 3] = a.max(self.rgba[i + 3]);
        } else {
            for k in 0..3 {
                self.rgba[i + k] = ((self.rgba[i + k] as u32 * (255 - a as u32)
                    + c[k] as u32 * a as u32)
                    / 255) as u8;
            }
        }
    }
    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, c, 255);
            }
        }
    }
    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 3]) {
        let (x0, x1, y0, y1) = (
            (cx - r - 1.0).floor() as i32,
            (cx + r + 1.0).ceil() as i32,
            (cy - r - 1.0).floor() as i32,
            (cy + r + 1.0).ceil() as i32,
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.put(x, y, c, 255);
                }
            }
        }
    }
    fn diamond(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 3]) {
        let (x0, x1, y0, y1) = (
            (cx - r - 1.0).floor() as i32,
            (cx + r + 1.0).ceil() as i32,
            (cy - r - 1.0).floor() as i32,
            (cy + r + 1.0).ceil() as i32,
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                if (x as f32 + 0.5 - cx).abs() + (y as f32 + 0.5 - cy).abs() <= r {
                    self.put(x, y, c, 255);
                }
            }
        }
    }
    /// PNG bytes.
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = vec![];
        let enc = image::codecs::png::PngEncoder::new(&mut out);
        use image::ImageEncoder;
        let _ = enc.write_image(&self.rgba, self.w, self.h, image::ExtendedColorType::Rgba8);
        out
    }
    /// Decode PNG bytes.
    pub fn from_png(bytes: &[u8]) -> Option<Image> {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        Some(Image {
            w: img.width(),
            h: img.height(),
            rgba: img.into_raw(),
        })
    }
}

/// Player colour (RED, BLUE, GREEN, YELLOW, WHITE, NEUTRAL, PURPLE, BLACK, ORANGE).
pub fn player_color(p: u8) -> [u8; 3] {
    match p {
        0 => [230, 50, 50],
        1 => [60, 110, 240],
        2 => [60, 200, 80],
        3 => [240, 220, 50],
        4 => [245, 245, 245],
        5 => [140, 140, 140],
        6 => [170, 80, 210],
        7 => [30, 30, 36],
        8 => [250, 150, 40],
        _ => [255, 0, 255],
    }
}

/// Base colour of a slab by name.
pub fn slab_color(name: &str) -> [u8; 3] {
    match name {
        "HARD" => [52, 48, 46],
        "GOLD" => [196, 150, 36],
        "DENSE_GOLD" => [226, 182, 48],
        "GEMS" => [90, 206, 226],
        "DIRT" => [112, 78, 48],
        "TORCH_DIRT" => [128, 88, 52],
        "PATH" => [138, 124, 104],
        "PRETTY_PATH" => [170, 152, 124],
        "PURPLE_PATH" => [128, 96, 150],
        "HARD_FLOOR" => [96, 96, 104],
        "LAVA" => [222, 84, 18],
        "WATER" => [38, 92, 182],
        "ABYSS" => [10, 8, 22],
        "BRIDGE_FRAME" => [156, 112, 66],
        "ENTRANCE_ZONE" => [214, 208, 130],
        "HEART_PEDESTAL" => [212, 36, 64],
        "TREASURY_AREA" => [218, 172, 38],
        "LAIR_AREA" => [132, 84, 154],
        "HATCHERY" => [160, 200, 84],
        "BOOK_SHELVES" => [56, 88, 186],
        "TRAINING_AREA" => [190, 72, 46],
        "WORKSHOP_AREA" => [150, 104, 58],
        "PRISON_AREA" => [112, 118, 140],
        "TORTURE_AREA" => [164, 36, 74],
        "TEMPLE_POOL" => [120, 206, 232],
        "GRAVE_AREA" => [82, 106, 96],
        "SCAVENGE_AREA" => [88, 156, 122],
        "BARRACK_AREA" => [176, 154, 96],
        "GUARD_AREA" => [206, 114, 58],
        "DOOR_WOODEN" | "DOOR_WOODEN2" => [132, 90, 48],
        "DOOR_BRACE" | "DOOR_BRACE2" => [104, 104, 114],
        "DOOR_STEEL" | "DOOR_STEEL2" => [176, 188, 198],
        "DOOR_MAGIC" | "DOOR_MAGIC2" => [124, 84, 206],
        "DOOR_SECRET" | "DOOR_SECRET2" => [82, 80, 98],
        "DOOR_MIDAS" | "DOOR_MIDAS2" => [236, 204, 60],
        n if n.ends_with("_WALL") => [74, 74, 92],
        _ => [200, 0, 200],
    }
}

fn blend(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t) as u8)
}

fn is_room_or_heart(name: &str) -> bool {
    matches!(
        name,
        "HEART_PEDESTAL"
            | "TREASURY_AREA"
            | "LAIR_AREA"
            | "HATCHERY"
            | "BOOK_SHELVES"
            | "TRAINING_AREA"
            | "WORKSHOP_AREA"
            | "PRISON_AREA"
            | "TORTURE_AREA"
            | "TEMPLE_POOL"
            | "GRAVE_AREA"
            | "SCAVENGE_AREA"
            | "BARRACK_AREA"
            | "GUARD_AREA"
    )
}

/// Colour of a cell with owner tint.
fn cell_color(c: &Cell, owner: u8) -> [u8; 3] {
    let base = slab_color(&c.slab);
    if owner == 5
        || matches!(
            c.slab.as_str(),
            "HARD"
                | "GOLD"
                | "DENSE_GOLD"
                | "GEMS"
                | "DIRT"
                | "TORCH_DIRT"
                | "LAVA"
                | "WATER"
                | "ABYSS"
                | "BRIDGE_FRAME"
        )
    {
        base
    } else if c.slab.ends_with("_WALL") || c.slab.contains("PATH") {
        blend(base, player_color(owner), 0.22)
    } else {
        base
    }
}

fn thing_color(t: &PrefabThing) -> [u8; 3] {
    let m = t.model.as_str();
    match t.class {
        ThingClass::Object if t.gate.is_some() => [60, 230, 90],
        ThingClass::Object if m.starts_with("GOLD") => [255, 214, 40],
        ThingClass::Object if m.starts_with("SPECBOX") => [214, 90, 230],
        ThingClass::Object if m.starts_with("SPELLBOOK") => [90, 150, 255],
        ThingClass::Object if m.starts_with("WRKBOX") => [250, 150, 60],
        ThingClass::Object
            if m.starts_with("TORCH") || m.starts_with("CANDLE") || m.starts_with("LANTERN") =>
        {
            [255, 232, 120]
        }
        ThingClass::Object => [190, 190, 200],
        ThingClass::Trap => [255, 40, 40],
        ThingClass::EffectGen => [180, 255, 255],
        _ => [255, 255, 255],
    }
}

/// Render `p` with `px` pixels per slab (at least 2).
pub fn render(p: &Prefab, px: u32, owners: &OwnerMap) -> Image {
    let px = px.max(2) as i32;
    let mut img = Image::new(p.w * px as u32, p.h * px as u32);
    let border = if px >= 24 {
        3
    } else if px >= 12 {
        2
    } else {
        1
    };
    for y in 0..p.h as i32 {
        for x in 0..p.w as i32 {
            let Some(c) = p.cell(x as u32, y as u32) else {
                continue;
            };
            let owner = owners.map(c.owner);
            let col = cell_color(c, owner);
            img.rect(x * px, y * px, (x + 1) * px, (y + 1) * px, col);
            // subtle edge so single slabs read as tiles
            if px >= 8 {
                for k in 0..px {
                    img.put(x * px + k, y * px, [0, 0, 0], 28);
                    img.put(x * px, y * px + k, [0, 0, 0], 28);
                }
            }
            // owner colour along the outline of owned rooms, heart and doors (not between two slabs
            // of the same room)
            let owned = owner != 5 && (is_room_or_heart(&c.slab) || c.slab.starts_with("DOOR_"));
            if owned && px >= 6 {
                let oc = player_color(owner);
                let same = |dx: i32, dy: i32| {
                    p.cell((x + dx).max(0) as u32, (y + dy).max(0) as u32)
                        .is_some_and(|n| {
                            x + dx >= 0 && y + dy >= 0 && n.slab == c.slab && n.owner == c.owner
                        })
                };
                for k in 0..border {
                    for i in 0..px {
                        if !same(0, -1) {
                            img.put(x * px + i, y * px + k, oc, 255);
                        }
                        if !same(0, 1) {
                            img.put(x * px + i, y * px + px - 1 - k, oc, 255);
                        }
                        if !same(-1, 0) {
                            img.put(x * px + k, y * px + i, oc, 255);
                        }
                        if !same(1, 0) {
                            img.put(x * px + px - 1 - k, y * px + i, oc, 255);
                        }
                    }
                }
            }
            if c.slab.starts_with("DOOR_") && px >= 6 {
                img.rect(
                    x * px + px / 4,
                    y * px + px * 3 / 8,
                    x * px + px * 3 / 4,
                    y * px + px * 5 / 8,
                    [30, 24, 20],
                );
            }
        }
    }
    let to_px = |raw: i32| raw as f32 * px as f32 / RAW_PER_SLAB as f32;
    for l in &p.lights {
        img.disc(
            to_px(l.x),
            to_px(l.y),
            (px as f32 * 0.12).max(1.0),
            [255, 245, 150],
        );
    }
    for t in &p.things {
        let (cx, cy) = (to_px(t.x), to_px(t.y));
        let r = (px as f32 * 0.26).max(1.5);
        let col = thing_color(t);
        match t.class {
            ThingClass::Creature => {
                let oc = player_color(owners.map(t.owner));
                img.disc(cx, cy, r + 1.0, [10, 10, 10]);
                img.disc(cx, cy, r, oc);
                img.disc(cx, cy, r * 0.45, col);
            }
            ThingClass::Door => {}
            ThingClass::Trap => {
                img.diamond(cx, cy, r + 1.0, [10, 10, 10]);
                img.diamond(cx, cy, r, col);
            }
            _ if t.gate.is_some() => {
                img.diamond(cx, cy, r * 1.5 + 1.0, [10, 10, 10]);
                img.diamond(cx, cy, r * 1.5, col);
            }
            _ => {
                let (x0, y0, x1, y1) = (
                    (cx - r * 0.7) as i32,
                    (cy - r * 0.7) as i32,
                    (cx + r * 0.7) as i32 + 1,
                    (cy + r * 0.7) as i32 + 1,
                );
                img.rect(x0 - 1, y0 - 1, x1 + 1, y1 + 1, [10, 10, 10]);
                img.rect(x0, y0, x1, y1, col);
            }
        }
    }
    for a in &p.action_points {
        let (cx, cy) = (to_px(a.x), to_px(a.y));
        let r = (px as f32 * 0.3).max(2.0);
        img.diamond(cx, cy, r + 1.0, [0, 40, 50]);
        img.diamond(cx, cy, r, [60, 230, 240]);
    }
    img
}

/// A thumbnail: `render` at a size that fits `max_side` pixels (whole pixels per slab, at least 2).
pub fn thumbnail(p: &Prefab, max_side: u32) -> Image {
    let px = (max_side / p.w.max(p.h)).clamp(2, 16);
    render(p, px, &OwnerMap::default())
}

/// A thumbnail encoded as PNG, for [`Prefab::thumbnail`].
pub fn thumbnail_png(p: &Prefab, max_side: u32) -> Vec<u8> {
    thumbnail(p, max_side).to_png()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_cells_and_png_round_trips() {
        let mut p = Prefab::blank("t", 3, 2);
        p.set_cell(0, 0, Some(Cell::new("HARD", Owner::Neutral)));
        p.set_cell(1, 0, Some(Cell::new("TREASURY_AREA", Owner::Slot(0))));
        let img = render(&p, 8, &OwnerMap::default());
        assert_eq!((img.w, img.h), (24, 16));
        assert_eq!(img.rgba[3], 255, "hard rock is opaque");
        let masked = ((8 * 24 + 20) * 4 + 3) as usize;
        assert_eq!(img.rgba[masked], 0, "masked cell is transparent");
        let back = Image::from_png(&img.to_png()).unwrap();
        assert_eq!(back, img);
    }
}
