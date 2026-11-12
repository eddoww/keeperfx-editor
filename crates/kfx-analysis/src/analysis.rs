//! Every analysis of a map in one value, computed from a document or from a terrain + sites
//! pair (the generator checks its plans this way).
use crate::economy::{economy, Economy};
use crate::fairness::{fairness, Fairness};
use crate::heroes::{hero_groups, hero_pressure, HeroGroup, HeroPressure};
use crate::nav::{meeting, DistanceField, Mover};
use crate::reach::{pockets, thing_reach, Endpoint, KeeperFields, Pocket, Route, ThingReach};
use crate::rules::Rules;
use crate::sites::Sites;
use crate::symmetry::{detect, SymmetryCheck};
use crate::terrain::{KindTable, Terrain};
use kfx_map::autotile::SlabRect;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use std::time::Instant;

/// First contact between two keepers digging towards each other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub a: u8,
    pub b: u8,
    /// Where they meet.
    pub slab: (i32, i32),
    /// Turns until then (the slower side's dig time to that slab).
    pub turns: u32,
}

/// What to compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub symmetry: bool,
    pub heroes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            symmetry: true,
            heroes: true,
        }
    }
}

/// Microseconds spent per part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timings {
    pub classify_us: u64,
    pub fields_us: u64,
    pub economy_us: u64,
    pub heroes_us: u64,
    pub symmetry_us: u64,
    pub total_us: u64,
}

/// The analyses of one map.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub rules: Rules,
    pub table: KindTable,
    pub terrain: Terrain,
    pub sites: Sites,
    pub options: Options,
    /// Per keeper with a heart, in owner order.
    pub fields: Vec<KeeperFields>,
    /// Per hero gate (same order as `sites.gates`): heroes with a tunneller.
    pub gate_fields: Vec<DistanceField>,
    /// Shortest dig routes heart to heart (each pair once) and gate to nearest heart.
    pub routes: Vec<Route>,
    pub contacts: Vec<Contact>,
    pub thing_reach: Vec<ThingReach>,
    pub pockets: Vec<Pocket>,
    pub economy: Economy,
    pub hero_groups: Vec<HeroGroup>,
    pub hero_pressure: Vec<HeroPressure>,
    /// `None` with fewer than two keepers.
    pub fairness: Option<Fairness>,
    pub symmetry: Vec<SymmetryCheck>,
    pub timings: Timings,
}

fn us(t: Instant) -> u64 {
    t.elapsed().as_micros() as u64
}

impl Analysis {
    /// Analyse a document with the rules of `gd`.
    pub fn of_doc(doc: &MapDoc, gd: &GameData) -> Analysis {
        let rules = Rules::from_game(gd);
        let table = KindTable::new(gd, &rules);
        Analysis::of_doc_with(doc, gd, rules, table, Options::default())
    }

    pub fn of_doc_with(
        doc: &MapDoc,
        gd: &GameData,
        rules: Rules,
        table: KindTable,
        options: Options,
    ) -> Analysis {
        let t0 = Instant::now();
        let terrain = Terrain::of_doc(doc, &table);
        let sites = Sites::of_doc(doc, gd, &rules, &terrain);
        let c = us(t0);
        let mut a = Analysis::from_parts(gd, rules, table, terrain, sites, options);
        a.timings.classify_us = c;
        a.timings.total_us += c;
        a
    }

    /// Analyse a classified terrain with its sites.
    pub fn from_parts(
        gd: &GameData,
        rules: Rules,
        table: KindTable,
        terrain: Terrain,
        sites: Sites,
        options: Options,
    ) -> Analysis {
        let mut a = Analysis {
            rules,
            table,
            terrain,
            sites,
            options,
            fields: vec![],
            gate_fields: vec![],
            routes: vec![],
            contacts: vec![],
            thing_reach: vec![],
            pockets: vec![],
            economy: Economy {
                owner_of: vec![],
                contested: vec![],
                players: vec![],
                total_gold_slabs: 0,
                total_gold_value: 0,
                total_gems: 0,
                unreachable_gold_slabs: 0,
                rooms: vec![],
            },
            hero_groups: vec![],
            hero_pressure: vec![],
            fairness: None,
            symmetry: vec![],
            timings: Timings::default(),
        };
        a.recompute(gd);
        a
    }

