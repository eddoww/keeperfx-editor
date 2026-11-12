//! Golden test: regenerate the columns of every shipped map and compare them, subtile by
//! subtile, with the shipped `.clm` / `.dat` (by column content, not pool index).
//!
//! Tables printed:
//! * **Table 1, exact**: the generated column has the same cubes, floor texture, solid mask,
//!   orientation and height/lintel bits (`bitfields & 0xFE`) as the shipped one. A slab is exact
//!   when its 9 subtiles are.
//! * **Table 2, loose**: the shipped column occurs at the same subtile position in any of the 28
//!   slabset entries of the slab's kind or of the kind the generator resolved for that subtile
//!   (owner cubes substituted), compared modulo the decoration families below.
//!
//! Then every non-exact subtile is put in the first matching explanation class:
//! 1. `bits only`: same content, different height/lintel bits.
//! 2. `visually identical`: every differing cube has the same 6 texture ids (KFX added copies
//!    of original cubes as ownership-group templates, e.g. 448 = cube 4), and/or only the floor
//!    texture under a solid bottom cube differs (not drawn, see [`Norm::visual`]).
//! 3. `decoration randomisation`: equal once cubes / floor textures of the families in
//!    [`CUBE_FAMILIES`] / [`FLOOR_FAMILIES`] are identified. The game never randomises
//!    (map_blocks.c has no random choice); the shipped maps were made by editors that pick a random
//!    member of these families.
//! 4. `KFX slabset changed`: matches (modulo 2+3) what the port produces with the slabset /
//!    columnset / cubes of KeeperFX commit bef5e1b88 (Dec 2023, the first TOML slabset, before the
//!    look changes listed in the report). Enabled with `KFX_GOLDEN_OLD_SLABSET=<dir>`: a copy of
//!    the install's `fxdata` configs where `slabset.toml`, `columnset.toml` and `cubes.cfg` are and
//!    `terrain.cfg` replaced by `git show bef5e1b88:config/fxdata/<file>`; it replaces the base
//!    layer.
//! 5. `room wall not decorated`: the port used the adjacent room's wall kind
//!    (`place_single_slab_fill_arrays_std`) and the shipped column is the same style of the slab's
//!    own kind.
//! 6. `other variant of the same kind`: loose match (Table 2) in the current or old slabset.
//! 7. `unreferenced columnset column`: the shipped column is in `columnset.toml` but no slabset
//!    entry uses it (e.g. column 344, the original editor's fill earth).
//! 8. `column of another slab kind`: the shipped column is what the (current or old) slabset
//!    gives for a different slab kind (columns not updated after a `.slb` change).
//! 9. `unexplained`.
//!
//! Also printed: wibble agreement, slab objects vs the map's thing/light files, bridge frames.
//!
//! Diagnostics: `KFX_GOLDEN_DETAIL=<n>` samples per class, `KFX_GOLDEN_ONLY=<substring>` limits
//! maps, `KFX_GOLDEN_KINDS=a,b` limits samples to slab kinds, `KFX_GOLDEN_CLASS=<substring>` limits
//! samples to classes.
use kfx_formats::level_bin::{Column, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_map::autotile::{owner_column, MapGen, SlabView};
use kfx_map::gamedata::{
    pack_folders, ConfigLayer, ConfigLayers, GameData, SlabsetColumn, SLABSETS_PER_SLAB,
};
use kfx_map::mapdoc::MapDoc;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Cube families the original editors randomise. Evidence: in shipped maps these substitute for
/// each other at the same column position with near-uniform counts in both directions (printed by
/// this test), and they are named / textured as variants (cubes.cfg: EARTH_STD1-3, PATH_SMOOTH1-3 +
/// PATH_LRGSTONES + PATH_SMLSTONES (top textures 27-31), GOLD1-3, GOLD_DARKSD1-3, GEMS1-4,
/// DENSE_GOLD1-3, DENSE_GOLD_NEAR_LAVA1-3; unnamed brick / floor triples with consecutive texture
/// ids).
const CUBE_FAMILIES: &[&[u16]] = &[
    &[1, 2, 3],
    &[25, 26, 27, 28, 29],
    &[49, 50, 51],
    &[52, 53, 54],
    &[72, 73, 74],
    &[76, 77, 78],
    &[81, 82, 83],
    &[126, 127, 128],
    &[174, 175],
    &[263, 264, 265],
    &[441, 442, 443, 444],
    &[482, 483, 484],
    &[485, 486, 487],
];
/// Floor texture families: the top textures of the path cubes (27-31), the two lava surfaces
/// (546/547, LAVA_SURFACE1/2), the claimed floor tops (176-178, cubes 126-128).
const FLOOR_FAMILIES: &[&[u16]] = &[&[27, 28, 29, 30, 31], &[546, 547], &[176, 177, 178]];

fn folders(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for top in ["campgns", "levels", "multiplayer"] {
        let Some(t) = testutil::resolve_ci(root, top) else {
            continue;
        };
        out.push(t.clone());
        let mut stack = vec![t];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    out.push(e.path());
                    stack.push(e.path());
                }
            }
        }
    }
    out.sort();
    out
}

