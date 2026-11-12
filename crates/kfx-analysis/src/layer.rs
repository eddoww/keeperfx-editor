//! Per-slab overlay layers with legends, and a top-down RGBA renderer.
use crate::analysis::Analysis;
use crate::economy::NOBODY;
use crate::nav::{DistanceField, INF};
use crate::reach::Tier;
use crate::terrain::{Class, Terrain};

/// RGBA colour.
pub type Rgba = [u8; 4];

/// One legend entry of a categorical layer.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntry {
    pub value: u8,
    pub label: String,
    pub color: Rgba,
}

/// Layer values.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerData {
    /// One value per slab, `NaN` = no value. Drawn with [`ramp`] from `min` to `max`.
    Scalar {
        values: Vec<f32>,
        min: f32,
        max: f32,
        unit: String,
    },
    /// One category per slab, 255 = no value.
    Categorical {
        values: Vec<u8>,
        legend: Vec<LegendEntry>,
    },
}

/// A per-slab layer ready for a heat-map overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub w: i32,
    pub h: i32,
    pub data: LayerData,
}

/// Owner colours (red, blue, green, yellow, white, neutral, purple, black, orange).
pub const OWNER_COLORS: [Rgba; 9] = [
    [210, 40, 40, 255],
    [50, 90, 230, 255],
    [40, 170, 60, 255],
    [225, 210, 40, 255],
    [235, 235, 235, 255],
    [150, 150, 150, 255],
    [160, 70, 200, 255],
    [60, 60, 80, 255],
    [240, 140, 30, 255],
];

pub fn owner_color(o: u8) -> Rgba {
    OWNER_COLORS
        .get(o as usize)
        .copied()
        .unwrap_or([255, 0, 255, 255])
}

/// A 5-stop ramp (dark blue, teal, green, yellow, red), `t` in 0..1.
pub fn ramp(t: f32) -> Rgba {
    const S: [[f32; 3]; 5] = [
        [30.0, 30.0, 120.0],
        [20.0, 140.0, 160.0],
        [60.0, 190.0, 70.0],
        [240.0, 220.0, 40.0],
        [220.0, 40.0, 30.0],
    ];
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) } * 4.0;
    let i = (t as usize).min(3);
    let f = t - i as f32;
    let c = |k: usize| (S[i][k] + (S[i + 1][k] - S[i][k]) * f) as u8;
    [c(0), c(1), c(2), 255]
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), 255]
}

/// The colour of a slab in the top-down terrain view.
pub fn terrain_color(c: &crate::terrain::Cell) -> Rgba {
    let oc = owner_color(c.owner);
    match c.class {
        Class::Rock => [35, 33, 30, 255],
        Class::Earth => [118, 84, 52, 255],
        Class::Gold => [235, 195, 40, 255],
        Class::Gems => [190, 70, 230, 255],
        Class::Wall => mix([110, 100, 90, 255], oc, 0.35),
        Class::Floor if c.owner == kfx_map::autotile::PLAYER_NEUTRAL => [75, 66, 58, 255],
        Class::Floor => mix([75, 66, 58, 255], oc, 0.55),
        Class::Room => mix([140, 120, 100, 255], oc, 0.7),
        Class::Water => [40, 95, 205, 255],
        Class::Lava => [235, 85, 20, 255],
        Class::Door => mix([250, 250, 250, 255], oc, 0.4),
    }
}

/// An RGBA image, row-major, 4 bytes per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn new(w: u32, h: u32, fill: Rgba) -> RgbaImage {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            pixels.extend_from_slice(&fill);
        }
        RgbaImage { w, h, pixels }
    }
    pub fn put(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let a = c[3] as u32;
        for k in 0..3 {
            let d = self.pixels[i + k] as u32;
            self.pixels[i + k] = ((c[k] as u32 * a + d * (255 - a)) / 255) as u8;
        }
        self.pixels[i + 3] = 255;
    }
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c);
            }
        }
    }
    /// Paste `o` at (x, y).
    pub fn blit(&mut self, o: &RgbaImage, x: i32, y: i32) {
        for yy in 0..o.h as i32 {
            for xx in 0..o.w as i32 {
                let i = ((yy as u32 * o.w + xx as u32) * 4) as usize;
                self.put(
                    x + xx,
                    y + yy,
                    [o.pixels[i], o.pixels[i + 1], o.pixels[i + 2], 255],
                );
            }
        }
    }
}

