//! Config-driven constants (creature, room, slab, ... names).
//!
//! The editor feeds the real layered configuration through [`ConstantProvider`]; tests use
//! [`InstallProvider`], which reads `Name` keys straight from the install's `fxdata` files.
use kfx_formats::cfg::{latin1_decode, CfgDoc};
use kfx_formats::rnc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Kinds of config-defined names a script can reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConstKind {
    Creature,
    Room,
    Slab,
    Trap,
    Door,
    /// `power_desc` (magic.cfg `[powerN]`).
    Power,
    /// `spell_desc` (magic.cfg `[spellN]`).
    Spell,
    Object,
    /// Effects and effect elements (effects.toml).
    Effect,
    EffectGen,
    /// Creature instances (creature.cfg `[instanceN]` `Name`): `SET_CREATURE_INSTANCE`.
    Instance,
    /// Computer player processes, checks and events (keepcompp.cfg `[processN]`, `[checkN]`,
    /// `[eventN]` `NAME`): `SET_COMPUTER_PROCESS`, `SET_COMPUTER_CHECKS`, `SET_COMPUTER_EVENT`.
    ComputerProcess,
    ComputerCheck,
    ComputerEvent,
}

/// The kinds added after [`ALL_CONST_KINDS`] (kept apart so that array keeps its type). A host
/// that does not supply them leaves the validator silent about them.
pub const EXTRA_CONST_KINDS: [ConstKind; 4] = [
    ConstKind::Instance,
    ConstKind::ComputerProcess,
    ConstKind::ComputerCheck,
    ConstKind::ComputerEvent,
];

pub const ALL_CONST_KINDS: [ConstKind; 10] = [
    ConstKind::Creature,
    ConstKind::Room,
    ConstKind::Slab,
    ConstKind::Trap,
    ConstKind::Door,
    ConstKind::Power,
    ConstKind::Spell,
    ConstKind::Object,
    ConstKind::Effect,
    ConstKind::EffectGen,
];

/// Source of config-defined names and of map facts the script refers to.
///
/// An empty `names(kind)` slice means "unknown": the validator then stays silent for that kind.
pub trait ConstantProvider {
    /// Names (any case) valid for `kind`.
    fn names(&self, kind: ConstKind) -> &[String];
    /// Action point numbers present on the map (see [`ConstantProvider::action_points_known`]).
    fn action_points(&self) -> &[i32];
    /// Hero gate numbers present on the map.
    fn hero_gates(&self) -> &[i32];
    /// True when `action_points()` is authoritative (an empty list then means "none").
    fn action_points_known(&self) -> bool {
        false
    }
    /// True when `hero_gates()` is authoritative.
    fn hero_gates_known(&self) -> bool {
        false
    }
    fn knows(&self, kind: ConstKind) -> bool {
        !self.names(kind).is_empty()
    }
}

/// Provider that knows nothing: no unknown-constant diagnostics are produced.
#[derive(Default, Clone, Debug)]
pub struct NullProvider;

impl ConstantProvider for NullProvider {
    fn names(&self, _kind: ConstKind) -> &[String] {
        &[]
    }
    fn action_points(&self) -> &[i32] {
        &[]
    }
    fn hero_gates(&self) -> &[i32] {
        &[]
    }
}

/// Simple in-memory provider (for the editor and tests).
#[derive(Default, Clone, Debug)]
pub struct StaticProvider {
    pub names: HashMap<ConstKind, Vec<String>>,
    pub action_points: Vec<i32>,
    pub hero_gates: Vec<i32>,
    pub map_known: bool,
}

