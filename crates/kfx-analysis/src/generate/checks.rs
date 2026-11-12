//! Constraint checks of a plan through this crate's analyses, and repairs (dig corridors,
//! extra gold) of a plan that fails them.
use super::*;

/// Terrain and sites of a plan for the analyses.
pub fn plan_parts(
    plan: &Plan,
    gd: &GameData,
    rules: &Rules,
    table: &KindTable,
) -> (Terrain, Sites) {
    let doors = plan.doors.clone();
    let t = Terrain::from_grid(table, plan.w, plan.h, &plan.kinds, &plan.owners, |x, y| {
        doors.iter().any(|d| d.slab == (x, y) && d.locked)
    });
    let mut s = Sites::default();
    for b in &plan.bases {
        let mut cells = vec![];
        for dy in -1..=1 {
            for dx in -1..=1 {
                cells.push((b.heart.0 + dx, b.heart.1 + dy));
            }
        }
        s.hearts.push(Heart {
            owner: b.owner,
            slab: b.heart,
            cells,
        });
    }
    for th in &plan.things {
        match th.kind {
            PlanThingKind::HeroGate { number } => s.gates.push(Gate {
                number,
                slab: th.slab,
                thing: None,
            }),
            PlanThingKind::Creature { model, level } => {
                let name = gd
                    .creatures
                    .get((model - 1).max(0) as usize)
                    .cloned()
                    .unwrap_or_default();
                s.things.push(ThingSite {
                    thing: None,
                    owner: th.owner,
                    slab: th.slab,
                    kind: ThingKind::Creature {
                        model,
                        name,
                        level,
                        score: creature_score(gd, rules, model, level),
                    },
                });
            }
            PlanThingKind::Object { model } => {
                let name = gd
                    .objects_cfg
                    .get(&format!("object{model}"), "Name")
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let genre = gd
                    .object_genre(model as usize)
                    .unwrap_or("")
                    .trim()
                    .to_ascii_uppercase();
                s.things.push(ThingSite {
                    thing: None,
                    owner: th.owner,
                    slab: th.slab,
                    kind: ThingKind::Pickup { genre, name },
                });
            }
        }
    }
    let (_, portal_room) = sites::special_rooms(gd);
    s.portals = sites::portals_of(&t, portal_room);
    (t, s)
}

/// Constraint checks of an analysis (see the module docs).
pub fn constraint_checks(a: &Analysis, p: &GenParams) -> Vec<Check> {
    let mut out = vec![];
    let keepers = a.sites.keepers();
    let mut add = |name: String, ok: bool, detail: String| out.push(Check { name, ok, detail });
    add(
        "keeper count".into(),
        keepers.len() == p.players as usize,
        format!(
            "{} keepers with a heart, {} wanted",
            keepers.len(),
            p.players
        ),
    );
    for f in &a.fields {
        let pl = f.player;
        let nm = sites::owner_name(pl);
        let e = a.economy.players.iter().find(|e| e.player == pl);
        let portal = e
            .and_then(|e| e.portals.first())
            .filter(|d| d.turns != crate::nav::INF);
        add(
            format!("{nm} portal"),
            portal.is_some(),
            portal
                .map(|d| format!("nearest portal {} turns away", d.turns))
                .unwrap_or_else(|| "no portal reachable by digging".into()),
        );
        let gold = e.map(|e| e.gold_value + e.loose_gold).unwrap_or(0);
        add(
            format!("{nm} gold"),
            gold >= p.min_gold_value(),
            format!("{gold} gold reachable first, {} wanted", p.min_gold_value()),
        );
        for g in &a.fields {
            if g.player > pl {
                let ok = f.dig.nearest(&a.sites.heart_cells(g.player)).is_some();
                add(
                    format!("{nm} to {}", sites::owner_name(g.player)),
                    ok,
                    if ok {
                        "dig route".into()
                    } else {
                        "no dig route".into()
                    },
                );
            }
        }
        if p.heroes && !a.sites.gates.is_empty() {
            let ok = a
                .sites
                .gates
                .iter()
                .any(|g| f.dig.at(g.slab.0, g.slab.1) != crate::nav::INF);
            add(
                format!("{nm} hero gate"),
                ok,
                if ok {
                    "a gate is reachable".into()
                } else {
                    "no hero gate reachable".into()
                },
            );
        }
    }
    if p.heroes {
        add(
            "hero gates".into(),
            !a.sites.gates.is_empty(),
            format!("{} hero gates", a.sites.gates.len()),
        );
    }
    if let Some(fr) = &a.fairness {
        add(
            "fairness".into(),
            fr.score <= p.max_imbalance,
            format!(
                "imbalance {:.3} (max {:.3}); {}",
                fr.score,
                p.max_imbalance,
                fr.top(3).join("; ")
            ),
        );
    }
    out
}

/// Analyse a plan and check it.
pub fn check_plan(
    plan: &Plan,
    gd: &GameData,
    rules: &Rules,
    table: &KindTable,
    p: &GenParams,
) -> (Analysis, Vec<Check>) {
    let (t, s) = plan_parts(plan, gd, rules, table);
    let a = Analysis::from_parts(
        gd,
        rules.clone(),
        table.clone(),
        t,
        s,
        Options {
            symmetry: false,
            heroes: true,
        },
    );
    let c = constraint_checks(&a, p);
    (a, c)
}