fn slb_numbers(dir: &Path) -> Vec<u32> {
    let mut v: Vec<u32> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            let num = n.strip_prefix("map")?.strip_suffix(".slb")?;
            (num.len() == 5).then(|| num.parse().ok())?
        })
        .collect();
    v.sort();
    v
}

fn same_content(a: &Column, b: &Column) -> bool {
    a.cubes == b.cubes
        && a.floor_texture == b.floor_texture
        && a.solidmask == b.solidmask
        && a.orient == b.orient
}

type Key = ([u16; 8], u16, u16, u8);

/// Normalisation used by classes 2 and 3.
struct Norm {
    /// Floor textures whose top cube (`init_top_texture_to_cube_table`, src/map_columns.c:446) is
    /// an abyss cube: their floor texture matters even under a solid column.
    abyss_ft: HashSet<u16>,
    tex_canon: HashMap<u16, u16>,
    family: HashMap<u16, u16>,
    ft_family: HashMap<u16, u16>,
}

impl Norm {
    fn new(gd: &GameData) -> Self {
        let mut first: HashMap<[u16; 6], u16> = HashMap::new();
        let mut tex_canon = HashMap::new();
        for (i, c) in gd.cubes.iter().enumerate() {
            let id = *first.entry(c.textures).or_insert(i as u16);
            tex_canon.insert(i as u16, id);
        }
        let mut family = HashMap::new();
        for f in CUBE_FAMILIES {
            for &c in *f {
                family.insert(c, f[0]);
            }
        }
        let mut ft_family = HashMap::new();
        for f in FLOOR_FAMILIES {
            for &c in *f {
                ft_family.insert(c, f[0]);
            }
        }
        let mut abyss_ft = HashSet::new();
        for n in 1..2048u16 {
            if let Some(c) = gd.cubes.iter().skip(1).find(|c| c.textures[4] == n) {
                if c.properties & kfx_map::gamedata::cube_flags::ABYSS != 0 {
                    abyss_ft.insert(n);
                }
            }
        }
        Norm {
            abyss_ft,
            tex_canon,
            family,
            ft_family,
        }
    }
    /// Under a solid bottom cube the floor texture is not drawn (src/engine_render.c:4154-4166 draws
    /// the top of the highest floor cube instead); it only feeds the first-person ceiling
    /// (`floor_to_ceiling_map`, from data/ceiling.txt, which maps every floor texture these maps
    /// use to 207) and the abyss test. So it is ignored there unless it is an abyss texture.
    fn visual(&self, c: &Column) -> Key {
        let mut cubes = c.cubes;
        for x in cubes.iter_mut() {
            *x = *self.tex_canon.get(x).unwrap_or(x);
        }
        let ft = if c.cubes[0] != 0 && !self.abyss_ft.contains(&c.floor_texture) {
            u16::MAX
        } else {
            c.floor_texture
        };
        (cubes, ft, c.solidmask, c.orient)
    }
    fn decor(&self, c: &Column) -> Key {
        let (mut cubes, ft, sm, o) = self.visual(c);
        for x in cubes.iter_mut() {
            *x = *self.family.get(x).unwrap_or(x);
        }
        (cubes, *self.ft_family.get(&ft).unwrap_or(&ft), sm, o)
    }
}

#[derive(Default, Clone)]
struct Row {
    slabs: usize,
    slab_exact: usize,
    slab_decor: usize,
    slab_loose: usize,
    subtiles: usize,
    sub_exact: usize,
    sub_decor: usize,
    sub_loose: usize,
}

