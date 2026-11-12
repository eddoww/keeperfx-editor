//! Plan building, first part: generation context, creature and room lists, symmetry choice,
//! keeper base positions and the noise background.
use super::*;

/// What the plan building steps share: game data, slab kinds and parameters.
pub(super) struct Ctx<'a> {
    pub(super) gd: &'a GameData,
    pub(super) k: Kinds,
    pub(super) p: &'a GenParams,
    /// Impenetrable rock on the outer ring (whole maps; not for a rectangle patch).
    pub(super) border: bool,
}

pub(super) fn creature_models(gd: &GameData, names: &[&str]) -> Vec<i64> {
    names
        .iter()
        .filter_map(|n| gd.creature_id(n).map(|i| i as i64))
        .collect()
}

pub(super) const NEUTRAL_CREATURES: [&str; 12] = [
    "FLY",
    "BUG",
    "SPIDER",
    "DEMONSPAWN",
    "TROLL",
    "ORC",
    "SORCEROR",
    "BILE_DEMON",
    "DARK_MISTRESS",
    "HELL_HOUND",
    "TENTACLE",
    "VAMPIRE",
];
pub(super) const HERO_CREATURES: [&str; 9] = [
    "KNIGHT",
    "ARCHER",
    "MONK",
    "WIZARD",
    "BARBARIAN",
    "SAMURAI",
    "DWARFA",
    "GIANT",
    "WITCH",
];
pub(super) const NEUTRAL_ROOMS: [&str; 6] = [
    "RESEARCH",
    "TRAINING",
    "LAIR",
    "TEMPLE",
    "GRAVEYARD",
    "WORKSHOP",
];

pub(super) fn special_boxes(gd: &GameData) -> Vec<i64> {
    gd.object_names()
        .into_iter()
        .map(|(m, _)| m as i64)
        .filter(|&m| {
            kthings::object_genre_is(gd, m, "SPECIALBOX")
                && gd
                    .objects_cfg
                    .get(&format!("object{m}"), "Name")
                    .is_some_and(|n| !n.contains("CUSTOM"))
        })
        .collect()
}

/// Pick the symmetry in effect, or explain why the request cannot be met.
pub(super) fn resolve_symmetry(
    p: &GenParams,
    w: i32,
    h: i32,
    rng: &mut Rng,
) -> Result<Sym, String> {
    let n = p.players as u32;
    let phase = rng.below(1000) as f64 / 1000.0;
    let base = Sym {
        mode: None,
        radial: 0,
        phase,
        w,
        h,
    };
    let mode = |m: SymmetryMode| Sym {
        mode: Some(m),
        ..base
    };
    Ok(match p.symmetry {
        SymmetryChoice::None => base,
        SymmetryChoice::Radial => Sym {
            radial: n.max(2),
            ..base
        },
        SymmetryChoice::Mode(m) => {
            if m == SymmetryMode::Rot4 && w != h {
                return Err(format!(
                    "quarter-turn symmetry needs a square map, not {w}x{h}"
                ));
            }
            let o = if matches!(m, SymmetryMode::FourWay | SymmetryMode::Rot4) {
                4
            } else {
                2
            };
            if n > 0 && n % o != 0 {
                return Err(format!(
                    "{m:?} symmetry needs a multiple of {o} keepers, not {n}"
                ));
            }
            mode(m)
        }
        SymmetryChoice::Auto => match n {
            0 | 1 => base,
            2 => mode(SymmetryMode::Rot2),
            4 if w == h => mode(SymmetryMode::Rot4),
            4 => mode(SymmetryMode::FourWay),
            _ => Sym { radial: n, ..base },
        },
    })
}

/// Seed angles (turns, y down) of the keepers in the fundamental region of a mode.
pub(super) fn seed_angles(sym: &Sym, n: usize, rng: &mut Rng) -> Vec<f64> {
    if n == 0 {
        return vec![];
    }
    if sym.radial > 1 {
        return vec![sym.phase];
    }
    let o = sym.order();
    let k = n / o;
    let (a0, a1) = match sym.mode {
        Some(SymmetryMode::NorthSouth) | Some(SymmetryMode::Rot2) => (0.5, 1.0),
        Some(SymmetryMode::EastWest) => (0.25, 0.75),
        Some(SymmetryMode::FourWay) | Some(SymmetryMode::Rot4) => (0.5, 0.75),
        None => {
            // free placement: evenly spaced with a little jitter
            return (0..n)
                .map(|i| sym.phase + i as f64 / n as f64 + (rng.below(60) as f64 - 30.0) / 1000.0)
                .collect();
        }
    };
    let jitter = if k == 1 {
        0.0
    } else {
        (a1 - a0) / k as f64 * 0.15
    };
    (0..k)
        .map(|i| {
            a0 + (a1 - a0) * (i as f64 + 0.5) / k as f64
                + jitter * (rng.below(200) as f64 / 100.0 - 1.0)
        })
        .collect()
}

