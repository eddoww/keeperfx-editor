//! Mod layers (`mods/load_order.cfg`) with the two demo mods of the KeeperFX source tree
//! (`config/mods`), layered configs with provenance, and typed creature configs.
//!
//! The tests that need the demo mods read them from the KeeperFX source checkout named by
//! `KFX_SOURCE` and the install named by `KFX_INSTALL`; they skip when either is unset.
use kfx_formats::testutil;
use kfx_map::creature::{SpriteRef, PROPERTY_NAMES};
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mods::{CampaignLocations, LayerOrigin, LayerStack, ModsConfig, Stage};
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(not(unix))]
fn symlink(_: PathBuf, _: PathBuf) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// The KeeperFX source checkout (`KFX_SOURCE`), `None` when unset.
fn source_dir() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("KFX_SOURCE")?);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// A temp install: fxdata / creatrs are symlinks into the real (read-only) install, `mods/` holds
/// the demo mods of the source tree with the load order activated, plus a campaign config folder
/// and a levels folder with small overrides.
fn temp_install(tag: &str) -> Option<(PathBuf, PathBuf)> {
    if !cfg!(unix) {
        return None; // the temp install symlinks the real one (unix only): skip elsewhere
    }
    let real = testutil::install_dir()?;
    let Some(source) = source_dir() else {
        eprintln!("skip: KFX_SOURCE (KeeperFX source checkout) is not set");
        return None;
    };
    let demo = source.join("config/mods");
    if !demo.is_dir() {
        eprintln!("skip: no config/mods in {}", source.display());
        return None;
    }
    let root = testutil::scratch_dir(&format!("kfx-map/mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for d in ["fxdata", "creatrs"] {
        symlink(testutil::resolve_ci(&real, d).unwrap(), root.join(d)).unwrap();
    }
    copy_dir(&demo, &root.join("mods"));
    // the shipped file is a disabled example: renamed it becomes active
    std::fs::rename(
        root.join("mods/_load_order.cfg"),
        root.join("mods/load_order.cfg"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("campcfg")).unwrap();
    std::fs::create_dir_all(root.join("lvls")).unwrap();
    std::fs::write(
        root.join("campcfg/rules.cfg"),
        "[game]\nGoldPileValue = 111\n[magic]\nHoldAudienceTime = 50\n",
    )
    .unwrap();
    std::fs::write(
        root.join("lvls/map00001.rules.cfg"),
        "[game]\nGoldPileValue = 222\n",
    )
    .unwrap();
    Some((real, root))
}

fn loc() -> CampaignLocations {
    CampaignLocations {
        configs: Some("campcfg".into()),
        levels: Some("lvls".into()),
        creatures: None,
    }
}

#[test]
fn load_order_parsing_follows_the_game() {
    let t = "; header\n\n[after_base]\nmod_a\n  mod_b  \n; commented\nmod_c\n[AFTER_CAMPAIGN]\n; \
        none\n[after_map]\nmod_d ; not a comment\n";
    let m = ModsConfig::parse(t.as_bytes());
    assert_eq!(m.after_base, vec!["mod_a", "mod_b", "mod_c"]);
    assert!(m.after_campaign.is_empty());
    assert_eq!(m.after_map, vec!["mod_d ; not a comment"]);
    assert!(ModsConfig::parse(b"no blocks here").is_empty());
    // > 50 mods: the rest is dropped; names of 64+ bytes are ignored
    let many: String = std::iter::once("[after_base]\n".to_string())
        .chain((0..60).map(|i| format!("m{i}\n")))
        .collect();
    let m = ModsConfig::parse(many.as_bytes());
    assert_eq!(m.after_base.len(), 50);
    let long = format!("[after_base]\n{}\nok\n", "x".repeat(64));
    assert_eq!(ModsConfig::parse(long.as_bytes()).after_base, vec!["ok"]);
    // multiplayer ignores mods
    assert!(m.for_game(true).is_empty());
    assert!(!m.for_game(false).is_empty());
}

#[test]
fn demo_mods_layers_and_provenance() {
    let Some((_, root)) = temp_install("layers") else {
        return;
    };
    let mods = ModsConfig::load(&root).expect("load_order.cfg");
    assert_eq!(mods.after_base, vec!["tunneller_imp_swap"]);
    assert!(mods.after_campaign.is_empty());
    assert_eq!(mods.after_map, vec!["instant_charge_up"]);
    let stack = LayerStack::build(&root, &loc(), Some(1), &mods);
    let order: Vec<String> = stack.config.iter().map(|(o, _)| o.to_string()).collect();
    println!("config layers: {order:?}");
    assert_eq!(
        order,
        vec![
            "base",
            "mod tunneller_imp_swap (after_base)",
            "campaign",
            "this map",
            "mod instant_charge_up (after_map)"
        ]
    );
    let corder: Vec<String> = stack.creatures.iter().map(|(o, _)| o.to_string()).collect();
    assert_eq!(
        corder,
        vec!["base", "mod tunneller_imp_swap (after_base)", "this map"]
    );

    // rules.cfg: base < campaign < map for GoldPileValue; the after_map mod flips a magic rule
    let rules = {
        let files = stack.files("rules.cfg");
        kfx_map::layered::LayeredCfg::from_cfg_files(files).unwrap()
    };
    let v = rules.get("game", "GoldPileValue").unwrap();
    assert_eq!(v.value, "222");
    assert_eq!(
        rules.origin("game", "GoldPileValue"),
        Some(&LayerOrigin::map())
    );
    assert!(rules.is_overridden("game", "GoldPileValue"));
    let hist: Vec<(String, String)> = rules
        .history("game", "GoldPileValue")
        .into_iter()
        .map(|(o, v)| (o.to_string(), v.to_string()))
        .collect();
    println!("GoldPileValue history: {hist:?}");
    assert_eq!(hist.len(), 3);
    assert_eq!(hist[0].0, "base");
    assert_eq!(hist[1], ("campaign".to_string(), "111".to_string()));
    assert_eq!(hist[2], ("this map".to_string(), "222".to_string()));
    assert_eq!(rules.value("magic", "HoldAudienceTime"), Some("50"));
    assert_eq!(
        rules.origin("magic", "HoldAudienceTime"),
        Some(&LayerOrigin::campaign())
    );
    let o = rules.origin("magic", "AllowInstantChargeUp").unwrap();
    assert_eq!(
        (o.stage, o.mod_name.as_deref()),
        (Stage::AfterMap, Some("instant_charge_up"))
    );
    assert_eq!(rules.value("magic", "AllowInstantChargeUp"), Some("1"));
    // a key nobody overrides comes from the base layer
    let any_base = rules
        .sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .find(|(_, v)| v.overridden.is_empty())
        .unwrap();
    assert_eq!(rules.layers[any_base.1.layer].origin, LayerOrigin::base());

    // magic.cfg: the after_base mod swaps the use functions of imp / tunneller
    let magic = kfx_map::layered::LayeredCfg::from_cfg_files(stack.files("magic.cfg")).unwrap();
    assert_eq!(
        magic.value("power2", "UseFunction"),
        Some("magic_use_power_tunneller")
    );
    assert_eq!(
        magic
            .origin("power2", "UseFunction")
            .unwrap()
            .mod_name
            .as_deref(),
        Some("tunneller_imp_swap")
    );
    assert!(magic.history("power2", "UseFunction").len() >= 2);

    // multiplayer: mods are ignored
    let mp = LayerStack::build(&root, &loc(), Some(1), &mods.for_game(true));
    assert_eq!(mp.config.len(), 3);
    let rules_mp = kfx_map::layered::LayeredCfg::from_cfg_files(mp.files("rules.cfg")).unwrap();
    assert_ne!(rules_mp.value("magic", "AllowInstantChargeUp"), Some("1"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn creature_layers_with_the_swap_mod() {
    let Some((_, root)) = temp_install("crtr") else {
        return;
    };
    let mods = ModsConfig::load(&root).unwrap();
    let stack = LayerStack::build(&root, &loc(), Some(1), &mods);
    let gd = GameData::load_stack(&stack).unwrap();
    let imp = gd.creature_config("IMP").expect("imp");
    assert_eq!(imp.name, "imp");
    assert_eq!(imp.attributes.lair_object, "LAIR_TUNLR");
    assert_eq!(
        imp.origin("attributes", "LairObject")
            .unwrap()
            .mod_name
            .as_deref(),
        Some("tunneller_imp_swap")
    );
    assert_eq!(imp.attributes.name_text_id, 546);
    assert_eq!(imp.sprite("Stand"), Some(&SpriteRef::Index(338)));
    // untouched keys keep the base value and report the base layer
    assert_eq!(imp.attributes.health, 75);
    assert_eq!(
        imp.origin("attributes", "Health"),
        Some(&LayerOrigin::base())
    );
    assert!(imp.cfg.is_overridden("sprites", "Stand"));
    assert_eq!(
        imp.cfg
            .history("sprites", "Stand")
            .iter()
            .map(|(_, v)| v.to_string())
            .collect::<Vec<_>>(),
        vec!["556", "338"]
    );
    // cached: the same Arc comes back
    assert!(Arc::ptr_eq(&imp, &gd.creature_config("imp").unwrap()));
    // a clone of the game data keeps working
    assert!(gd.clone().creature_config("tunneller").is_some());
    // without mods the imp is the stock one
    let plain = GameData::load_stack(&LayerStack::build(
        &root,
        &loc(),
        Some(1),
        &ModsConfig::default(),
    ))
    .unwrap();
    assert_eq!(
        plain.creature_config("imp").unwrap().attributes.lair_object,
        "LAIR_IMP"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn per_map_creature_override() {
    let Some((_, root)) = temp_install("crtrmap") else {
        return;
    };
    std::fs::write(
        root.join("lvls/map00001.imp.cfg"),
        "[attributes]\nHealth = 999\n[sprites]\nStand = my_custom_stand\nQuerySymbol = my_icon\n",
    )
    .unwrap();
    let stack = LayerStack::build(&root, &loc(), Some(1), &ModsConfig::default());
    let gd = GameData::load_stack(&stack).unwrap();
    let imp = gd.creature_config("imp").unwrap();
    assert_eq!(imp.attributes.health, 999);
    assert_eq!(
        imp.origin("attributes", "Health"),
        Some(&LayerOrigin::map())
    );
    assert_eq!(
        imp.sprite("Stand"),
        Some(&SpriteRef::Name("my_custom_stand".into()))
    );
    assert_eq!(imp.query_symbol(), Some(&SpriteRef::Name("my_icon".into())));
    assert_eq!(imp.hand_symbol(), Some(&SpriteRef::Index(222)));
    assert_eq!(
        imp.attributes.strength, 5,
        "other fields keep the base value"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn every_shipped_creature_config_loads() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let gd = GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap();
    assert!(!gd.creatures.is_empty());
    let (mut loaded, mut missing, mut sprites, mut unknown) = (0, 0, 0, vec![]);
    let (mut flying, mut evil) = (0, 0);
    for (i, name) in gd.creatures.iter().enumerate() {
        match gd.creature_config_of_model(i + 1) {
            Some(c) => {
                loaded += 1;
                sprites += c.sprites.len();
                flying += c.is_flying() as usize;
                evil += c.is_evil() as usize;
                assert!(c.attributes.health > 0, "{name}: Health");
                assert!(
                    c.sprite("Stand").is_some()
                        && c.query_symbol().is_some()
                        && c.hand_symbol().is_some(),
                    "{name}: sprites"
                );
                unknown.extend(
                    c.unknown_properties()
                        .into_iter()
                        .map(|p| format!("{name}:{p}")),
                );
            }
            None => missing += 1,
        }
    }
    println!(
        "[creatures] {} models, {loaded} configs loaded, {missing} without a file, {sprites} \
        sprite entries, {flying} flying, {evil} evil, unknown properties: {unknown:?}",
        gd.creatures.len()
    );
    assert!(loaded >= 30);
    assert!(unknown.is_empty(), "{unknown:?}");
    let imp = gd.creature_config("imp").unwrap();
    assert!(imp.is_special_digger() && imp.is_evil() && !imp.is_flying());
    assert_eq!((imp.attributes.size_xy, imp.attributes.size_z), (200, 256));
    assert_eq!(
        (imp.attributes.thing_size_xy, imp.attributes.thing_size_z),
        (240, 400)
    );
    assert_eq!(imp.query_symbol(), Some(&SpriteRef::Index(154)));
    assert_eq!(imp.hand_symbol(), Some(&SpriteRef::Index(222)));
    assert_eq!(imp.appearance.transparency_flags, 0);
    assert_eq!(imp.appearance.shot_origin, [0, 0, 256]);
    assert_eq!(imp.raw("attributes", "HurtByLava"), Some("2"));
    assert!(PROPERTY_NAMES.contains(&"FLYING"));
    assert!(gd.creature_config("no_such_creature").is_none());
}
