//! Plan building, second part: keeper bases, style features, extras (gold, gems, neutral
//! rooms and creatures, hero gates) and the border, assembled by [`build_plan`].
use super::*;

/// Stamp the bases: clear earth, claimed square, heart; portal towards the centre; gold.
pub(super) fn stamp_bases(ctx: &Ctx, plan: &mut Plan, rng: &mut Rng) {
    let k = &ctx.k;
    let gold_n = 2.0 + ctx.p.richness.min(100) as f64 / 40.0;
    // portal and gold offsets are chosen once and turned with the base (symmetric layouts)
    let portal_turn = (rng.below(120) as f64 - 60.0) / 1000.0;
    let portal_dist = 10.0 + rng.below(3) as f64;
    let gold_side = if rng.chance(1, 2) { 0.25 } else { -0.25 };
    let bases = plan.bases.clone();
    for (i, b) in bases.iter().enumerate() {
        let (hx, hy) = (b.heart.0 as f64, b.heart.1 as f64);
        let a_in = b.angle + 0.5; // towards the centre
                                  // mirrored bases turn the other way
        let flip = match plan.sym.mode {
            Some(SymmetryMode::NorthSouth)
            | Some(SymmetryMode::EastWest)
            | Some(SymmetryMode::FourWay) => {
                let imgs = plan.sym.order();
                let k = i % imgs;
                matches!(k, 1 | 2) && imgs == 4 || (imgs == 2 && k == 1)
            }
            _ => false,
        };
        let sgn = if flip { -1.0 } else { 1.0 };
        // clear around the heart
        for (x, y) in plan.cells(Shape::Disk {
            c: (hx, hy),
            r: 6.5,
        }) {
            let kk = plan.kind(x, y);
            if kk != k.earth || plan.owners[plan.idx(x, y)] != PLAYER_NEUTRAL {
                plan.set(x, y, k.earth, PLAYER_NEUTRAL);
            }
        }
        // portal
        let (pc, ps) = dir(a_in + sgn * portal_turn);
        let pp = (round(hx + pc * portal_dist), round(hy + ps * portal_dist));
        let portal = if plan.interior(pp.0 - 2, pp.1 - 2) && plan.interior(pp.0 + 2, pp.1 + 2) {
            for (x, y) in plan.cells(Shape::Disk {
                c: (pp.0 as f64, pp.1 as f64),
                r: 3.6,
            }) {
                if plan.kind(x, y) != k.earth {
                    plan.set(x, y, k.earth, PLAYER_NEUTRAL);
                }
            }
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let kk = if dx.abs() <= 1 && dy.abs() <= 1 {
                        k.portal
                    } else {
                        k.path
                    };
                    plan.force(pp.0 + dx, pp.1 + dy, kk, PLAYER_NEUTRAL);
                    let i = plan.idx(pp.0 + dx, pp.1 + dy);
                    plan.locked[i] = true;
                }
            }
            Some(pp)
        } else {
            None
        };
        plan.bases[i].portal = portal;
        // gold to the side
        let (gc, gs) = dir(a_in + sgn * gold_side);
        let gp = (hx + gc * 8.0, hy + gs * 8.0);
        for (x, y) in plan.cells(Shape::Disk {
            c: gp,
            r: gold_n.sqrt() + 0.6,
        }) {
            if plan.kind(x, y) == k.earth {
                plan.set(x, y, k.gold, PLAYER_NEUTRAL);
            }
        }
        // claimed square and heart
        for dy in -3i32..=3 {
            for dx in -3i32..=3 {
                let kk = if dx.abs() <= 1 && dy.abs() <= 1 {
                    k.heart
                } else {
                    k.claimed
                };
                plan.force(b.heart.0 + dx, b.heart.1 + dy, kk, b.owner);
                let j = plan.idx(b.heart.0 + dx, b.heart.1 + dy);
                plan.locked[j] = true;
            }
        }
        // imps
        if let Some(imp) = ctx.gd.creature_id("IMP") {
            let spots = [
                (-2, 0),
                (2, 0),
                (0, -2),
                (0, 2),
                (-2, -2),
                (2, 2),
                (-2, 2),
                (2, -2),
            ];
            for &(dx, dy) in spots.iter().take(ctx.p.imps as usize) {
                plan.things.push(PlanThing {
                    kind: PlanThingKind::Creature {
                        model: imp as i64,
                        level: 1,
                    },
                    owner: b.owner,
                    slab: (b.heart.0 + dx, b.heart.1 + dy),
                });
            }
        }
    }
}