pub(super) fn dir(a: f64) -> (f64, f64) {
    cos_sin(a)
}

/// Lay out the keeper bases (positions only).
pub(super) fn place_bases(
    plan: &mut Plan,
    n: usize,
    ring: f64,
    rng: &mut Rng,
) -> Result<(), String> {
    if n == 0 {
        return Ok(());
    }
    let sym = plan.sym;
    let (cx, cy) = sym.centre();
    let (rx, ry) = (
        ring * (plan.w as f64 - 1.0) / 2.0,
        ring * (plan.h as f64 - 1.0) / 2.0,
    );
    let seeds = seed_angles(&sym, n, rng);
    let mut bases = vec![];
    for a in seeds {
        let (c, s) = dir(a);
        let heart = (cx + rx * c, cy + ry * s);
        let heart = (round(heart.0) as f64, round(heart.1) as f64);
        if sym.mode.is_none() && sym.radial <= 1 {
            bases.push((heart, a));
            continue;
        }
        for (k, img) in sym.point_images(heart).into_iter().enumerate() {
            bases.push((
                (round(img.0) as f64, round(img.1) as f64),
                sym.angle_image(a, k),
            ));
        }
    }
    bases.truncate(n);
    for (i, ((x, y), a)) in bases.into_iter().enumerate() {
        let (x, y) = (x as i32, y as i32);
        if x < 6 || y < 6 || x > plan.w - 7 || y > plan.h - 7 {
            return Err(format!(
                "map {}x{} too small for {n} keepers (heart at ({x},{y}) is too \
                close to the border)",
                plan.w, plan.h
            ));
        }
        plan.bases.push(Base {
            owner: KEEPERS[i],
            heart: (x, y),
            portal: None,
            angle: a.rem_euclid(1.0),
        });
    }
    for i in 0..plan.bases.len() {
        for j in i + 1..plan.bases.len() {
            let (a, b) = (plan.bases[i].heart, plan.bases[j].heart);
            if (a.0 - b.0).abs().max((a.1 - b.1).abs()) < 18 {
                return Err(format!(
                    "map {}x{} too small for {n} keepers (hearts {a:?} and {b:?} \
                    closer than 18 slabs)",
                    plan.w, plan.h
                ));
            }
        }
    }
    Ok(())
}

/// Background terrain: earth with rock outcrops, gold veins and pools (noise at the
/// representative so it follows the symmetry).
pub(super) fn background(
    ctx: &Ctx,
    plan: &mut Plan,
    rng: &mut Rng,
    rock_pct: u32,
    gold_pct: u32,
    lava_pct: u32,
    water_pct: u32,
    cave_pct: u32,
) {
    let k = &ctx.k;
    let seeds: [u64; 5] = std::array::from_fn(|_| rng.next_u64());
    let (w, h) = (plan.w, plan.h);
    let mut fields: Vec<[u32; 5]> = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let (rx, ry) = plan.sym.rep(x, y);
            let (rx, ry) = (rx as i64, ry as i64);
            fields.push([
                fractal(seeds[0], rx, ry, 8),
                // gold seams: clumps of a smooth noise, denser in some regions
                {
                    let n = crate::rng::value_noise(seeds[1], rx, ry, 5) as u64;
                    let zone = crate::rng::value_noise(seeds[1] ^ 0xA5A5, rx, ry, 17) as u64;
                    (n * (32768 + zone) / 98304) as u32
                },
                fractal(seeds[2], rx, ry, 9),
                fractal(seeds[3], rx, ry, 9),
                fractal(seeds[4], rx, ry, 11),
            ]);
        }
    }
    // thresholds from histograms (exact, deterministic)
    let thr = |f: usize, pct: u32| -> u32 {
        if pct == 0 {
            return u32::MAX;
        }
        let mut v: Vec<u32> = fields.iter().map(|a| a[f]).collect();
        v.sort_unstable();
        let at = v
            .len()
            .saturating_sub((v.len() as u64 * pct as u64 / 100) as usize)
            .min(v.len() - 1);
        v[at]
    };
    let (tr, tg, tl, tw, tc) = (
        thr(0, rock_pct),
        thr(1, gold_pct),
        thr(2, lava_pct),
        thr(3, water_pct),
        thr(4, cave_pct),
    );
    for y in 0..h {
        for x in 0..w {
            let f = fields[(y * w + x) as usize];
            let kind = if f[0] >= tr {
                k.rock
            } else if f[2] >= tl {
                k.lava
            } else if f[3] >= tw {
                k.water
            } else if f[4] >= tc {
                k.path
            } else if f[1] >= tg {
                k.gold
            } else {
                k.earth
            };
            plan.set(x, y, kind, PLAYER_NEUTRAL);
        }
    }
}

pub(super) fn wall_kind(k: &Kinds, x: i32, y: i32) -> u16 {
    if k.walls.is_empty() {
        return k.rock;
    }
    k.walls[((x + 2 * y).rem_euclid(k.walls.len() as i32)) as usize]
}
