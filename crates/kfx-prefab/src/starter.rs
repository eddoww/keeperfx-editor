//! The starter prefab set shipped under `data/prefabs/`.
//!
//! Every piece is authored here as an ASCII sketch (a legend turns characters into slabs, doors
//! and bridges) plus a list of things, action points and a script fragment. [`build`] lays the
//! sketch out on a scratch map with the real document API (`place_slab`, `place_door`,
//! `apply_slab_objects`, `add_thing`), then *captures* it with [`crate::capture::capture`], so
//! the shipped files are exactly what the editor itself would save for such a layout. The
//! generator is the example `make_starter`.
//!
//! Legend: `.` not part of the prefab; `#` rock, `:` earth, `$` gold, `%` dense gold, `*` gems,
//! `_` unclaimed path, `-` claimed floor of player A, `=` of B, `q` hero floor; `w` / `v` / `W`
//! wall of A / B / the hero player (a style is picked per position, and a wall next to a room
//! takes the room's wall style); rooms of A: `H` heart `T` treasury `L` lair `Y` hatchery
//! `B` library `N` training `K` workshop `P` prison `Q` torture `M` temple `R` graveyard
//! `S` scavenger `U` barracks `u` guard post, `J` heart of B, `Z` portal zone; `~` water `^` lava
//! `` ` `` abyss, `b` / `n` bridge over water / lava; doors: `d` wooden, `e` braced, `f` steel,
//! `a` magic (upper case = locked), `k` secret, `i` midas.
use crate::capture::{capture, CaptureOptions, Selection};
use crate::model::*;
use crate::{Error, Result};
use kfx_formats::level_bin::MapDims;
use kfx_formats::level_fx::StlCoord;
use kfx_map::autotile::SlabRect;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class};

/// A thing of a sketch. Positions are slab units from the sketch's top-left (centre = +0.5).
pub struct ThSpec {
    pub class: ThingClass,
    pub model: &'static str,
    /// Player number (0 = A, 1 = B, 4 = hero, 5 = neutral).
    pub owner: u8,
    pub x: f32,
    pub y: f32,
    pub level: i64,
    /// Hero gate placeholder key.
    pub gate: Option<&'static str>,
}

fn at(c: ThingClass, model: &'static str, owner: u8, x: i32, y: i32) -> ThSpec {
    ThSpec {
        class: c,
        model,
        owner,
        x: x as f32 + 0.5,
        y: y as f32 + 0.5,
        level: 1,
        gate: None,
    }
}
fn creature(model: &'static str, owner: u8, x: i32, y: i32, level: i64) -> ThSpec {
    ThSpec {
        level,
        ..at(ThingClass::Creature, model, owner, x, y)
    }
}
fn object(model: &'static str, owner: u8, x: i32, y: i32) -> ThSpec {
    at(ThingClass::Object, model, owner, x, y)
}
fn trap(model: &'static str, x: i32, y: i32) -> ThSpec {
    at(ThingClass::Trap, model, 0, x, y)
}
fn gate(key: &'static str, x: i32, y: i32) -> ThSpec {
    ThSpec {
        gate: Some(key),
        ..at(ThingClass::Object, "HERO_GATE", 4, x, y)
    }
}

/// One starter piece.
pub struct Spec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub tags: &'static [&'static str],
    pub rows: Vec<String>,
    pub things: Vec<ThSpec>,
    /// `(key, x, y, range in subtiles)` of action points.
    pub aps: Vec<(&'static str, i32, i32, i32)>,
    /// Owner (player number) of the doors.
    pub door_owner: u8,
    pub script_dk: Option<&'static str>,
}

fn rows(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn spec(
    id: &'static str,
    name: &'static str,
    tags: &'static [&'static str],
    description: &'static str,
    r: Vec<String>,
) -> Spec {
    Spec {
        id,
        name,
        description,
        tags,
        rows: r,
        things: vec![],
        aps: vec![],
        door_owner: 0,
        script_dk: None,
    }
}

// ---------------------------------------------------------------------------------------------
// generators

