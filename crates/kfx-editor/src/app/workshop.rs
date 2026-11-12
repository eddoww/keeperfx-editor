//! Thing palette extras: workshop boxes (crates) drawn with the trap or door they hold, and
//! spellbooks labelled with the power they give.
//!
//! A crate is the object named by a trap's or door's `Crate` field in `trapdoor.cfg`
//! (`assign_crate_door` / `assign_crate_trap`, src/config_trapdoor.c:150-171: the object maps back
//! to the door or trap; sections are read in file order, so a later one wins). A spellbook is
//! the object named by a power's `Artifact` in `magic.cfg` (`assign_artifact`,
//! src/config_magic.c:467). Both files are read through every config layer (base, campaign, map,
//! mods) like the game's `load_config`.
use super::*;
use kfx_map::gamedata::parse_number;

/// What a workshop box holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrateOf {
    pub door: bool,
    pub model: usize,
    pub name: String,
    /// `SymbolSprites` of the trap / door (big icon first).
    pub symbol: Option<String>,
}

/// An object reference of a config value (name or number), 0 for none / `NULL`.
fn object_ref(gd: &GameData, v: &str) -> usize {
    let v = v.split_whitespace().next().unwrap_or("");
    if v.is_empty() || v.eq_ignore_ascii_case("NULL") {
        return 0;
    }
    parse_number(v)
        .map(|n| n.max(0) as usize)
        .or_else(|| gd.object_id(v))
        .unwrap_or(0)
}

/// Crate object model -> the trap or door it holds.
pub fn crates(gd: &GameData) -> HashMap<usize, CrateOf> {
    let mut sections: Vec<(String, bool, usize, String)> = vec![];
    for (door, list) in [(true, gd.door_names()), (false, gd.trap_names())] {
        for (m, name) in list {
            if m != 0 {
                sections.push((
                    format!("{}{m}", if door { "door" } else { "trap" }),
                    door,
                    m,
                    name,
                ));
            }
        }
    }
    // file order: doors come before traps in trapdoor.cfg
    let mut out = HashMap::new();
    for (sec, door, model, name) in sections {
        let Some(c) = gd.trapdoor_cfg.get(&sec, "Crate") else {
            continue;
        };
        let obj = object_ref(gd, c);
        if obj != 0 {
            let symbol = gd
                .trapdoor_cfg
                .get(&sec, "SymbolSprites")
                .map(str::to_string);
            out.insert(
                obj,
                CrateOf {
                    door,
                    model,
                    name,
                    symbol,
                },
            );
        }
    }
    out
}

/// Spellbook object model -> power name (`magic.cfg` `[powerN] Artifact`, all layers).
pub fn spellbooks(gd: &GameData) -> HashMap<usize, String> {
    let Ok(magic) = gd.layered_cfg("magic.cfg") else {
        return HashMap::new();
    };
    let g = magic.to_generic();
    let mut out = HashMap::new();
    for (n, name) in g.numbered_names("power") {
        if let Some(a) = g.get(&format!("power{n}"), "Artifact") {
            let obj = object_ref(gd, a);
            if obj != 0 {
                out.insert(obj, name);
            }
        }
    }
    out
}

/// Nearest-neighbour scale of `img` to fit in `max` x `max`.
fn fit(img: &kfx_gfx::Rgba, max: u32) -> kfx_gfx::Rgba {
    let s = (max as f32 / img.w.max(img.h).max(1) as f32).min(1.0);
    let (w, h) = (
        ((img.w as f32 * s).round() as u32).max(1),
        ((img.h as f32 * s).round() as u32).max(1),
    );
    let mut out = kfx_gfx::Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.put(
                x,
                y,
                img.pixel(
                    (x * img.w / w).min(img.w - 1),
                    (y * img.h / h).min(img.h - 1),
                ),
            );
        }
    }
    out
}

/// The box sprite with the trap / door icon over its lower right part.
pub fn with_crate_icon(
    base: Option<kfx_gfx::Rgba>,
    icon: Option<&kfx_gfx::Rgba>,
) -> Option<kfx_gfx::Rgba> {
    let Some(icon) = icon.filter(|i| i.w > 0 && i.h > 0) else {
        return base;
    };
    let base = base
        .filter(|b| b.w > 0 && b.h > 0)
        .unwrap_or_else(|| kfx_gfx::Rgba::new(32, 32));
    // a canvas at least 48 px so the icon stays readable next to a small box sprite
    let side = base.w.max(base.h).max(48);
    let mut out = kfx_gfx::Rgba::new(side, side);
    out.blit(
        &base,
        ((side - base.w) / 2) as i32,
        ((side - base.h) / 2) as i32,
    );
    let ic = fit(icon, side * 3 / 5);
    let (ox, oy) = (side - ic.w, side - ic.h);
    // dark rim behind the icon so it reads over the box
    for y in oy.saturating_sub(1)..side {
        for x in ox.saturating_sub(1)..side {
            let p = out.pixel(x, y);
            out.put(x, y, [p[0] / 3, p[1] / 3, p[2] / 3, 255]);
        }
    }
    out.blit(&ic, ox as i32, oy as i32);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_goes_bottom_right() {
        let mut base = kfx_gfx::Rgba::new(20, 20);
        for p in base.data.chunks_exact_mut(4) {
            p.copy_from_slice(&[0, 0, 255, 255]);
        }
        let mut icon = kfx_gfx::Rgba::new(64, 64);
        for p in icon.data.chunks_exact_mut(4) {
            p.copy_from_slice(&[255, 0, 0, 255]);
        }
        let out = with_crate_icon(Some(base), Some(&icon)).unwrap();
        assert_eq!((out.w, out.h), (48, 48));
        assert_eq!(out.pixel(47, 47), [255, 0, 0, 255]);
        assert_eq!(out.pixel(24, 16), [0, 0, 255, 255]);
        assert_eq!(out.pixel(0, 0)[3], 0);
        assert!(with_crate_icon(None, None).is_none());
    }

    #[test]
    fn crates_and_spellbooks_of_the_install() {
        let Some(inst) = kfx_formats::testutil::install_dir() else {
            return;
        };
        let gd = GameData::load(&crate::document::base_layers(&inst)).unwrap();
        let c = crates(&gd);
        let boulder = gd.object_id("WRKBOX_BOULDER").expect("WRKBOX_BOULDER");
        assert_eq!(
            c.get(&boulder).map(|x| (x.door, x.name.as_str())),
            Some((false, "BOULDER"))
        );
        let wood = gd.object_id("WRKBOX_WOOD").expect("WRKBOX_WOOD");
        assert_eq!(
            c.get(&wood).map(|x| (x.door, x.name.as_str())),
            Some((true, "WOOD"))
        );
        let books = spellbooks(&gd);
        let hoe = gd.object_id("SPELLBOOK_HOE").expect("SPELLBOOK_HOE");
        assert_eq!(books.get(&hoe).map(String::as_str), Some("POWER_HAND"));
    }
}

#[cfg(test)]
impl EditorApp {
    /// Tooltip and image size of the thing palette entry labelled `label`.
    pub fn thing_palette_info(&self, label: &str) -> Option<(String, [usize; 2])> {
        let e = self.thing_palette.iter().find(|e| e.label == label)?;
        Some((
            e.tooltip.clone(),
            e.tex.as_ref().map_or([0, 0], |t| t.size()),
        ))
    }
}