/// Carve an earth corridor between two slabs (rock, gems and liquids become earth; images too).
pub(super) fn carve(plan: &mut Plan, k: &Kinds, a: (i32, i32), b: (i32, i32)) {
    let s = Shape::Capsule {
        a: (a.0 as f64, a.1 as f64),
        b: (b.0 as f64, b.1 as f64),
        r: 0.8,
    };
    let (rock, gems, lava, water, earth) = (k.rock, k.gems, k.lava, k.water, k.earth);
    for (x, y) in plan.stamp_cells(s) {
        let i = plan.idx(x, y);
        let kk = plan.kinds[i];
        let blocked = kk == rock
            || kk == gems
            || kk == lava
            || kk == water
            || (plan.owners[i] == HEROES && !plan.locked[i]);
        if blocked && plan.interior(x, y) && !plan.locked[i] {
            plan.kinds[i] = earth;
            plan.owners[i] = PLAYER_NEUTRAL;
        }
    }
}

/// Repairs for the failed checks; returns what was done.
pub(super) fn repair(
    plan: &mut Plan,
    k: &Kinds,
    a: &Analysis,
    checks: &[Check],
    p: &GenParams,
    with_border: bool,
) -> Vec<String> {
    let mut done = vec![];
    let all_bases = plan.bases.clone();
    let base = |o: u8| all_bases.iter().find(|b| b.owner == o).copied();
    for c in checks.iter().filter(|c| !c.ok) {
        for b in plan.bases.clone() {
            let nm = sites::owner_name(b.owner);
            if c.name == format!("{nm} portal") {
                if let Some(pp) = b.portal {
                    carve(plan, k, b.heart, pp);
                    done.push(format!("carved {nm} heart to portal"));
                }
            }
            if c.name == format!("{nm} gold") {
                // a gold blob behind the heart
                let (cx, cy) = plan.sym.centre();
                let (dx, dy) = (b.heart.0 as f64 - cx, b.heart.1 as f64 - cy);
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let g = (
                    b.heart.0 as f64 + dx / len * 6.0,
                    b.heart.1 as f64 + dy / len * 6.0,
                );
                let n = (p.min_gold_value() / 1024 + 1) as f64;
                let (earth, gold) = (k.earth, k.gold);
                plan.stamp(
                    Shape::Disk {
                        c: g,
                        r: (n / 3.0).sqrt() + 0.5,
                    },
                    gold,
                    PLAYER_NEUTRAL,
                    |kk| kk == earth,
                );
                done.push(format!("added gold near {nm}"));
            }
            if c.name == format!("{nm} hero gate") {
                if let Some(g) = a
                    .sites
                    .gates
                    .iter()
                    .min_by_key(|g| (g.slab.0 - b.heart.0).abs() + (g.slab.1 - b.heart.1).abs())
                {
                    carve(plan, k, b.heart, g.slab);
                    done.push(format!("carved {nm} to hero gate {}", g.number));
                }
            }
            for o in KEEPERS {
                if c.name == format!("{nm} to {}", sites::owner_name(o)) {
                    if let Some(ob) = base(o) {
                        carve(plan, k, b.heart, ob.heart);
                        done.push(format!("carved {nm} to {}", sites::owner_name(o)));
                    }
                }
            }
        }
        if c.name == "fairness" {
            // give gold to the poorest when gold is the largest difference
            if let Some(fr) = &a.fairness {
                if fr.differences.first().is_some_and(|d| d.metric == "gold") {
                    if let (Some(lo), Some(hi)) = (
                        fr.rows.iter().min_by_key(|r| r.gold),
                        fr.rows.iter().max_by_key(|r| r.gold),
                    ) {
                        if let Some(b) = base(lo.player) {
                            let missing = (hi.gold - lo.gold) / 1024 + 1;
                            let (cx, cy) = plan.sym.centre();
                            let (dx, dy) = (b.heart.0 as f64 - cx, b.heart.1 as f64 - cy);
                            let len = (dx * dx + dy * dy).sqrt().max(1.0);
                            let g = (
                                b.heart.0 as f64 + dx / len * 6.0 - dy / len * 3.0,
                                b.heart.1 as f64 + dy / len * 6.0 + dx / len * 3.0,
                            );
                            let (earth, gold) = (k.earth, k.gold);
                            for (x, y) in plan.cells(Shape::Disk {
                                c: g,
                                r: (missing as f64 / 3.0).sqrt() + 0.7,
                            }) {
                                if plan.kind(x, y) == earth {
                                    plan.set(x, y, gold, PLAYER_NEUTRAL);
                                }
                            }
                            done.push(format!(
                                "added {missing} gold slabs near {}",
                                sites::owner_name(lo.player)
                            ));
                        }
                    }
                }
            }
        }
    }
    if with_border {
        border(plan, k.rock);
    }
    plan.symmetrize();
    done
}