impl ConstantProvider for StaticProvider {
    fn names(&self, kind: ConstKind) -> &[String] {
        self.names.get(&kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn action_points(&self) -> &[i32] {
        &self.action_points
    }
    fn hero_gates(&self) -> &[i32] {
        &self.hero_gates
    }
    fn action_points_known(&self) -> bool {
        self.map_known
    }
    fn hero_gates_known(&self) -> bool {
        self.map_known
    }
}

/// Reads names from an install's config files. Limits (rough on purpose):
///
/// * Names are the union over the base `fxdata` files and any extra layer directories added with
///   [`InstallProvider::add_layer`]; a layer never removes a name and `mods/` are not applied.
/// * Section names are matched by prefix (`[roomN]`, `[slabN]`, `[trapN]`, `[doorN]`,
///   `[powerN]`, `[spellN]`, `[objectN]`, `[effectN]`/`[effectElementN]`, `[effectGeneratorN]`);
///   the `Name` key is taken as written, with surrounding quotes removed (effects.toml).
/// * Creature names come from the `Creatures` list of the `[common]` section of `creature.cfg`;
///   names added in-script with `NEW_*_TYPE` are handled by the validator, not here.
/// * Map facts (action points, hero gates) are only available through [`Self::set_map`].
#[derive(Default, Clone, Debug)]
pub struct InstallProvider {
    names: HashMap<ConstKind, Vec<String>>,
    action_points: Vec<i32>,
    hero_gates: Vec<i32>,
    map_known: bool,
    layers: Vec<PathBuf>,
}

fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    t.trim_matches('"').trim_matches('\'').to_string()
}

fn is_indexed(section: &str, prefix: &str) -> bool {
    let s = section.to_ascii_lowercase();
    s.strip_prefix(&prefix.to_ascii_lowercase())
        .map(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or(false)
}

impl InstallProvider {
    /// Base `fxdata` of the install.
    pub fn new(install: &Path) -> InstallProvider {
        let mut p = InstallProvider::default();
        p.add_layer(&install.join("fxdata"));
        p
    }

    /// Add a directory with partial/complete config files (a campaign's `CONFIGS_LOCATION`).
    pub fn add_layer(&mut self, dir: &Path) {
        if !dir.is_dir() {
            return;
        }
        self.layers.push(dir.to_path_buf());
        let load = |name: &str| -> Option<CfgDoc> {
            let p = find_ci(dir, name)?;
            let bytes = rnc::load_file(&p).ok()?;
            Some(CfgDoc::parse(&latin1_decode(&bytes)))
        };
        let mut add = |kind: ConstKind, doc: &Option<CfgDoc>, prefix: &str| {
            if let Some(doc) = doc {
                for sec in doc.sections() {
                    if is_indexed(&sec.name, prefix) {
                        for e in sec
                            .entries
                            .iter()
                            .filter(|e| e.key.eq_ignore_ascii_case("name"))
                        {
                            let n = strip_quotes(&e.value);
                            if !n.is_empty() {
                                self.names
                                    .entry(kind)
                                    .or_default()
                                    .push(n.to_ascii_uppercase());
                            }
                        }
                    }
                }
            }
        };
        let terrain = load("terrain.cfg");
        add(ConstKind::Room, &terrain, "room");
        add(ConstKind::Slab, &terrain, "slab");
        let td = load("trapdoor.cfg");
        add(ConstKind::Trap, &td, "trap");
        add(ConstKind::Door, &td, "door");
        let magic = load("magic.cfg");
        add(ConstKind::Power, &magic, "power");
        add(ConstKind::Spell, &magic, "spell");
        add(ConstKind::Object, &load("objects.cfg"), "object");
        let fx = load("effects.toml");
        add(ConstKind::Effect, &fx, "effect");
        add(ConstKind::Effect, &fx, "effectElement");
        add(ConstKind::EffectGen, &fx, "effectGenerator");
        let comp = load("keepcompp.cfg");
        add(ConstKind::ComputerProcess, &comp, "process");
        add(ConstKind::ComputerCheck, &comp, "check");
        add(ConstKind::ComputerEvent, &comp, "event");
        if let Some(c) = load("creature.cfg") {
            for sec in c.sections() {
                if is_indexed(&sec.name, "instance") {
                    for e in sec
                        .entries
                        .iter()
                        .filter(|e| e.key.eq_ignore_ascii_case("name"))
                    {
                        let n = strip_quotes(&e.value);
                        if !n.is_empty() {
                            self.names
                                .entry(ConstKind::Instance)
                                .or_default()
                                .push(n.to_ascii_uppercase());
                        }
                    }
                }
            }
            if let Some(list) = c.get_tokens("common", "Creatures") {
                self.names
                    .entry(ConstKind::Creature)
                    .or_default()
                    .extend(list.into_iter().map(|s| s.to_ascii_uppercase()));
            }
        }
        for v in self.names.values_mut() {
            v.sort();
            v.dedup();
        }
    }

    /// Supply the map facts used by `L` arguments (`IF_ACTION_POINT`, negative hero gates, ...).
    pub fn set_map(&mut self, action_points: Vec<i32>, hero_gates: Option<Vec<i32>>) {
        self.action_points = action_points;
        self.map_known = true;
        if let Some(h) = hero_gates {
            self.hero_gates = h;
        }
    }

    pub fn layers(&self) -> &[PathBuf] {
        &self.layers
    }
}

impl ConstantProvider for InstallProvider {
    fn names(&self, kind: ConstKind) -> &[String] {
        self.names.get(&kind).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn action_points(&self) -> &[i32] {
        &self.action_points
    }
    fn hero_gates(&self) -> &[i32] {
        &self.hero_gates
    }
    fn action_points_known(&self) -> bool {
        self.map_known
    }
    fn hero_gates_known(&self) -> bool {
        !self.hero_gates.is_empty()
    }
}