/// Points away from the bases for features: `count` orbits (each point stamped with its images).
pub(super) fn feature_points(
    plan: &Plan,
    rng: &mut Rng,
    count: usize,
    min_heart: i32,
    min_gap: i32,
    taken: &mut Vec<(i32, i32)>,
) -> Vec<(i32, i32)> {
    let mut out = vec![];
    let mut tries = 0;
    while out.len() < count && tries < 400 {
        tries += 1;
        let p = (rng.range(5, plan.w - 6), rng.range(5, plan.h - 6));
        let imgs = plan.slab_images(p);
        let ok = imgs.iter().all(|&q| {
            plan.interior(q.0 - 4, q.1 - 4)
                && plan.interior(q.0 + 4, q.1 + 4)
                && plan.bases.iter().all(|b| {
                    (b.heart.0 - q.0).abs().max((b.heart.1 - q.1).abs()) >= min_heart
                        && b.portal
                            .is_none_or(|pp| (pp.0 - q.0).abs().max((pp.1 - q.1).abs()) >= 7)
                })
                && taken
                    .iter()
                    .all(|t| (t.0 - q.0).abs().max((t.1 - q.1).abs()) >= min_gap)
                && !plan.locked[plan.idx(q.0, q.1)]
        });
        // images of one point must not overlap each other
        let spread = imgs.iter().enumerate().all(|(i, a)| {
            imgs.iter()
                .skip(i + 1)
                .all(|b| (a.0 - b.0).abs().max((a.1 - b.1).abs()) >= min_gap || a == b)
        });
        if ok && spread {
            taken.extend(imgs.iter().copied());
            out.push(p);
        }
    }
    out
}