fn arena() -> Vec<String> {
    let n = 13i32;
    (0..n)
        .map(|y| {
            (0..n)
                .map(|x| {
                    let (dx, dy) = ((x - 6) as f32, (y - 6) as f32);
                    let d = (dx * dx + dy * dy).sqrt();
                    if x == 6 && y >= 9 {
                        '_'
                    } else if d <= 5.4 {
                        if dx.abs() >= 3.0 && dy.abs() >= 3.0 {
                            '^'
                        } else {
                            '_'
                        }
                    } else if d <= 6.6 {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

fn maze() -> Vec<String> {
    let n = 11usize;
    let mut g = vec![vec!['#'; n]; n];
    let mut seed = 0x2545_F491u32;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 16) as usize
    };
    let mut stack = vec![(1usize, 1usize)];
    g[1][1] = '_';
    while let Some(&(x, y)) = stack.last() {
        let mut opts = vec![];
        for (dx, dy) in [(0i32, -2i32), (2, 0), (0, 2), (-2, 0)] {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx > 0
                && ny > 0
                && (nx as usize) < n - 1
                && (ny as usize) < n - 1
                && g[ny as usize][nx as usize] == '#'
            {
                opts.push((nx as usize, ny as usize));
            }
        }
        if opts.is_empty() {
            stack.pop();
            continue;
        }
        let (nx, ny) = opts[rnd() % opts.len()];
        g[(y + ny) / 2][(x + nx) / 2] = '_';
        g[ny][nx] = '_';
        stack.push((nx, ny));
    }
    g[1][0] = '_';
    g[9][10] = '_';
    g[5][5] = '$';
    g.into_iter().map(|r| r.into_iter().collect()).collect()
}

// ---------------------------------------------------------------------------------------------
// the set

/// The sketches of the starter set.
pub fn specs() -> Vec<Spec> {
    let mut v = vec![];

    // ---- dungeon starts
    let mut s = spec(
        "start-compact",
        "Dungeon start: compact",
        &["dungeon", "start", "heart"],
        "A dungeon heart in a small walled hall with two imps and \
            gold seams in the corners. Owner slot A.",
        rows(
            "
            $:::::::::$
            :wwwwwwwww:
            :w-------w:
            :w--HHH--w:
            :w--HHH--w:
            :w--HHH--w:
            :w-------w:
            :wwwwwwwww:
            $:::::::::$
            ",
        ),
    );
    s.things = vec![creature("IMP", 0, 3, 2, 1), creature("IMP", 0, 7, 6, 1)];
    v.push(s);

    let mut s = spec(
        "start-gateway",
        "Dungeon start: hall with exits",
        &["dungeon", "start", "heart"],
        "A heart hall with an open doorway north and south, paths \
            leading out, three imps. Owner slot A.",
        rows(
            "
            ::::::_::::::
            :wwwww-wwwww:
            :w---------w:
            :w--HHH----w:
            :w--HHH----w:
            :w--HHH----w:
            :w---------w:
            :wwwww-wwwww:
            ::::::_::::::
            ",
        ),
    );
    s.things = vec![
        creature("IMP", 0, 2, 2, 1),
        creature("IMP", 0, 9, 3, 1),
        creature("IMP", 0, 6, 6, 1),
    ];
    v.push(s);

    let mut s = spec(
        "start-fortress",
        "Dungeon start: fortress",
        &["dungeon", "start", "heart", "doors"],
        "A big heart hall with pillars, a wooden door north and a \
            braced door south, four imps. Owner slot A.",
        rows(
            "
            :$::::::_::::::$:
            :wwwwwwwdwwwwwww:
            :w-------------w:
            :w-#---------#-w:
            :w-------------w:
            :w-----HHH-----w:
            :w-----HHH-----w:
            :w-----HHH-----w:
            :w-------------w:
            :w-#---------#-w:
            :w-------------w:
            :wwwwwwwewwwwwww:
            ::::::::_::::::::
            ",
        ),
    );
    s.things = vec![
        creature("IMP", 0, 3, 2, 1),
        creature("IMP", 0, 13, 2, 1),
        creature("IMP", 0, 3, 10, 1),
        creature("IMP", 0, 13, 10, 1),
    ];
    v.push(s);

    // ---- vault
    let mut s = spec(
        "treasure-vault",
        "Treasure vault with traps",
        &["treasury", "traps", "doors", "vault"],
        "A nine by three treasury full of gold hoards behind a \
            locked steel door, a trapped guard corridor and a wooden door to the outside.",
        rows(
            "
            :::::::::::::
            :wwwwwwwwwww:
            :wTTTTTTTTTw:
            :wTTTTTTTTTw:
            :wTTTTTTTTTw:
            :wwwwwFwwwww:
            :w---------w:
            :w---------w:
            :wwwwwdwwwww:
            ::::::_::::::
            ",
        ),
    );
    for y in 2..=4 {
        for x in 2..=10 {
            s.things.push(object("GOLD_HOARD_3", 0, x, y));
        }
    }
    s.things.extend([
        trap("BOULDER", 6, 6),
        trap("POISON_GAS", 3, 7),
        trap("POISON_GAS", 9, 7),
        trap("ALARM", 2, 6),
    ]);
    v.push(s);

    // ---- heroes
    let mut s = spec(
        "hero-watchtower",
        "Hero outpost: watchtower",
        &["hero", "outpost", "gate"],
        "A small walled hero post with a hero gate, a knight, an \
            archer and a wizard. Hero owned (player 4); the gate number is renumbered on apply.",
        rows(
            "
            .........
            .WWWWWWW.
            .WqqqqqW.
            .WqqqqqW.
            .WqqqqqW.
            .WWWqWWW.
            ....q....
            ",
        ),
    );
    s.things = vec![
        gate("gate", 4, 3),
        creature("KNIGHT", 4, 2, 2, 3),
        creature("ARCHER", 4, 6, 2, 3),
        creature("WIZARD", 4, 4, 4, 2),
        object("GUARDFLAG_WHITE", 4, 2, 4),
    ];
    v.push(s);

    let mut s = spec(
        "hero-camp",
        "Hero outpost: camp",
        &["hero", "outpost", "gate", "open"],
        "An open hero camp in a rock clearing: a hero gate, \
            barbarians, monks and a dwarf around a flag. Hero owned (player 4).",
        rows(
            "
            ...#####...
            ..#qqqqq#..
            .#qqqqqqq#.
            #qqqqqqqqq#
            #qqqqqqqqq#
            #qqqqqqqqq#
            .#qqqqqqq#.
            ..#qqqqq#..
            ...##q##...
            ",
        ),
    );
    s.things = vec![
        gate("gate", 5, 4),
        object("GUARDFLAG_WHITE", 4, 5, 2),
        creature("BARBARIAN", 4, 3, 3, 3),
        creature("BARBARIAN", 4, 7, 3, 3),
        creature("MONK", 4, 3, 5, 2),
        creature("MONK", 4, 7, 5, 2),
        creature("DWARFA", 4, 5, 6, 2),
    ];
    v.push(s);

    let mut s = spec(
        "hero-fortress",
        "Hero fortress keep",
        &["hero", "fortress", "gate", "doors"],
        "A walled keep with an outer steel gate, an inner wooden \
            door and a ring corridor. The hero gate stands in the keep, guards in the \
            courtyard. Hero owned (player 4).",
        rows(
            "
            .WWWWWWWWWWWWW.
            .WqqqqqqqqqqqW.
            .WqWWWWWWWWWqW.
            .WqWqqqqqqqWqW.
            .WqWqqqqqqqWqW.
            .WqWqqqqqqqWqW.
            .WqWWWWdWWWWqW.
            .WqqqqqqqqqqqW.
            .WqqqqqqqqqqqW.
            .WWWWWWfWWWWWW.
            .......q.......
            ",
        ),
    );
    s.door_owner = 4;
    s.things = vec![
        gate("gate", 7, 4),
        creature("ARCHER", 4, 5, 3, 4),
        creature("ARCHER", 4, 9, 3, 4),
        creature("MONK", 4, 7, 5, 3),
        creature("KNIGHT", 4, 3, 7, 5),
        creature("KNIGHT", 4, 11, 7, 5),
        creature("BARBARIAN", 4, 7, 8, 4),
        creature("WIZARD", 4, 2, 3, 3),
        creature("WIZARD", 4, 12, 3, 3),
        object("GUARDFLAG_WHITE", 4, 5, 7),
        object("GUARDFLAG_WHITE", 4, 9, 7),
    ];
    v.push(s);

    // ---- gems
    let mut s = spec(
        "gem-chamber",
        "Guarded gem chamber",
        &["gems", "secret", "guards", "traps"],
        "A rock vault lined with gems behind a secret door, guarded \
            by two trolls and a demonspawn on a lightning trap.",
        rows(
            "
            :::::::::::
            :#########:
            :#*******#:
            :#_______#:
            :#_______#:
            :#_______#:
            :####k####:
            :::::_:::::
            :::::::::::
            ",
        ),
    );
    s.things = vec![
        creature("TROLL", 5, 3, 4, 3),
        creature("TROLL", 5, 7, 4, 3),
        creature("DEMONSPAWN", 5, 5, 3, 4),
        trap("LIGHTNING", 5, 5),
    ];
    v.push(s);

    // ---- keeper rooms
    v.push({
        let mut s = spec(
            "prison-torture-wing",
            "Prison and torture wing",
            &["prison", "torture", "rooms", "doors"],
            "A prison and a torture chamber side by side, each with its \
                own door onto a shared corridor, one door to the outside.",
            rows(
                "
                :::::::::::::::
                :wwwwwwwwwwwww:
                :wPPPPPwQQQQQw:
                :wPPPPPwQQQQQw:
                :wPPPPPwQQQQQw:
                :wwwwdwwwwdwww:
                :w-----------w:
                :wwwwwwdwwwwww:
                :::::::_:::::::
                ",
            ),
        );
        s.things = vec![];
        s
    });
    v.push(spec(
        "library",
        "Library",
        &["library", "rooms", "doors"],
        "A five by four library with one door.",
        rows(
            "
            :::::::::
            :wwwwwww:
            :wBBBBBw:
            :wBBBBBw:
            :wBBBBBw:
            :wBBBBBw:
            :wwwdwww:
            ::::_::::
            ",
        ),
    ));
    v.push(spec(
        "lair-hatchery-block",
        "Lairs and hatchery block",
        &["lair", "hatchery", "rooms", "doors"],
        "Two lairs behind doors and a long hatchery along a claimed corridor, one door south.",
        rows(
            "
            :::::::::::::
            :wwwwwwwwwww:
            :wLLLw-YYYYw:
            :wLLLd-YYYYw:
            :wLLLw-YYYYw:
            :wwwww-YYYYw:
            :wLLLw-YYYYw:
            :wLLLd-YYYYw:
            :wLLLw-YYYYw:
            :wwwwwdwwwww:
            ::::::_::::::
            ",
        ),
    ));
    v.push(spec(
        "training-hall",
        "Training hall",
        &["training", "rooms", "doors"],
        "A five by five training room with a braced door.",
        rows(
            "
            :::::::::
            :wwwwwww:
            :wNNNNNw:
            :wNNNNNw:
            :wNNNNNw:
            :wNNNNNw:
            :wNNNNNw:
            :wwwewww:
            ::::_::::
            ",
        ),
    ));
    v.push(spec(
        "workshop-forge",
        "Workshop",
        &["workshop", "rooms", "doors"],
        "A five by four workshop with a wooden door.",
        rows(
            "
            :::::::::
            :wwwwwww:
            :wKKKKKw:
            :wKKKKKw:
            :wKKKKKw:
            :wKKKKKw:
            :wwwdwww:
            ::::_::::
            ",
        ),
    ));
    v.push(spec(
        "temple",
        "Temple",
        &["temple", "rooms", "doors"],
        "A five by five temple pool with a door.",
        rows(
            "
            :::::::::
            :wwwwwww:
            :wMMMMMw:
            :wMMMMMw:
            :wMMMMMw:
            :wMMMMMw:
            :wMMMMMw:
            :wwwdwww:
            ::::_::::
            ",
        ),
    ));
    v.push(spec(
        "graveyard",
        "Graveyard",
        &["graveyard", "rooms", "doors"],
        "A five by four graveyard with a door.",
        rows(
            "
            :::::::::
            :wwwwwww:
            :wRRRRRw:
            :wRRRRRw:
            :wRRRRRw:
            :wRRRRRw:
            :wwwdwww:
            ::::_::::
            ",
        ),
    ));
    v.push(spec(
        "barracks-guardpost",
        "Barracks and guard post",
        &["barracks", "guard", "rooms", "doors"],
        "A barracks with a guard post row in front of its door.",
        rows(
            "
            :::::::::::
            :wwwwwwwww:
            :wUUUUUUUw:
            :wUUUUUUUw:
            :wwwwdwwww:
            :wuuuuuuuw:
            :wwwwdwwww:
            :::::_:::::
            ",
        ),
    ));

    // ---- terrain
    let mut s = spec(
        "bridge-water",
        "Bridge over a river",
        &["bridge", "water", "terrain"],
        "A three slab wide river crossed by a one slab bridge between two paved banks.",
        rows(
            "
            #########
            #_______#
            #_______#
            ~~~~b~~~~
            ~~~~b~~~~
            ~~~~b~~~~
            #_______#
            #_______#
            #########
            ",
        ),
    );
    s.things = vec![];
    v.push(s);
    v.push(spec(
        "bridge-lava",
        "Bridge over lava",
        &["bridge", "lava", "terrain"],
        "A lava stream crossed by a bridge, with rock banks on both sides.",
        rows(
            "
            #########
            #_______#
            #_______#
            ^^^^n^^^^
            ^^^^n^^^^
            ^^^^n^^^^
            #_______#
            #_______#
            #########
            ",
        ),
    ));
    let mut s = spec(
        "portal-clearing",
        "Portal clearing",
        &["portal", "entrance", "terrain"],
        "A rock cavern with a three by three creature portal and openings north and south.",
        rows(
            "
            ..##_##..
            .#_____#.
            #_______#
            #__ZZZ__#
            #__ZZZ__#
            #__ZZZ__#
            #_______#
            .#_____#.
            ..##_##..
            ",
        ),
    );
    s.things = vec![];
    v.push(s);
    let mut s = spec(
        "boss-arena",
        "Boss arena",
        &["boss", "arena", "lava", "creatures"],
        "A round arena with lava in its corners, four lava pillars \
            and a dragon in the middle; one way in from the south.",
        arena(),
    );
    s.things = vec![
        creature("DRAGON", 5, 6, 6, 8),
        object("LAVA_PILLAR", 5, 4, 4),
        object("LAVA_PILLAR", 5, 8, 4),
        object("LAVA_PILLAR", 5, 4, 8),
        object("LAVA_PILLAR", 5, 8, 8),
    ];
    v.push(s);
    let mut s = spec(
        "maze-segment",
        "Maze segment",
        &["maze", "terrain", "gold"],
        "An eleven by eleven rock maze with an entrance west and an \
            exit east and a gold seam in the middle.",
        maze(),
    );
    s.things = vec![];
    v.push(s);

    // ---- neutral dens
    let mut s = spec(
        "den-spiders",
        "Neutral den: spiders",
        &["neutral", "den", "creatures"],
        "A small cave with three wild spiders and two bags of gold, open to the south.",
        rows(
            "
            ...:::...
            .::___::.
            .:_____:.
            :_______:
            :_______:
            :_______:
            .:_____:.
            .::___::.
            ...:_:...
            ",
        ),
    );
    s.things = vec![
        creature("SPIDER", 5, 3, 3, 2),
        creature("SPIDER", 5, 5, 4, 2),
        creature("SPIDER", 5, 4, 2, 3),
        object("GOLD_BAG", 5, 2, 4),
        object("GOLD_BAG", 5, 6, 3),
    ];
    v.push(s);
    let mut s = spec(
        "den-orc-camp",
        "Neutral den: orc camp",
        &["neutral", "den", "creatures"],
        "A rock camp with three wild orcs and some barrels.",
        rows(
            "
            ..#######..
            .#_______#.
            #_________#
            #_________#
            #_________#
            #_________#
            .#_______#.
            ..###_###..
            ",
        ),
    );
    s.things = vec![
        creature("ORC", 5, 3, 3, 3),
        creature("ORC", 5, 7, 3, 3),
        creature("ORC", 5, 5, 5, 4),
        object("BARREL", 5, 2, 2),
        object("BARREL", 5, 8, 5),
        object("BARREL", 5, 3, 5),
    ];
    v.push(s);

    // ---- ambush
    let mut s = spec(
        "ambush-corridor",
        "Ambush corridor",
        &["ambush", "script", "action-point", "hero", "gate"],
        "A claimed corridor with an action point trigger in the \
            middle and a hidden hero alcove behind a secret door. When a creature of player \
            A reaches the trigger, a hero party pours out of the hero gate (script fragment included).",
        rows(
            "
            :::::::::::::::::::
            :::::::wwwww:::::::
            :::::::wqqqw:::::::
            :wwwwwwwwkwwwwwwww:
            _-----------------_
            :wwwwwwwwwwwwwwwww:
            :::::::::::::::::::
            ",
        ),
    );
    s.things = vec![gate("gate", 9, 2)];
    s.aps = vec![("trigger", 9, 4, 3)];
    s.script_dk = Some(
        "REM Ambush corridor: heroes leave the gate when a creature of the keeper reaches the trigger
CREATE_PARTY(AMBUSH{{ap.trigger}})
ADD_TO_PARTY(AMBUSH{{ap.trigger}}, KNIGHT, 3, 0, ATTACK_ENEMIES, 0)
ADD_TO_PARTY(AMBUSH{{ap.trigger}}, ARCHER, 3, 0, ATTACK_ENEMIES, 0)
ADD_TO_PARTY(AMBUSH{{ap.trigger}}, WIZARD, 2, 0, ATTACK_ENEMIES, 0)
IF_ACTION_POINT({{ap.trigger}}, {{player.A}})
\tADD_PARTY_TO_LEVEL(PLAYER_GOOD, AMBUSH{{ap.trigger}}, -{{gate.gate}}, 1)
ENDIF
",
    );
    v.push(s);

    // ---- secret box room
    let mut s = spec(
        "secret-box-room",
        "Secret special-box room",
        &["secret", "special", "treasure"],
        "A rock room behind a secret door holding five special \
            boxes: reveal map, hidden world, increase level, resurrect creature and heal all.",
        rows(
            "
            :::::::::
            :#######:
            :#_____#:
            :#_____#:
            :#_____#:
            :###k###:
            ::::_::::
            ",
        ),
    );
    s.things = vec![
        object("SPECBOX_REVMAP", 5, 2, 2),
        object("SPECBOX_HIDNWRL", 5, 3, 2),
        object("SPECBOX_INCLEV", 5, 4, 3),
        object("SPECBOX_RESURCT", 5, 5, 2),
        object("SPECBOX_HEALALL", 5, 6, 2),
    ];
    // the room is the three slabs inside; positions above are on the 5x3 floor (x 2..6, y 2..4)
    v.push(s);

    // ---- treasure island
    let mut s = spec(
        "treasure-island",
        "Treasure island",
        &["treasure", "lava", "island", "bridge"],
        "A path island in a lava lake with gold bags and a \
            spellbook, reached by a bridge from the south.",
        rows(
            "
            ...#######...
            ..#^^^^^^^#..
            .#^^^^^^^^^#.
            #^^^^___^^^^#
            #^^^^___^^^^#
            #^^^^___^^^^#
            #^^^^^n^^^^^#
            #^^^^^n^^^^^#
            .#^^^^n^^^^#.
            ..#_______#..
            ...###_###...
            ",
        ),
    );
    s.things = vec![
        object("GOLD_BAG", 5, 5, 3),
        object("GOLD_BAG", 5, 7, 4),
        object("SPELLBOOK_SOE", 5, 6, 4),
    ];
    v.push(s);

    v
}

// ---------------------------------------------------------------------------------------------
// building

enum Glyph {
    Slab(&'static str, u8),
    Wall(u8),
    Door(i64, bool),
    Bridge(u8),
}

fn legend(c: char) -> Option<Glyph> {
    use Glyph::*;
    Some(match c {
        '#' => Slab("HARD", 5),
        ':' => Slab("DIRT", 5),
        '$' => Slab("GOLD", 5),
        '%' => Slab("DENSE_GOLD", 5),
        '*' => Slab("GEMS", 5),
        '_' => Slab("PATH", 5),
        '-' => Slab("PRETTY_PATH", 0),
        '=' => Slab("PRETTY_PATH", 1),
        'q' => Slab("PRETTY_PATH", 4),
        'w' => Wall(0),
        'v' => Wall(1),
        'W' => Wall(4),
        'H' => Slab("HEART_PEDESTAL", 0),
        'J' => Slab("HEART_PEDESTAL", 1),
        'T' => Slab("TREASURY_AREA", 0),
        'L' => Slab("LAIR_AREA", 0),
        'Y' => Slab("HATCHERY", 0),
        'B' => Slab("BOOK_SHELVES", 0),
        'N' => Slab("TRAINING_AREA", 0),
        'K' => Slab("WORKSHOP_AREA", 0),
        'P' => Slab("PRISON_AREA", 0),
        'Q' => Slab("TORTURE_AREA", 0),
        'M' => Slab("TEMPLE_POOL", 0),
        'R' => Slab("GRAVE_AREA", 0),
        'S' => Slab("SCAVENGE_AREA", 0),
        'U' => Slab("BARRACK_AREA", 0),
        'u' => Slab("GUARD_AREA", 0),
        'Z' => Slab("ENTRANCE_ZONE", 5),
        '~' => Slab("WATER", 5),
        '^' => Slab("LAVA", 5),
        '`' => Slab("ABYSS", 5),
        'b' => Bridge(2),
        'n' => Bridge(1),
        'd' => Door(1, false),
        'D' => Door(1, true),
        'e' => Door(2, false),
        'E' => Door(2, true),
        'f' => Door(3, false),
        'F' => Door(3, true),
        'a' => Door(4, false),
        'A' => Door(4, true),
        'k' => Door(5, false),
        'i' => Door(6, false),
        _ => return None,
    })
}

/// Wall style by position: mostly plain, some pairs, rare torches.
fn wall_kind(x: usize, y: usize) -> &'static str {
    const STYLES: [&str; 11] = [
        "PAIR_WALL",
        "PAIR_WALL",
        "TWINS_WALL",
        "PAIR_WALL",
        "WOMAN_WALL",
        "PAIR_WALL",
        "DRAPE_WALL",
        "TWINS_WALL",
        "PAIR_WALL",
        "TORCH_WALL",
        "WOMAN_WALL",
    ];
    STYLES[(x * 7 + y * 13 + (x ^ y) * 3) % STYLES.len()]
}

fn room_wall(room: &str) -> Option<&'static str> {
    Some(match room {
        "TREASURY_AREA" => "TREASURY_WALL",
        "BOOK_SHELVES" => "LIBRARY_WALL",
        "PRISON_AREA" => "PRISON_WALL",
        "TORTURE_AREA" => "TORTURE_WALL",
        "TRAINING_AREA" => "TRAINING_WALL",
        "HEART_PEDESTAL" => "HEART_WALL",
        "WORKSHOP_AREA" => "WORKSHOP_WALL",
        "SCAVENGE_AREA" => "SCAVENGER_WALL",
        "TEMPLE_POOL" => "TEMPLE_WALL",
        "GRAVE_AREA" => "GRAVE_WALL",
        "HATCHERY" => "HATCHERY_WALL",
        "LAIR_AREA" => "LAIR_WALL",
        "BARRACK_AREA" => "BARRACK_WALL",
        _ => return None,
    })
}

/// Lay the sketch out on a scratch map and capture it.
pub fn build(gd: &GameData, s: &Spec) -> Result<Prefab> {
    let h = s.rows.len();
    let w = s.rows.first().map_or(0, |r| r.chars().count());
    if h == 0 || s.rows.iter().any(|r| r.chars().count() != w) {
        return Err(Error::Invalid(format!(
            "{}: sketch rows differ in length",
            s.id
        )));
    }
    let m = 4usize;
    let dims = MapDims::new((w + 2 * m) as u16, (h + 2 * m) as u16);
    let k = |n: &str| -> Result<u16> {
        gd.slab_id(n).map(|i| i as u16).ok_or_else(|| {
            Error::Invalid(format!("{}: slab {n} is not in the configuration", s.id))
        })
    };
    let mut doc = MapDoc::new(std::path::Path::new("/nonexistent"), 1, dims, 0, gd);
    let dirt = k("DIRT")?;
    doc.place_slabs(
        gd,
        (1..dims.h as usize - 1)
            .flat_map(|y| (1..dims.w as usize - 1).map(move |x| (x, y, dirt, 5))),
    )
    .map_err(Error::from)?;
    let grid: Vec<Vec<char>> = s.rows.iter().map(|r| r.chars().collect()).collect();
    let mut selected = vec![];
    // slab kinds first, so room walls can look at their neighbours
    let mut kinds: Vec<Vec<Option<(&'static str, u8)>>> = vec![vec![None; w]; h];
    for y in 0..h {
        for x in 0..w {
            let c = grid[y][x];
            if c == '.' {
                continue;
            }
            let g = legend(c).ok_or_else(|| {
                Error::Invalid(format!("{}: unknown sketch character '{c}'", s.id))
            })?;
            kinds[y][x] = Some(match g {
                Glyph::Slab(n, o) => (n, o),
                Glyph::Wall(o) => (wall_kind(x, y), o),
                Glyph::Door(..) => ("PRETTY_PATH", s.door_owner),
                Glyph::Bridge(_) => ("BRIDGE_FRAME", 5),
            });
            selected.push((x + m, y + m));
        }
    }
    let mut placed = vec![];
    for y in 0..h {
        for x in 0..w {
            let Some((n, o)) = kinds[y][x] else { continue };
            let mut name = n;
            if n.ends_with("_WALL") {
                for (dx, dy) in [(0i32, -1i32), (1, 0), (0, 1), (-1, 0)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    if let Some((rn, _)) = kinds[ny as usize][nx as usize] {
                        if let Some(rw) = room_wall(rn) {
                            name = rw;
                            break;
                        }
                    }
                }
            }
            placed.push((x + m, y + m, k(name)?, o));
        }
    }
    doc.place_slabs(gd, placed).map_err(Error::from)?;
    doc.regenerate_rect(
        gd,
        SlabRect {
            x0: 0,
            y0: 0,
            x1: dims.w as i32 - 1,
            y1: dims.h as i32 - 1,
        },
    );
    for y in 0..h {
        for x in 0..w {
            match legend(grid[y][x]) {
                Some(Glyph::Door(model, locked)) => {
                    doc.place_door(gd, x + m, y + m, model, s.door_owner, locked)
                        .map_err(Error::from)?;
                }
                Some(Glyph::Bridge(wlb)) => {
                    doc.set_wlb(x + m, y + m, wlb).map_err(Error::from)?;
                }
                _ => {}
            }
        }
    }
    doc.apply_slab_objects(
        gd,
        SlabRect {
            x0: 0,
            y0: 0,
            x1: dims.w as i32 - 1,
            y1: dims.h as i32 - 1,
        },
    );
    let names = crate::names::Names::new(gd);
    let cls_of = |c: ThingClass| c.id();
    let mut gate_n = 0;
    let mut gate_keys: Vec<(i64, &str)> = vec![];
    for t in &s.things {
        let model = names.model_id(t.class, t.model).ok_or_else(|| {
            Error::Invalid(format!(
                "{}: {} {} is not in the configuration",
                s.id,
                t.class.name(),
                t.model
            ))
        })?;
        let (rx, ry) = (
            ((t.x + m as f32) * 768.0) as i64,
            ((t.y + m as f32) * 768.0) as i64,
        );
        let (sx, sy) = ((rx / 256) as i32, (ry / 256) as i32);
        let mut th = things::new_thing(
            cls_of(t.class),
            model,
            t.owner,
            StlCoord::new(sx, (rx % 256) as i32),
            StlCoord::new(sy, (ry % 256) as i32),
            doc.floor_z(sx, sy),
        );
        if t.class == ThingClass::Creature {
            th.creature_level = Some(t.level);
        }
        if let Some(key) = t.gate {
            gate_n += 1;
            th.herogate_number = Some(gate_n);
            gate_keys.push((gate_n, key));
        }
        let _ = class::OBJECT;
        doc.add_thing(gd, th);
    }
    let mut ap_keys: Vec<(i64, &str)> = vec![];
    for (i, (key, x, y, range)) in s.aps.iter().enumerate() {
        let n = i as i64 + 1;
        doc.add_action_point(
            things::centred(3 * (*x + m as i32) + 1),
            things::centred(3 * (*y + m as i32) + 1),
            StlCoord::new(*range, 0),
            Some(n),
        );
        ap_keys.push((n, key));
    }
    let opts = CaptureOptions {
        name: s.name.to_string(),
        author: "KeeperFX Editor starter set".into(),
        description: s.description.to_string(),
        tags: s.tags.iter().map(|t| t.to_string()).collect(),
        mask_empty: false,
        ..Default::default()
    };
    let mut p = capture(
        &doc,
        gd,
        &Selection::Cells(
            selected
                .iter()
                .map(|&(x, y)| (x as i32, y as i32))
                .collect(),
        ),
        &opts,
    )?;
    for a in &mut p.action_points {
        if let Some((_, key)) = ap_keys.iter().find(|(n, _)| Some(*n) == a.number) {
            a.key = key.to_string();
        }
    }
    for t in &mut p.things {
        if let Some(g) = &t.gate {
            let n: i64 = g.trim_start_matches("gate").parse().unwrap_or(0);
            if let Some((_, key)) = gate_keys.iter().find(|(i, _)| *i == n) {
                t.gate = Some(key.to_string());
            }
        }
    }
    p.scripts.dk = s.script_dk.map(str::to_string);
    p.sort_items();
    p.thumbnail = Some(crate::render::thumbnail_png(&p, 96));
    p.check()?;
    Ok(p)
}

/// All starter prefabs, in file name order.
pub fn build_all(gd: &GameData) -> Result<Vec<(String, Prefab)>> {
    specs()
        .iter()
        .map(|s| Ok((s.id.to_string(), build(gd, s)?)))
        .collect()
}

/// File name of a starter piece.
pub fn file_name(id: &str) -> String {
    format!("{id}.{FILE_EXT}")
}

/// Write the starter set into `dir` (replaces files of the same names). Returns the file names.
pub fn write_all(gd: &GameData, dir: &std::path::Path) -> Result<Vec<String>> {
    let mut out = vec![];
    for (id, p) in build_all(gd)? {
        p.save(&dir.join(file_name(&id)))?;
        out.push(file_name(&id));
    }
    Ok(out)
}