/// The terrain, `scale` pixels per slab.
pub fn render_terrain(t: &Terrain, scale: u32) -> RgbaImage {
    let s = scale.max(1) as i32;
    let mut img = RgbaImage::new(t.w as u32 * s as u32, t.h as u32 * s as u32, [0, 0, 0, 255]);
    for y in 0..t.h {
        for x in 0..t.w {
            img.fill_rect(x * s, y * s, s, s, terrain_color(t.at(x, y)));
        }
    }
    img
}

/// The colour of one layer value, `None` for no value.
pub fn layer_color(l: &Layer, i: usize) -> Option<Rgba> {
    match &l.data {
        LayerData::Scalar {
            values, min, max, ..
        } => {
            let v = *values.get(i)?;
            if v.is_nan() {
                return None;
            }
            let span = (max - min).max(1e-6);
            Some(ramp((v - min) / span))
        }
        LayerData::Categorical { values, legend } => {
            let v = *values.get(i)?;
            legend.iter().find(|e| e.value == v).map(|e| e.color)
        }
    }
}

/// `layer` drawn over the terrain (`alpha` 0..255 for the layer), `scale` pixels per slab.
pub fn render_layer(t: &Terrain, layer: &Layer, scale: u32, alpha: u8) -> RgbaImage {
    let mut img = render_terrain(t, scale);
    let s = scale.max(1) as i32;
    for y in 0..t.h.min(layer.h) {
        for x in 0..t.w.min(layer.w) {
            if let Some(mut c) = layer_color(layer, (y * layer.w + x) as usize) {
                c[3] = alpha;
                img.fill_rect(x * s, y * s, s, s, c);
            }
        }
    }
    img
}

/// Draw a slab polyline (slab centres) with a `width` pixel pen.
pub fn draw_polyline(img: &mut RgbaImage, scale: u32, pts: &[(i32, i32)], color: Rgba, width: i32) {
    let s = scale.max(1) as i32;
    let c = |p: (i32, i32)| (p.0 * s + s / 2, p.1 * s + s / 2);
    for w in pts.windows(2) {
        let (a, b) = (c(w[0]), c(w[1]));
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for k in 0..=n {
            let x = a.0 + (b.0 - a.0) * k / n;
            let y = a.1 + (b.1 - a.1) * k / n;
            img.fill_rect(x - width / 2, y - width / 2, width, width, color);
        }
    }
}

/// A filled square marker centred on a slab.
pub fn draw_marker(img: &mut RgbaImage, scale: u32, slab: (i32, i32), color: Rgba, size: i32) {
    let s = scale.max(1) as i32;
    let (x, y) = (slab.0 * s + s / 2, slab.1 * s + s / 2);
    img.fill_rect(
        x - size / 2 - 1,
        y - size / 2 - 1,
        size + 2,
        size + 2,
        [0, 0, 0, 255],
    );
    img.fill_rect(x - size / 2, y - size / 2, size, size, color);
}

// --------------------------------------------------------------------------------- layers

/// Reachability tiers of keeper `p`.
pub fn reach_layer(a: &Analysis, p: u8) -> Option<Layer> {
    let f = a.keeper(p)?;
    let values = f.tiers().into_iter().map(|t| t as u8).collect();
    Some(Layer {
        name: format!("reachability {}", crate::sites::owner_name(p)),
        w: a.terrain.w,
        h: a.terrain.h,
        data: LayerData::Categorical {
            values,
            legend: vec![
                LegendEntry {
                    value: Tier::Walk as u8,
                    label: Tier::Walk.name().into(),
                    color: [60, 200, 80, 255],
                },
                LegendEntry {
                    value: Tier::Dig as u8,
                    label: Tier::Dig.name().into(),
                    color: [230, 200, 50, 255],
                },
                LegendEntry {
                    value: Tier::Cross as u8,
                    label: Tier::Cross.name().into(),
                    color: [240, 110, 30, 255],
                },
                LegendEntry {
                    value: Tier::Never as u8,
                    label: Tier::Never.name().into(),
                    color: [120, 20, 140, 255],
                },
            ],
        },
    })
}