const NCLASS: usize = 9;
const CLASSES: [&str; NCLASS] = [
    "1 bits only",
    "2 visually identical (texture-identical cubes / hidden floor texture)",
    "3 decoration randomisation",
    "4 KFX slabset changed",
    "5 room wall not decorated by the map's tool",
    "6 other variant of the same kind",
    "7 unreferenced columnset column",
    "8 column of another slab kind",
    "9 unexplained",
];

struct Data {
    dec: kfx_map::decorate::Decorator,
    gd: GameData,
    norm: Norm,
    /// decor keys of every slabset column (any kind/pos/owner) -> kinds producing it
    by_key: HashMap<Key, HashSet<usize>>,
    /// decor keys of every columnset column
    columnset: HashSet<Key>,
}

impl Data {
    fn new(gd: GameData) -> Self {
        let norm = Norm::new(&gd);
        let mut by_key: HashMap<Key, HashSet<usize>> = HashMap::new();
        for (id, ss) in gd.slabset.iter().enumerate() {
            for c in ss.iter() {
                if let SlabsetColumn::Column(c) = c {
                    for owner in 0..9u8 {
                        by_key
                            .entry(norm.decor(&owner_column(&gd, c, owner)))
                            .or_default()
                            .insert(id / SLABSETS_PER_SLAB);
                    }
                }
            }
        }
        let columnset = gd
            .columnset
            .iter()
            .map(|c| norm.decor(&kfx_map::column::canonical_column(c, false)))
            .collect();
        let dec = kfx_map::decorate::Decorator::new(&gd, 1);
        Data {
            dec,
            gd,
            norm,
            by_key,
            columnset,
        }
    }
    fn loose(
        &self,
        cache: &mut HashMap<(usize, usize, u8), HashSet<Key>>,
        kinds: &[usize],
        pos: usize,
        owner: u8,
        ship: &Key,
    ) -> bool {
        kinds.iter().any(|&k| {
            cache
                .entry((k, pos, owner))
                .or_insert_with(|| {
                    (0..SLABSETS_PER_SLAB)
                        .filter_map(|st| self.gd.slabset_column(k * SLABSETS_PER_SLAB + st, pos))
                        .map(|c| self.norm.decor(&owner_column(&self.gd, c, owner)))
                        .collect()
                })
                .contains(ship)
        })
    }
}

