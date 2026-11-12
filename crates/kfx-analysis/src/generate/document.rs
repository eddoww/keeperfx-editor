//! Turning a plan into a [`MapDoc`], and the starter level script of a generated map.
use super::*;

/// Write a plan into `doc` at offset `(ox, oy)` (slab layer, torch dirt, columns, slab objects,
/// doors, things). `doc` must be large enough.
pub(super) fn write_plan(
    doc: &mut MapDoc,
    gd: &GameData,
    plan: &Plan,
    ox: i32,
    oy: i32,
    dec: &Decorator,
) {
    let d = doc.dims;
    let rect = SlabRect {
        x0: ox,
        y0: oy,
        x1: ox + plan.w - 1,
        y1: oy + plan.h - 1,
    }
    .clip(d);
    for y in 0..plan.h {
        for x in 0..plan.w {
            let (dx, dy) = (ox + x, oy + y);
            if !autotile::in_map(d, dx, dy) {
                continue;
            }
            doc.touch_slab(dx as usize, dy as usize);
            let i = d.slab_index(dx as usize, dy as usize);
            let k = plan.kind(x, y);
            doc.kinds[i] = k;
            doc.owners[i] = plan.owners[plan.idx(x, y)];
            doc.custom_cells[i] = 0;
            let s = gd.slab(k as usize);
            if s.wlb_type != autotile::WLB_BRIDGE {
                doc.wlb[i] = s.wlb_type;
            }
        }
    }
    // torch dirt (alter_rock_style src/map_blocks.c:1613), on the rect and its ring
    let ring = rect.grow(1).clip(d);
    let mut torch = vec![];
    {
        let v = doc.view(gd);
        for (x, y) in ring.iter() {
            let i = d.slab_index(x as usize, y as usize);
            let k = doc.kinds[i];
            if k == kinds::EARTH as u16 || k == kinds::TORCHDIRT as u16 {
                let n = autotile::alter_rock_style(&v, gd, kinds::EARTH, x, y) as u16;
                if n != k {
                    torch.push((x, y, n));
                }
            }
        }
    }
    for (x, y, n) in torch {
        doc.touch_slab(x as usize, y as usize);
        let i = d.slab_index(x as usize, y as usize);
        doc.kinds[i] = n;
    }
    doc.regenerate_rect_decorated(gd, dec, ring);
    doc.apply_slab_objects(gd, ring);
    for door in &plan.doors {
        let (x, y) = (door.slab.0 + ox, door.slab.1 + oy);
        if autotile::in_map(d, x, y) {
            let _ = doc.place_door(
                gd,
                x as usize,
                y as usize,
                door.model,
                door.owner,
                door.locked,
            );
        }
    }
    for t in &plan.things {
        let (sx, sy) = (3 * (t.slab.0 + ox) + 1, 3 * (t.slab.1 + oy) + 1);
        let z = doc.floor_z(sx, sy);
        let th = match t.kind {
            PlanThingKind::Creature { model, level } => {
                let mut th = kthings::new_thing(
                    class::CREATURE,
                    model,
                    t.owner,
                    kthings::centred(sx),
                    kthings::centred(sy),
                    z,
                );
                th.creature_level = Some(level.clamp(1, 10));
                th
            }
            PlanThingKind::Object { model } => kthings::new_thing(
                class::OBJECT,
                model,
                t.owner,
                kthings::centred(sx),
                kthings::centred(sy),
                z,
            ),
            PlanThingKind::HeroGate { number } => {
                let Some(m) = kthings::hero_gate_model(gd) else {
                    continue;
                };
                let mut th = kthings::new_thing(
                    class::OBJECT,
                    m,
                    HEROES,
                    kthings::centred(sx),
                    kthings::centred(sy),
                    z,
                );
                th.herogate_number = Some(number);
                th
            }
        };
        let _ = doc.add_thing(gd, th);
    }
}

/// The starter script values for a plan.
pub fn starter_script(gd: &GameData, plan: &Plan) -> StarterScript {
    let players = plan
        .bases
        .iter()
        .enumerate()
        .map(|(i, b)| PlayerSetup {
            owner: b.owner,
            script_name: script_player(b.owner).into(),
            start_money: 5000,
            max_creatures: 30,
            computer: (i > 0).then_some(0),
        })
        .collect();
    let pool = [
        "FLY",
        "BUG",
        "SPIDER",
        "DEMONSPAWN",
        "TROLL",
        "ORC",
        "SORCEROR",
        "BILE_DEMON",
        "DARK_MISTRESS",
        "DRAGON",
    ]
    .iter()
    .filter(|n| gd.creature_id(n).is_some())
    .map(|n| (n.to_string(), 10))
    .collect();
    let rooms = [
        "TREASURE",
        "LAIR",
        "GARDEN",
        "TRAINING",
        "RESEARCH",
        "WORKSHOP",
        "BARRACKS",
        "BRIDGE",
        "GUARD_POST",
        "PRISON",
        "TORTURE",
    ]
    .iter()
    .filter(|r| gd.room_id(r).is_some())
    .map(|r| {
        (
            r.to_string(),
            matches!(*r, "TREASURE" | "LAIR" | "GARDEN" | "TRAINING" | "RESEARCH"),
        )
    })
    .collect();
    let powers = vec![
        ("POWER_IMP".into(), true),
        ("POWER_SLAP".into(), true),
        ("POWER_SIGHT".into(), false),
        ("POWER_CALL_TO_ARMS".into(), false),
    ];
    let hero_gates = plan
        .things
        .iter()
        .filter_map(|t| {
            if let PlanThingKind::HeroGate { number } = t.kind {
                Some(number)
            } else {
                None
            }
        })
        .collect();
    StarterScript {
        generate_speed: 500,
        players,
        creature_pool: pool,
        rooms,
        powers,
        hero_gates,
    }
}