/// A distance field in seconds (unreached slabs have no value).
pub fn distance_layer(name: &str, f: &DistanceField, turns_per_second: u32) -> Layer {
    let tps = turns_per_second.max(1) as f32;
    let values: Vec<f32> = f
        .dist
        .iter()
        .map(|&d| if d == INF { f32::NAN } else { d as f32 / tps })
        .collect();
    let max = values
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .fold(0.0f32, f32::max);
    Layer {
        name: name.into(),
        w: f.w,
        h: f.h,
        data: LayerData::Scalar {
            values,
            min: 0.0,
            max,
            unit: "s".into(),
        },
    }
}

/// The keeper reaching each slab first (dimmed when contested).
pub fn territory_layer(a: &Analysis) -> Layer {
    let mut legend = vec![];
    for f in &a.fields {
        let c = owner_color(f.player);
        legend.push(LegendEntry {
            value: f.player,
            label: crate::sites::owner_name(f.player).into(),
            color: c,
        });
        legend.push(LegendEntry {
            value: f.player + 16,
            label: format!("{} (contested)", crate::sites::owner_name(f.player)),
            color: mix(c, [255, 255, 255, 255], 0.6),
        });
    }
    let values = a
        .economy
        .owner_of
        .iter()
        .zip(&a.economy.contested)
        .map(|(&o, &c)| {
            if o == NOBODY {
                255
            } else if c {
                o + 16
            } else {
                o
            }
        })
        .collect();
    Layer {
        name: "territory (nearest heart)".into(),
        w: a.terrain.w,
        h: a.terrain.h,
        data: LayerData::Categorical { values, legend },
    }
}

/// Slab classes.
pub fn class_layer(t: &Terrain) -> Layer {
    let classes = [
        Class::Rock,
        Class::Earth,
        Class::Gold,
        Class::Gems,
        Class::Wall,
        Class::Floor,
        Class::Room,
        Class::Water,
        Class::Lava,
        Class::Door,
    ];
    let legend = classes
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let cell = crate::terrain::Cell {
                kind: 0,
                owner: kfx_map::autotile::PLAYER_NEUTRAL,
                class: *c,
                health: 0,
                gold: 0,
                room: 0,
                locked: false,
                door_health: 0,
            };
            LegendEntry {
                value: k as u8,
                label: c.name().into(),
                color: terrain_color(&cell),
            }
        })
        .collect();
    let values = t
        .cells
        .iter()
        .map(|c| classes.iter().position(|k| *k == c.class).unwrap_or(0) as u8)
        .collect();
    Layer {
        name: "slab class".into(),
        w: t.w,
        h: t.h,
        data: LayerData::Categorical { values, legend },
    }
}

/// The map with hearts, gates, portals, routes and contact points drawn: a quick overview for
/// tests and the command line.
pub fn render_overview(a: &Analysis, scale: u32) -> RgbaImage {
    let mut img = render_terrain(&a.terrain, scale);
    for r in &a.routes {
        let col = match r.from {
            crate::reach::Endpoint::Gate { .. } => [255, 255, 255, 200],
            _ => [255, 60, 200, 200],
        };
        draw_polyline(&mut img, scale, &r.polyline, col, (scale as i32 / 3).max(1));
    }
    for c in &a.contacts {
        draw_marker(
            &mut img,
            scale,
            c.slab,
            [255, 0, 255, 255],
            (scale as i32).max(3),
        );
    }
    for p in &a.sites.portals {
        draw_marker(
            &mut img,
            scale,
            p.centre,
            [20, 20, 20, 255],
            (scale as i32 * 2).max(3),
        );
    }
    for g in &a.sites.gates {
        draw_marker(
            &mut img,
            scale,
            g.slab,
            [255, 255, 255, 255],
            (scale as i32 * 2).max(3),
        );
    }
    for h in &a.sites.hearts {
        draw_marker(
            &mut img,
            scale,
            h.slab,
            owner_color(h.owner),
            (scale as i32 * 2).max(3),
        );
    }
    img
}