    /// Re-run everything after the terrain / sites changed.
    pub fn recompute(&mut self, gd: &GameData) {
        let t0 = Instant::now();
        let (t, r, s) = (&self.terrain, &self.rules, &self.sites);
        self.fields = s
            .keepers()
            .into_iter()
            .map(|p| KeeperFields::compute(t, r, p, &s.heart_cells(p)))
            .collect();
        self.gate_fields = s
            .gates
            .iter()
            .map(|g| DistanceField::compute(t, r, Mover::Hero { tunnel: true }, &[g.slab]))
            .collect();
        self.routes.clear();
        self.contacts.clear();
        for i in 0..self.fields.len() {
            for j in i + 1..self.fields.len() {
                let (fa, fb) = (&self.fields[i], &self.fields[j]);
                let targets = s.heart_cells(fb.player);
                let (from, to) = (
                    Endpoint::Heart { owner: fa.player },
                    Endpoint::Heart { owner: fb.player },
                );
                match Route::build(t, s, &fa.dig, from, to, &targets) {
                    Some(rt) => self.routes.push(rt),
                    None => {
                        // walled in: the route breaking through walls
                        let siege = DistanceField::compute(
                            t,
                            r,
                            Mover::Siege { player: fa.player },
                            &s.heart_cells(fa.player),
                        );
                        self.routes
                            .extend(Route::build(t, s, &siege, from, to, &targets));
                    }
                }
                if let Some((slab, turns)) = meeting(&fa.dig, &fb.dig) {
                    self.contacts.push(Contact {
                        a: fa.player,
                        b: fb.player,
                        slab,
                        turns,
                    });
                }
            }
        }
        for (g, f) in s.gates.iter().zip(&self.gate_fields) {
            let mut best: Option<Route> = None;
            for p in s.keepers() {
                if let Some(rt) = Route::build(
                    t,
                    s,
                    f,
                    Endpoint::Gate { number: g.number },
                    Endpoint::Heart { owner: p },
                    &s.heart_cells(p),
                ) {
                    if best.as_ref().is_none_or(|b| rt.turns < b.turns) {
                        best = Some(rt);
                    }
                }
            }
            self.routes.extend(best);
        }
        self.thing_reach = thing_reach(s, &self.fields);
        let mut reached = vec![false; t.len()];
        for f in &self.fields {
            for (k, d) in f.cross.dist.iter().enumerate() {
                reached[k] |= *d != crate::nav::INF;
            }
        }
        for f in &self.gate_fields {
            for (k, d) in f.dist.iter().enumerate() {
                reached[k] |= *d != crate::nav::INF;
            }
        }
        let anchored = !self.fields.is_empty() || !self.gate_fields.is_empty();
        self.pockets = if anchored {
            pockets(t, s, &reached)
        } else {
            vec![]
        };
        self.timings.fields_us = us(t0);
        let t1 = Instant::now();
        self.economy = economy(t, s, gd, r, &self.fields);
        self.timings.economy_us = us(t1);
        let t2 = Instant::now();
        if self.options.heroes {
            self.hero_groups = hero_groups(gd, s);
            self.hero_pressure = hero_pressure(t, r, s, &self.hero_groups);
        } else {
            self.hero_groups.clear();
            self.hero_pressure.clear();
        }
        let contacts: Vec<(u8, u8, u32)> =
            self.contacts.iter().map(|c| (c.a, c.b, c.turns)).collect();
        self.fairness = fairness(
            t,
            s,
            &self.economy,
            &self.fields,
            &contacts,
            &self.hero_pressure,
            &self.hero_groups,
        );
        self.timings.heroes_us = us(t2);
        let t3 = Instant::now();
        self.symmetry = if self.options.symmetry {
            detect(gd, t, s)
        } else {
            vec![]
        };
        self.timings.symmetry_us = us(t3);
        self.timings.total_us = us(t0);
    }

    /// Update after an edit of `rect` (slabs, inclusive): the slabs of the rectangle (grown by
    /// one) are re-classified, the sites re-read (things may have moved), and the fields
    /// recomputed (a full recompute is fast enough: see the crate docs for timings).
    pub fn update(&mut self, doc: &MapDoc, gd: &GameData, rect: SlabRect) {
        let t0 = Instant::now();
        if doc.dims.w as i32 != self.terrain.w || doc.dims.h as i32 != self.terrain.h {
            self.terrain = Terrain::of_doc(doc, &self.table);
        } else {
            let r = rect.grow(1);
            self.terrain
                .update_rect(doc, &self.table, r.x0, r.y0, r.x1, r.y1);
        }
        self.sites = Sites::of_doc(doc, gd, &self.rules, &self.terrain);
        let c = us(t0);
        self.recompute(gd);
        self.timings.classify_us = c;
        self.timings.total_us += c;
    }

    /// The fields of keeper `p`.
    pub fn keeper(&self, p: u8) -> Option<&KeeperFields> {
        self.fields.iter().find(|f| f.player == p)
    }

    /// Modes the whole map (terrain and things) satisfies exactly.
    pub fn symmetric_modes(&self) -> Vec<kfx_map::transform::SymmetryMode> {
        self.symmetry
            .iter()
            .filter(|c| c.exact())
            .map(|c| c.mode)
            .collect()
    }
}