pub(super) fn style_features(ctx: &Ctx, plan: &mut Plan, rng: &mut Rng) {
    let k = ctx.k.clone();
    let p = ctx.p;
    let (cx, cy) = plan.sym.centre();
    let size = (plan.w.min(plan.h)) as f64;
    let any = |_: u16| true;
    let liquid = if p.lava > p.water { k.lava } else { k.water };
    match p.style {
        Style::OpenCave => {}
        Style::Chambers => {
            let n = ((plan.w * plan.h) as usize / 900).clamp(2, 10) / plan.sym.order().max(1) + 1;
            let mut taken = vec![];
            let pts = feature_points(plan, rng, n, 11, 9, &mut taken);
            let mut all: Vec<(i32, i32)> = pts.iter().flat_map(|&q| plan.slab_images(q)).collect();
            all.sort_by_key(|&(x, y)| (y, x));
            all.dedup();
            for &q in &pts {
                let (rw, rh) = (rng.range(2, 4) as f64, rng.range(2, 3) as f64);
                plan.stamp(
                    Shape::Rect {
                        a: (q.0 as f64 - rw, q.1 as f64 - rh),
                        b: (q.0 as f64 + rw, q.1 as f64 + rh),
                    },
                    k.path,
                    PLAYER_NEUTRAL,
                    any,
                );
            }
            // corridors: every chamber to its nearest other chamber (images follow)
            for &q in &pts {
                if let Some(&t) = all
                    .iter()
                    .filter(|&&t| t != q)
                    .min_by_key(|&&t| ((t.0 - q.0).pow(2) + (t.1 - q.1).pow(2), t.1, t.0))
                {
                    let mid = (t.0 as f64, q.1 as f64);
                    plan.stamp(
                        Shape::Capsule {
                            a: (q.0 as f64, q.1 as f64),
                            b: mid,
                            r: 0.5,
                        },
                        k.path,
                        PLAYER_NEUTRAL,
                        |kk| kk != k.lava,
                    );
                    plan.stamp(
                        Shape::Capsule {
                            a: mid,
                            b: (t.0 as f64, t.1 as f64),
                            r: 0.5,
                        },
                        k.path,
                        PLAYER_NEUTRAL,
                        |kk| kk != k.lava,
                    );
                }
            }
        }
        Style::Islands => {
            // sea, then islands around the bases and the centre, joined by causeways
            for y in 1..plan.h - 1 {
                for x in 1..plan.w - 1 {
                    plan.set(x, y, liquid, PLAYER_NEUTRAL);
                }
            }
            let seed = rng.next_u64();
            let r_base = (size * 0.16).max(9.0);
            let bases = plan.bases.clone();
            let island = |plan: &mut Plan, c: (f64, f64), r: f64| {
                for (x, y) in plan.cells(Shape::Blob { c, r, seed }) {
                    plan.set(x, y, k.earth, PLAYER_NEUTRAL);
                }
            };
            island(plan, (cx, cy), (size * 0.12).max(6.0));
            for b in &bases {
                island(plan, (b.heart.0 as f64, b.heart.1 as f64), r_base);
                for (x, y) in plan.cells(Shape::Capsule {
                    a: (b.heart.0 as f64, b.heart.1 as f64),
                    b: (cx, cy),
                    r: 1.3,
                }) {
                    plan.set(x, y, k.earth, PLAYER_NEUTRAL);
                }
            }
            let mut taken = vec![];
            for q in feature_points(plan, rng, 2, 14, 10, &mut taken) {
                plan.stamp(
                    Shape::Blob {
                        c: (q.0 as f64, q.1 as f64),
                        r: 4.0,
                        seed,
                    },
                    k.earth,
                    PLAYER_NEUTRAL,
                    any,
                );
                plan.stamp(
                    Shape::Disk {
                        c: (q.0 as f64, q.1 as f64),
                        r: 1.2,
                    },
                    k.gold,
                    PLAYER_NEUTRAL,
                    any,
                );
            }
            // the bases' own causeways may be cut by the extra islands' noise: none (earth only
            // added)
        }
        Style::River => {
            let mut angles: Vec<f64> = plan.bases.iter().map(|b| b.angle.rem_euclid(1.0)).collect();
            angles.sort_by(|a, b| a.total_cmp(b));
            let arms: Vec<f64> = if angles.len() < 2 {
                vec![plan.sym.phase + 0.25, plan.sym.phase + 0.75]
            } else {
                (0..angles.len())
                    .map(|i| {
                        let a = angles[i];
                        let b = if i + 1 < angles.len() {
                            angles[i + 1]
                        } else {
                            angles[0] + 1.0
                        };
                        (a + b) / 2.0
                    })
                    .collect()
            };
            let reach = size * 0.75;
            let width = 1.2 + (p.water.max(p.lava) as f64 / 100.0) * 1.3;
            let seed = rng.next_u64();
            for (ai, &a) in arms.iter().enumerate() {
                let (c, s) = dir(a);
                let (nx, ny) = (-s, c);
                let steps = 12;
                let mut prev = (cx, cy);
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let off =
                        (fractal(seed, i as i64, 0, 3) as f64 / 65535.0 - 0.5) * 6.0 * t.min(0.8);
                    let pt = (cx + c * reach * t + nx * off, cy + s * reach * t + ny * off);
                    for (x, y) in plan.cells(Shape::Capsule {
                        a: prev,
                        b: pt,
                        r: width,
                    }) {
                        if plan.kind(x, y) != k.gems {
                            plan.set(x, y, liquid, PLAYER_NEUTRAL);
                        }
                    }
                    prev = pt;
                }
                // fords
                for f in [0.3 + 0.1 * (ai % 2) as f64, 0.62] {
                    let off = (fractal(seed, (f * steps as f64) as i64, 0, 3) as f64 / 65535.0
                        - 0.5)
                        * 6.0
                        * f.min(0.8);
                    let pt = (cx + c * reach * f + nx * off, cy + s * reach * f + ny * off);
                    for (x, y) in plan.cells(Shape::Capsule {
                        a: (pt.0 - nx * 4.0, pt.1 - ny * 4.0),
                        b: (pt.0 + nx * 4.0, pt.1 + ny * 4.0),
                        r: 1.0,
                    }) {
                        if plan.kind(x, y) == liquid {
                            plan.set(x, y, k.earth, PLAYER_NEUTRAL);
                        }
                    }
                }
            }
        }
        Style::Fortress => {
            let r = (size * 0.12).round().clamp(6.0, 14.0);
            let (fx0, fy0, fx1, fy1) = (
                round(cx - r),
                round(cy - r),
                round(cx + (cx - (cx - r).round())),
                round(cy + (cy - (cy - r).round())),
            );
            // moat
            if p.water.max(p.lava) > 0 {
                let m = 2;
                for y in fy0 - m - 1..=fy1 + m + 1 {
                    for x in fx0 - m - 1..=fx1 + m + 1 {
                        let ring = x == fx0 - m - 1
                            || x == fx1 + m + 1
                            || y == fy0 - m - 1
                            || y == fy1 + m + 1
                            || x == fx0 - m
                            || x == fx1 + m
                            || y == fy0 - m
                            || y == fy1 + m;
                        if plan.interior(x, y) && ring {
                            plan.set(x, y, liquid, PLAYER_NEUTRAL);
                        }
                    }
                }
            }
            for y in fy0 - 1..=fy1 + 1 {
                for x in fx0 - 1..=fx1 + 1 {
                    if !plan.interior(x, y) {
                        continue;
                    }
                    let edge = x == fx0 || x == fx1 || y == fy0 || y == fy1;
                    let outside = x < fx0 || x > fx1 || y < fy0 || y > fy1;
                    let (kk, o) = if outside {
                        (k.earth, PLAYER_NEUTRAL)
                    } else if edge {
                        (wall_kind(&k, x, y), HEROES)
                    } else {
                        (k.claimed, HEROES)
                    };
                    plan.force(x, y, kk, o);
                    let i = plan.idx(x, y);
                    plan.locked[i] = !outside;
                }
            }
            // doors: images of one door on the north wall and one on the west wall, with earth
            // (and a moat crossing) in front
            let door_model = ctx
                .gd
                .door_names()
                .into_iter()
                .map(|(i, _)| i as i64)
                .find(|&i| i == 2)
                .or_else(|| {
                    ctx.gd
                        .door_names()
                        .into_iter()
                        .map(|(i, _)| i as i64)
                        .find(|&i| i > 0)
                });
            let mid = (round(cx) - 2, round(cy) + 2);
            let mut doors: Vec<(i32, i32)> = vec![];
            for seed_door in [(mid.0, fy0), (fx0, mid.1)] {
                for d in plan.slab_images(seed_door) {
                    if !doors.contains(&d)
                        && doors
                            .iter()
                            .all(|o| (o.0 - d.0).abs() + (o.1 - d.1).abs() >= 3)
                    {
                        doors.push(d);
                    }
                }
            }
            if plan.sym.mode.is_none() {
                doors = vec![
                    (mid.0, fy0),
                    (fx0, mid.1),
                    (round(cx) + 2, fy1),
                    (fx1, round(cy) - 2),
                ];
            }
            for &(x, y) in &doors {
                // walls must stay on both sides along the wall line (corners excluded)
                let on_h = y == fy0 || y == fy1;
                let out = if on_h {
                    (0, if y == fy0 { -1 } else { 1 })
                } else {
                    (if x == fx0 { -1 } else { 1 }, 0)
                };
                if let Some(m) = door_model {
                    if let Some(ks) = kthings::door_slab_kinds(ctx.gd, m) {
                        plan.force(x, y, ks[0], HEROES);
                        plan.doors.push(PlanDoor {
                            slab: (x, y),
                            model: m,
                            owner: HEROES,
                            locked: false,
                        });
                    }
                }
                for s in 1..=4 {
                    let (ox, oy) = (x + out.0 * s, y + out.1 * s);
                    if plan.interior(ox, oy) && !plan.locked[plan.idx(ox, oy)] {
                        plan.force(ox, oy, k.earth, PLAYER_NEUTRAL);
                    }
                }
            }
            // rooms in the corners of the courtyard
            let rooms: Vec<u16> = ["TREASURE", "GUARD_POST", "TRAINING", "BARRACKS"]
                .iter()
                .filter_map(|n| room_slab(ctx.gd, n))
                .collect();
            let inner = (fx1 - fx0 - 1) / 2;
            if inner >= 4 && !rooms.is_empty() {
                let corner = [
                    (fx0 + 2, fy0 + 2),
                    (fx1 - 4, fy0 + 2),
                    (fx0 + 2, fy1 - 4),
                    (fx1 - 4, fy1 - 4),
                ];
                for (ci, &(x0, y0)) in corner.iter().enumerate() {
                    let rk = rooms[ci % rooms.len()];
                    for y in y0..y0 + 3 {
                        for x in x0..x0 + 3 {
                            plan.force(x, y, rk, HEROES);
                        }
                    }
                }
            }
            // garrison
            if p.heroes {
                let models = creature_models(ctx.gd, &HERO_CREATURES);
                if !models.is_empty() {
                    // two seed spots and their images (all four sides without symmetry)
                    let base_m = rng.below(models.len() as u64) as usize;
                    let seeds: Vec<(i32, i32)> = if plan.sym.mode.is_none() && plan.sym.radial <= 1
                    {
                        vec![(-2, 0), (0, -2), (2, 0), (0, 2)]
                    } else {
                        vec![(-2, 0), (0, -2)]
                    };
                    let mut used: Vec<(i32, i32)> = vec![];
                    for (si, &(dx, dy)) in seeds.iter().enumerate() {
                        let m = models[(base_m + si) % models.len()];
                        let lvl = 2 + si as i64 % 3;
                        for q in plan.slab_images((round(cx) + dx, round(cy) + dy)) {
                            if !used.contains(&q) {
                                used.push(q);
                                plan.things.push(PlanThing {
                                    kind: PlanThingKind::Creature {
                                        model: m,
                                        level: lvl,
                                    },
                                    owner: HEROES,
                                    slab: q,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Gems, neutral chambers with rooms / creatures / boxes, hero gates.
pub(super) fn extras(ctx: &Ctx, plan: &mut Plan, rng: &mut Rng) {
    let k = ctx.k.clone();
    let p = ctx.p;
    let order = plan.sym.order().max(1);
    let per = |n: u8| (n as usize).div_ceil(order);
    let mut taken: Vec<(i32, i32)> = vec![];
    // hero gates: the centre orbit (or the fortress centre)
    if p.heroes {
        let (cx, cy) = plan.sym.centre();
        // the centre when it is its own image; else (even sizes) an orbit around it, so gates
        // are not packed side by side; radial maps get the single centre slab
        let c = (round(cx), round(cy));
        let mut gate_cells = plan.slab_images(c);
        if plan.sym.radial > 1 || gate_cells.len() == 1 {
            gate_cells = vec![c];
        } else if p.style != Style::Fortress {
            gate_cells = plan.slab_images((c.0, c.1 - 5));
        } else {
            gate_cells = plan.slab_images((c.0, c.1 - 1));
            gate_cells.retain(|q| plan.locked[plan.idx(q.0, q.1)]);
        }
        if p.hero_gates > 0 && (p.hero_gates as usize) < gate_cells.len() {
            gate_cells.truncate(p.hero_gates as usize);
        }
        if p.hero_gates as usize > gate_cells.len() {
            let more = feature_points(
                plan,
                rng,
                (p.hero_gates as usize - gate_cells.len()).div_ceil(order),
                14,
                8,
                &mut taken,
            );
            for q in more {
                gate_cells.extend(plan.slab_images(q));
            }
        }
        for (n, &(x, y)) in gate_cells.iter().enumerate() {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let i = plan.idx(x + dx, y + dy);
                    if plan.interior(x + dx, y + dy)
                        && (!plan.locked[i] || p.style == Style::Fortress)
                        && !sites::is_keeper(plan.owners[i])
                    {
                        let o = if p.style == Style::Fortress {
                            HEROES
                        } else {
                            PLAYER_NEUTRAL
                        };
                        let kk = if p.style == Style::Fortress {
                            k.claimed
                        } else {
                            k.path
                        };
                        plan.force(x + dx, y + dy, kk, o);
                    }
                }
            }
            plan.things.push(PlanThing {
                kind: PlanThingKind::HeroGate {
                    number: n as i64 + 1,
                },
                owner: HEROES,
                slab: (x, y),
            });
            taken.push((x, y));
        }
    }
    let fortress = |plan: &Plan, q: (i32, i32)| plan.locked[plan.idx(q.0, q.1)];
    // gems
    for q in feature_points(plan, rng, per(p.gems), 13, 8, &mut taken) {
        if fortress(plan, q) {
            continue;
        }
        let r = 1.0 + rng.below(2) as f64 * 0.5;
        plan.stamp(
            Shape::Disk {
                c: (q.0 as f64, q.1 as f64),
                r: r + 1.6,
            },
            k.earth,
            PLAYER_NEUTRAL,
            |kk| kk != k.portal,
        );
        plan.stamp(
            Shape::Disk {
                c: (q.0 as f64, q.1 as f64),
                r,
            },
            k.gems,
            PLAYER_NEUTRAL,
            |_| true,
        );
    }
    // neutral chambers
    let chambers = feature_points(plan, rng, per(p.neutral_rooms), 12, 9, &mut taken);
    let room_kinds: Vec<u16> = NEUTRAL_ROOMS
        .iter()
        .filter_map(|n| room_slab(ctx.gd, n))
        .collect();
    let creatures = creature_models(ctx.gd, &NEUTRAL_CREATURES);
    let boxes = special_boxes(ctx.gd);
    let (mut c_left, mut b_left) = (per(p.neutral_creatures), per(p.special_boxes));
    for (ci, &q) in chambers.iter().enumerate() {
        let c = (q.0 as f64, q.1 as f64);
        plan.stamp(Shape::Disk { c, r: 4.2 }, k.earth, PLAYER_NEUTRAL, |kk| {
            kk == k.rock || kk == k.lava || kk == k.water
        });
        plan.stamp(Shape::Disk { c, r: 2.9 }, k.path, PLAYER_NEUTRAL, |_| true);
        if ci % 2 == 0 && !room_kinds.is_empty() {
            let rk = room_kinds[rng.below(room_kinds.len() as u64) as usize];
            plan.stamp(
                Shape::Rect {
                    a: (c.0 - 1.0, c.1 - 1.0),
                    b: (c.0 + 1.0, c.1 + 1.0),
                },
                rk,
                PLAYER_NEUTRAL,
                |_| true,
            );
        }
        let left_here = chambers.len() - ci;
        let nc = c_left.div_ceil(left_here);
        c_left -= nc;
        let spots = [(-2, 0), (2, 0), (0, -2), (0, 2), (-1, -2), (1, 2)];
        for s in 0..nc.min(spots.len()) {
            if creatures.is_empty() {
                break;
            }
            let m = creatures[rng.below(creatures.len() as u64) as usize];
            let lvl = rng.range(1, 4) as i64;
            for img in plan.slab_images((q.0 + spots[s].0, q.1 + spots[s].1)) {
                plan.things.push(PlanThing {
                    kind: PlanThingKind::Creature {
                        model: m,
                        level: lvl,
                    },
                    owner: PLAYER_NEUTRAL,
                    slab: img,
                });
            }
        }
        let nb = b_left.div_ceil(left_here);
        b_left -= nb;
        for s in 0..nb.min(2) {
            if let Some(&m) = rng.pick(&boxes) {
                let off = [(-2, -2), (2, 2)][s];
                for img in plan.slab_images((q.0 + off.0, q.1 + off.1)) {
                    plan.things.push(PlanThing {
                        kind: PlanThingKind::Object { model: m },
                        owner: PLAYER_NEUTRAL,
                        slab: img,
                    });
                }
            }
        }
    }
}

pub(super) fn border(plan: &mut Plan, rock: u16) {
    for y in 0..plan.h {
        for x in 0..plan.w {
            if x == 0 || y == 0 || x == plan.w - 1 || y == plan.h - 1 {
                let i = plan.idx(x, y);
                plan.kinds[i] = rock;
                plan.owners[i] = PLAYER_NEUTRAL;
            }
        }
    }
}

/// Build one plan (no checks).
pub(super) fn build_plan(ctx: &Ctx, w: i32, h: i32, seed: u64) -> Result<Plan, String> {
    let p = ctx.p;
    let mut rng = Rng::new(seed);
    let sym = resolve_symmetry(p, w, h, &mut rng)?;
    let mut plan = Plan::new(w, h, ctx.k.earth, sym);
    let n = (p.players as usize).min(KEEPERS.len());
    let ring = match p.style {
        Style::Fortress => 0.72,
        Style::Islands => 0.66,
        _ => 0.64 + rng.below(60) as f64 / 1000.0,
    };
    place_bases(&mut plan, n, ring, &mut rng)?;
    let wl = |v: u8| (v.min(100) as u32 * 8) / 100;
    let gold = 2 + (p.richness.min(100) as u32 * 5) / 100;
    let (rock, cave) = match p.style {
        Style::OpenCave => (14, 7),
        Style::Chambers => (24, 0),
        Style::Islands => (6, 0),
        Style::River => (12, 2),
        Style::Fortress => (14, 3),
    };
    let (lava, water) = match p.style {
        Style::Islands | Style::River => (wl(p.lava) / 2, wl(p.water) / 2),
        _ => (wl(p.lava), wl(p.water)),
    };
    let mut r = rng.fork(1);
    background(ctx, &mut plan, &mut r, rock, gold, lava, water, cave);
    let mut r = rng.fork(2);
    style_features(ctx, &mut plan, &mut r);
    let mut r = rng.fork(3);
    stamp_bases(ctx, &mut plan, &mut r);
    let mut r = rng.fork(4);
    extras(ctx, &mut plan, &mut r);
    if ctx.border {
        border(&mut plan, ctx.k.rock);
    }
    plan.symmetrize();
    // things on solid slabs or outside are dropped (images may land on walls)
    let gd = ctx.gd;
    let mut keep = vec![];
    for t in std::mem::take(&mut plan.things) {
        if plan.open_at(gd, t.slab.0, t.slab.1) && plan.interior(t.slab.0, t.slab.1) {
            keep.push(t);
        }
    }
    plan.things = keep;
    // doors whose slab was overwritten are dropped
    let doors = std::mem::take(&mut plan.doors);
    plan.doors = doors
        .into_iter()
        .filter(|d| {
            crate::terrain::classify(gd, plan.kind(d.slab.0, d.slab.1) as usize) == Class::Door
        })
        .collect();
    Ok(plan)
}