#[test]
fn golden_columns() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let env = |k: &str| std::env::var(k).ok();
    let detail: usize = env("KFX_GOLDEN_DETAIL")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let only = env("KFX_GOLDEN_ONLY");
    let sample_kinds: Option<Vec<usize>> =
        env("KFX_GOLDEN_KINDS").map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect());
    let sample_class = env("KFX_GOLDEN_CLASS");
    let old_dir = env("KFX_GOLDEN_OLD_SLABSET")
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let packs = pack_folders(&root);
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut cache: HashMap<ConfigLayers, (Data, Option<Data>)> = HashMap::new();
    let mut loose_cache: HashMap<(ConfigLayers, bool), HashMap<(usize, usize, u8), HashSet<Key>>> =
        HashMap::new();
    let mut rows: BTreeMap<usize, Row> = BTreeMap::new();
    let mut classes: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut class_tot = [0usize; NCLASS];
    let mut samples: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
    let mut residual_pairs: HashMap<(u16, u16), usize> = HashMap::new();
    let mut maps = 0;
    let mut problems: Vec<String> = vec![];
    let mut per_map: Vec<(String, f64, f64)> = vec![];
    let mut warnings: HashSet<String> = HashSet::new();
    // (wlb, wlb from file, shipped cube 0 at the centre) of every bridge slab
    let mut bridges: BTreeMap<(u8, bool, u16), usize> = BTreeMap::new();
    // wibble: (subtiles, equal)
    let mut wib = (0usize, 0usize);
    let mut wib_diff: BTreeMap<(usize, u8, u8), usize> = BTreeMap::new();
    // slab objects: key -> (generated, generated and present in the map's thing/light file)
    let mut objs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut shipped_parented = (0usize, 0usize);
    // decoration statistics: family name -> member -> (shipped count, decorated count)
    // (shipped objects/lights with a parent slab, of those generated)
    let mut decor_stats: BTreeMap<String, BTreeMap<u16, (usize, usize)>> = BTreeMap::new();

    for dir in folders(&root) {
        let cdir = canon(&dir);
        let pack = packs.iter().find(|(l, _, _)| canon(l) == cdir);
        for n in slb_numbers(&dir) {
            let tag = format!("{}/map{n:05}", dir.strip_prefix(&root).unwrap().display());
            if only.as_ref().is_some_and(|o| !tag.contains(o.as_str())) {
                continue;
            }
            let layers = ConfigLayers::for_level(
                &root,
                pack.and_then(|p| p.1.as_deref()),
                Some(&dir),
                pack.and_then(|p| p.2.as_deref()),
                Some(n),
            );
            if !cache.contains_key(&layers) {
                let gd = GameData::load(&layers).expect("game data");
                for w in &gd.warnings {
                    warnings.insert(w.clone());
                }
                let old = old_dir.as_ref().map(|d| {
                    let mut l = layers.clone();
                    l.layers[0] = ConfigLayer::dir(d);
                    Data::new(GameData::load(&l).expect("old game data"))
                });
                cache.insert(layers.clone(), (Data::new(gd), old));
            }
            let (data, old) = &cache[&layers];
            let gd = &data.gd;
            let slb = rnc::load_file(&kfx_formats::level_bin::find_file(&dir, n, "slb").unwrap())
                .unwrap();
            let Some(dims) = MapDims::from_slb_len(slb.len()) else {
                problems.push(format!("{tag}: cannot infer dims"));
                continue;
            };
            let doc = match MapDoc::load(&dir, n, dims, gd) {
                Ok(d) => d,
                Err(e) => {
                    problems.push(format!("{tag}: load: {e}"));
                    continue;
                }
            };
            if !doc.had_columns {
                problems.push(format!("{tag}: no clm/dat"));
                continue;
            }
            maps += 1;
            let view = doc.view(gd);
            let gen = MapGen::regenerate_all(&view, gd);
            let gen_old = old
                .as_ref()
                .map(|o| MapGen::regenerate_all(&doc.view(&o.gd), &o.gd));
            let (mut m_sub, mut m_ok, mut m_explained) = (0usize, 0usize, 0usize);
            for y in 0..dims.h as usize {
                for x in 0..dims.w as usize {
                    let kind = view.kind(x as i32, y as i32).unwrap() as usize;
                    let owner = view.owner(x as i32, y as i32).unwrap();
                    let si = dims.slab_index(x, y);
                    if gd.slab(kind).wlb_type == kfx_map::autotile::WLB_BRIDGE {
                        let c = doc.columns[dims.stl_index(3 * x + 1, 3 * y + 1)].cubes[0];
                        *bridges
                            .entry((view.wlb(x as i32, y as i32), doc.wlb_from_file, c))
                            .or_default() += 1;
                    }
                    let row = rows.entry(kind).or_default();
                    row.slabs += 1;
                    let (mut all_exact, mut all_decor, mut all_loose) = (true, true, true);
                    for i in 0..9 {
                        let k = dims.stl_index(3 * x + i % 3, 3 * y + i / 3);
                        let g = &gen.columns[k];
                        let s = &doc.columns[k];
                        row.subtiles += 1;
                        m_sub += 1;
                        let content = same_content(g, s);
                        let exact = content && (g.bitfields & 0xFE) == (s.bitfields & 0xFE);
                        let sk = data.norm.decor(s);
                        let decor = data.norm.decor(g) == sk;
                        let rk = gen.subtile_kinds[si][i] as usize;
                        let lc = loose_cache.entry((layers.clone(), false)).or_default();
                        let loose = decor || data.loose(lc, &[kind, rk], i, owner, &sk);
                        row.sub_exact += exact as usize;
                        row.sub_decor += decor as usize;
                        row.sub_loose += loose as usize;
                        all_exact &= exact;
                        all_decor &= decor;
                        all_loose &= loose;
                        if exact {
                            m_ok += 1;
                            m_explained += 1;
                            continue;
                        }
                        let old_match =
                            |o: &Data, go: &MapGen| o.norm.decor(&go.columns[k]) == o.norm.decor(s);
                        let cls = if content {
                            0
                        } else if data.norm.visual(g) == data.norm.visual(s) {
                            1
                        } else if decor {
                            2
                        } else if old
                            .as_ref()
                            .zip(gen_old.as_ref())
                            .is_some_and(|(o, go)| old_match(o, go))
                        {
                            3
                        } else if rk != kind && {
                            // fill_arrays_std made this subtile use the room's wall kind; the
                            // same style of the slab's own kind is what a tool without room walls
                            // writes.
                            let st = gen.slabset_ids[si][i] as usize % SLABSETS_PER_SLAB;
                            [st, st.saturating_sub(9)].iter().any(|&st| {
                                [Some(data), old.as_ref()].into_iter().flatten().any(|d| {
                                    d.gd.slabset_column(kind * SLABSETS_PER_SLAB + st, i)
                                        .is_some_and(|c| {
                                            d.norm.decor(&owner_column(&d.gd, c, owner))
                                                == d.norm.decor(s)
                                        })
                                })
                            })
                        } {
                            4
                        } else if loose
                            || old.as_ref().zip(gen_old.as_ref()).is_some_and(|(o, go)| {
                                let ork = go.subtile_kinds[si][i] as usize;
                                let lc = loose_cache.entry((layers.clone(), true)).or_default();
                                o.loose(lc, &[kind, ork], i, owner, &o.norm.decor(s))
                            })
                        {
                            5
                        } else if data.columnset.contains(&sk) && !data.by_key.contains_key(&sk) {
                            6
                        } else if [Some(data), old.as_ref()].into_iter().flatten().any(|d| {
                            d.by_key
                                .get(&d.norm.decor(s))
                                .is_some_and(|ks| !ks.contains(&kind))
                        }) {
                            7
                        } else {
                            8
                        };
                        if cls < 8 {
                            m_explained += 1;
                        }
                        class_tot[cls] += 1;
                        *classes.entry((kind, cls)).or_default() += 1;
                        if cls == 8 {
                            let gn = data.norm.decor(g).0;
                            for z in 0..8 {
                                if gn[z] != sk.0[z] {
                                    *residual_pairs.entry((g.cubes[z], s.cubes[z])).or_default() +=
                                        1;
                                }
                            }
                        }
                        let v = samples.entry((kind, cls)).or_default();
                        if v.len() < detail
                            && sample_kinds.as_ref().is_none_or(|ks| ks.contains(&kind))
                            && sample_class
                                .as_ref()
                                .is_none_or(|c| CLASSES[cls].contains(c.as_str()))
                        {
                            let around: Vec<String> = kfx_map::autotile::AROUND_EIGHT
                                .iter()
                                .map(|(dx, dy)| {
                                    let (ax, ay) = (x as i32 + dx, y as i32 + dy);
                                    format!(
                                        "{}/{}",
                                        view.kind(ax, ay).map_or(-1, |k| k as i32),
                                        view.owner(ax, ay).map_or(-1, |o| o as i32)
                                    )
                                })
                                .collect();
                            v.push(format!(
                                "{tag} slab({x},{y}) own {owner} sub {i} ss {} (style {}) rk {rk} \
                                    around[N..NW kind/own] {}\n        gen  {:?} ft {} bf \
                                    {:02x}\n        ship {:?} ft {} bf {:02x}{}",
                                gen.slabset_ids[si][i],
                                gen.slabset_ids[si][i] as usize % SLABSETS_PER_SLAB,
                                around.join(" "),
                                g.cubes,
                                g.floor_texture,
                                g.bitfields,
                                s.cubes,
                                s.floor_texture,
                                s.bitfields,
                                gen_old.as_ref().map_or(String::new(), |go| format!(
                                    "\n        old  {:?} ft {}",
                                    go.columns[k].cubes, go.columns[k].floor_texture
                                ))
                            ));
                        }
                    }
                    row.slab_exact += all_exact as usize;
                    row.slab_decor += all_decor as usize;
                    row.slab_loose += all_loose as usize;
                }
            }
            {
                // where the generated (undecorated) column and the shipped one use the same
                // family at the same height, compare which member each one has
                let dec = &kfx_map::decorate::Decorator {
                    seed: kfx_map::decorate::Decorator::seed_for_level(n),
                    ..data.dec.clone()
                };
                let fams = &dec.families;
                for y in 0..dims.stl_h() {
                    for x in 0..dims.stl_w() {
                        let k = dims.stl_index(x, y);
                        let (g, s) = (&gen.columns[k], &doc.columns[k]);
                        let d = dec.column(g, x, y);
                        for z in 0..8 {
                            let (Some(fs), Some(fg)) =
                                (fams.cube_family(s.cubes[z]), fams.cube_family(g.cubes[z]))
                            else {
                                continue;
                            };
                            if fs != fg {
                                continue;
                            }
                            if !dec.cube_enabled[fs] {
                                continue;
                            }
                            let m = decor_stats
                                .entry(format!("cube {}", fams.cubes[fs].name))
                                .or_default();
                            m.entry(s.cubes[z]).or_default().0 += 1;
                            m.entry(d.cubes[z]).or_default().1 += 1;
                        }
                        if s.cubes[0] == 0 && g.cubes[0] == 0 {
                            if let (Some(fs), Some(fg)) = (
                                fams.floor_family(s.floor_texture),
                                fams.floor_family(g.floor_texture),
                            ) {
                                if fs == fg {
                                    let m = decor_stats
                                        .entry(format!("floor {}", fams.floors[fs].name))
                                        .or_default();
                                    m.entry(s.floor_texture).or_default().0 += 1;
                                    m.entry(d.floor_texture).or_default().1 += 1;
                                }
                            }
                        }
                    }
                }
            }
            for y in 0..dims.stl_h() {
                for x in 0..dims.stl_w() {
                    let k = dims.stl_index(x, y);
                    wib.0 += 1;
                    wib.1 += (gen.wibble[k] == doc.wibble[k]) as usize;
                    if gen.wibble[k] != doc.wibble[k] {
                        let kd = view.kind(x as i32 / 3, y as i32 / 3).unwrap_or(0) as usize;
                        let e = wib_diff
                            .entry((kd, gen.wibble[k], doc.wibble[k]))
                            .or_default();
                        *e += 1;
                        if *e <= detail && kd == 12 {
                            let around: Vec<String> = kfx_map::autotile::AROUND_EIGHT
                                .iter()
                                .map(|(dx, dy)| {
                                    format!(
                                        "{}",
                                        view.kind(x as i32 / 3 + dx, y as i32 / 3 + dy)
                                            .map_or(-1, |k| k as i32)
                                    )
                                })
                                .collect();
                            println!(
                                "WIB {tag} stl({x},{y}) sub ({},{}) gen {} ship {} around {}",
                                x % 3,
                                y % 3,
                                gen.wibble[k],
                                doc.wibble[k],
                                around.join(" ")
                            );
                        }
                    }
                }
            }
            {
                use kfx_map::autotile::SlabObjectKind as K;
                let placed = gen.placed_objects(&view, gd);
                let thing_at = |cls: i64, model: i64, sx: i32, sy: i32| {
                    doc.things.items.iter().any(|t| {
                        t.class.as_ref().and_then(|c| c.id()) == Some(cls)
                            && t.subtype == Some(model)
                            && t.x.is_some_and(|c| c.stl == sx)
                            && t.y.is_some_and(|c| c.stl == sy)
                    })
                };
                let light_at = |sx: i32, sy: i32| {
                    doc.lights.items.iter().any(|l| {
                        l.x.is_some_and(|c| c.stl == sx) && l.y.is_some_and(|c| c.stl == sy)
                    })
                };
                let mut gen_keys: HashSet<(i64, i64, i32, i32)> = HashSet::new();
                for o in &placed {
                    let (sx, sy) = (o.x >> 8, o.y >> 8);
                    let (key, found, gk) = match o.kind {
                        K::Object { model } => (
                            format!("object {model}"),
                            thing_at(1, model as i64, sx, sy),
                            (1, model as i64, sx, sy),
                        ),
                        K::EffectGen { model, .. } => (
                            format!("effectgen {model}"),
                            thing_at(7, model as i64, sx, sy),
                            (7, model as i64, sx, sy),
                        ),
                        K::Light { .. } => ("light".to_string(), light_at(sx, sy), (0, 0, sx, sy)),
                    };
                    gen_keys.insert(gk);
                    let e = objs.entry(key).or_default();
                    e.0 += 1;
                    e.1 += found as usize;
                }
                for t in &doc.things.items {
                    let cls = t.class.as_ref().and_then(|c| c.id()).unwrap_or(0);
                    if (cls == 1 || cls == 7) && t.parent_tile.is_some_and(|p| p > 0 && p < 65535) {
                        if let (Some(x), Some(y), Some(m)) = (t.x, t.y, t.subtype) {
                            shipped_parented.0 += 1;
                            shipped_parented.1 +=
                                gen_keys.contains(&(cls, m, x.stl, y.stl)) as usize;
                        }
                    }
                }
            }
            per_map.push((
                tag,
                100.0 * m_ok as f64 / m_sub.max(1) as f64,
                100.0 * m_explained as f64 / m_sub.max(1) as f64,
            ));
        }
    }

    let gd_any = cache.values().next().map(|d| &d.0.gd);
    let name = |k: usize| {
        gd_any
            .and_then(|g| g.slab_name(k))
            .unwrap_or("?")
            .to_string()
    };
    println!(
        "maps compared: {maps}{}",
        if old_dir.is_some() {
            " (old slabset layer enabled)"
        } else {
            ""
        }
    );
    for p in &problems {
        println!("  problem: {p}");
    }
    for w in &warnings {
        println!("  game data warning: {w}");
    }
    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f64 / b as f64
        }
    };
    let mut tot = Row::default();
    println!(
        "\nTable 1: exact match (content + height/lintel bits); 'decor' = equal modulo \
        texture-identical cubes and decoration families"
    );
    println!(
        "{:>4} {:<16} {:>8} {:>8} {:>7} {:>7} | {:>9} {:>9} {:>7} {:>7}",
        "kind", "name", "slabs", "exact", "%", "decor%", "subtiles", "exact", "%", "decor%"
    );
    for (k, r) in &rows {
        println!(
            "{:>4} {:<16} {:>8} {:>8} {:>6.2}% {:>6.2}% | {:>9} {:>9} {:>6.2}% {:>6.2}%",
            k,
            name(*k),
            r.slabs,
            r.slab_exact,
            pct(r.slab_exact, r.slabs),
            pct(r.slab_decor, r.slabs),
            r.subtiles,
            r.sub_exact,
            pct(r.sub_exact, r.subtiles),
            pct(r.sub_decor, r.subtiles)
        );
        tot.slabs += r.slabs;
        tot.slab_exact += r.slab_exact;
        tot.slab_decor += r.slab_decor;
        tot.slab_loose += r.slab_loose;
        tot.subtiles += r.subtiles;
        tot.sub_exact += r.sub_exact;
        tot.sub_decor += r.sub_decor;
        tot.sub_loose += r.sub_loose;
    }
    println!(
        "{:>4} {:<16} {:>8} {:>8} {:>6.2}% {:>6.2}% | {:>9} {:>9} {:>6.2}% {:>6.2}%",
        "",
        "TOTAL",
        tot.slabs,
        tot.slab_exact,
        pct(tot.slab_exact, tot.slabs),
        pct(tot.slab_decor, tot.slabs),
        tot.subtiles,
        tot.sub_exact,
        pct(tot.sub_exact, tot.subtiles),
        pct(tot.sub_decor, tot.subtiles)
    );
    println!(
        "\nTable 2: loose match (shipped column is some slabset variant of the kind / \
        resolved kind at that position, modulo decoration)"
    );
    println!(
        "{:>4} {:<16} {:>8} {:>8} {:>7} | {:>9} {:>9} {:>7}",
        "kind", "name", "slabs", "loose", "%", "subtiles", "loose", "%"
    );
    for (k, r) in &rows {
        println!(
            "{:>4} {:<16} {:>8} {:>8} {:>6.2}% | {:>9} {:>9} {:>6.2}%",
            k,
            name(*k),
            r.slabs,
            r.slab_loose,
            pct(r.slab_loose, r.slabs),
            r.subtiles,
            r.sub_loose,
            pct(r.sub_loose, r.subtiles)
        );
    }
    println!(
        "{:>4} {:<16} {:>8} {:>8} {:>6.2}% | {:>9} {:>9} {:>6.2}%",
        "",
        "TOTAL",
        tot.slabs,
        tot.slab_loose,
        pct(tot.slab_loose, tot.slabs),
        tot.subtiles,
        tot.sub_loose,
        pct(tot.sub_loose, tot.subtiles)
    );
    println!(
        "\nNon-exact subtiles by explanation class (of {} subtiles):",
        tot.subtiles
    );
    for (c, n) in class_tot.iter().enumerate() {
        println!("  {:>9} {:>6.2}%  {}", n, pct(*n, tot.subtiles), CLASSES[c]);
    }
    println!("\nPer kind and class, biggest first:");
    let mut cl: Vec<_> = classes.iter().collect();
    cl.sort_by(|a, b| b.1.cmp(a.1));
    for ((k, c), n) in cl.iter().take(80) {
        println!("  {:>9}  kind {:>3} {:<16} {}", n, k, name(*k), CLASSES[*c]);
        if let Some(s) = samples.get(&(*k, *c)) {
            for x in s {
                println!("      {x}");
            }
        }
    }
    let mut pv: Vec<_> = residual_pairs.into_iter().collect();
    pv.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\nCube substitutions (generated -> shipped) in unexplained subtiles:");
    for ((a, b), n) in pv.iter().take(40) {
        println!("  {n:>9}  {a:>4} -> {b:>4}");
    }
    println!(
        "\nWibble: {} of {} playable subtiles equal ({:.2}%)",
        wib.1,
        wib.0,
        pct(wib.1, wib.0)
    );
    let mut wd: Vec<_> = wib_diff.into_iter().collect();
    wd.sort_by(|a, b| b.1.cmp(&a.1));
    for ((k, g, sh), n) in wd.iter().take(12) {
        println!(
            "  kind {k:>3} {:<14} generated {g} shipped {sh}: {n}",
            name(*k)
        );
    }
    println!(
        "\nSlab objects generated for the shipped layouts, and how many of them the map's own \
        thing/light file has (same model, same subtile):"
    );
    for (k, (g, f)) in &objs {
        println!(
            "  {k:<16} generated {g:>7}  present in map {f:>7} ({:.1}%)",
            pct(*f, *g)
        );
    }
    println!(
        "Shipped objects/effect generators with a parent slab index: {}, of which generated by the \
            port at the same subtile: {} ({:.1}%)",
        shipped_parented.0,
        shipped_parented.1,
        pct(shipped_parented.1, shipped_parented.0)
    );
    println!("\nBridge slabs: (wlb, wlb from file, shipped centre bottom cube) -> count");
    for (k, n) in &bridges {
        println!("  {k:?}: {n}");
    }
    println!(
        "\nDecoration families: member shares in shipped maps vs the decoration pass \
        (positions where both use the family); TVD = total variation distance"
    );
    let mut decor_fail = vec![];
    for (fam, members) in &decor_stats {
        let (ts, td): (usize, usize) = members.values().fold((0, 0), |a, v| (a.0 + v.0, a.1 + v.1));
        if ts == 0 {
            continue;
        }
        let tvd: f64 = members
            .values()
            .map(|v| (v.0 as f64 / ts as f64 - v.1 as f64 / td.max(1) as f64).abs())
            .sum::<f64>()
            / 2.0;
        let shares: Vec<String> = members
            .iter()
            .map(|(m, v)| format!("{m}: {:.1}%/{:.1}%", pct(v.0, ts), pct(v.1, td)))
            .collect();
        println!("  {fam:<34} n={ts:>8} TVD {tvd:.3}  {}", shares.join("  "));
        // The shipped maps were randomised by their editors, but not every column was (the
        // slabset's own member is over-represented, e.g. earth 45/28/26%); the pass picks
        // uniformly. A family seen often must still come out with a similar spread.
        if ts >= 2000 && tvd > 0.3 {
            decor_fail.push(format!("{fam}: TVD {tvd:.3}"));
        }
    }
    per_map.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
    println!("\nMaps with the most unexplained subtiles (exact %, explained %):");
    for (t, p, e) in per_map.iter().take(20) {
        println!("  {p:6.2}% {e:6.2}%  {t}");
    }
    assert!(
        problems.is_empty(),
        "maps that could not be compared: {problems:?}"
    );
    assert!(
        decor_fail.is_empty(),
        "decoration distribution far from the shipped maps: {decor_fail:?}"
    );
    if maps > 0 {
        // Regression guards (values at the time of writing: 90.98% and 0.46% without the old
        // slabset layer, 0.05% with it).
        assert!(
            pct(tot.sub_decor, tot.subtiles) > 90.0,
            "decoration-normalised subtile match dropped"
        );
        assert!(
            pct(class_tot[NCLASS - 1], tot.subtiles) < 1.0,
            "unexplained subtiles above 1%"
        );
    }
}
