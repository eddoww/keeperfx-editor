use kfx_formats::level_fx::*;
use kfx_formats::testutil::install_dir;
use std::path::PathBuf;

fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if let Some(x) = p.extension() {
                let x = x.to_string_lossy().to_ascii_lowercase();
                if matches!(x.as_str(), "tngfx" | "aptfx" | "lgtfx") {
                    out.push(p);
                }
            }
        }
    }
}

fn check<T: FxItem + std::fmt::Debug>(
    raw: &[u8],
    name: &str,
    exact: &mut usize,
    total: &mut usize,
    items: &mut usize,
) {
    let a = FxFile::<T>::from_bytes(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(a.warnings.is_empty(), "{name}: {:?}", a.warnings);
    // the port of the game's reader and a TOML reader see the same file
    let src = String::from_utf8_lossy(raw);
    assert_eq!(
        kfx_formats::game_toml::same_as_toml(&src),
        Ok(()),
        "{name}: original"
    );
    let text = a.to_text();
    assert_eq!(
        kfx_formats::game_toml::same_as_toml(&text),
        Ok(()),
        "{name}: written"
    );
    let b = FxFile::<T>::parse(&text).unwrap();
    assert_eq!(a.items, b.items, "{name}: semantic round trip");
    assert_eq!(a.form, b.form);
    *total += 1;
    *items += a.items.len();
    if text.as_bytes() == raw {
        *exact += 1;
    } else {
        let t = text.as_bytes();
        let at = t
            .iter()
            .zip(raw)
            .position(|(x, y)| x != y)
            .unwrap_or(t.len().min(raw.len()));
        println!(
            "{name}: not byte exact at {at} (len {} vs {})",
            t.len(),
            raw.len()
        );
    }
}

#[test]
fn install_files_roundtrip() {
    let Some(root) = install_dir() else { return };
    let mut files = vec![];
    walk(&root, &mut files);
    files.sort();
    let (mut te, mut tt, mut ti) = (0, 0, 0);
    let (mut ae, mut at, mut ai) = (0, 0, 0);
    let (mut le, mut lt, mut li) = (0, 0, 0);
    for f in &files {
        let raw = kfx_formats::rnc::load_file(f).unwrap();
        let name = f.strip_prefix(&root).unwrap().display().to_string();
        match f
            .extension()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "tngfx" => check::<FxThing>(&raw, &name, &mut te, &mut tt, &mut ti),
            "aptfx" => check::<FxActionPoint>(&raw, &name, &mut ae, &mut at, &mut ai),
            _ => check::<FxLight>(&raw, &name, &mut le, &mut lt, &mut li),
        }
    }
    println!(
        "tngfx: {te}/{tt} byte-exact, {ti} things; aptfx: {ae}/{at} byte-exact, {ai} points; \
        lgtfx: {le}/{lt} byte-exact, {li} lights"
    );
    assert!(tt + at + lt > 0);
}

#[test]
fn array_form_extras_and_legacy() {
    let src = r#"
[common]
Version = 2
[[thing]]
ThingType = 5
Subtype = 6
Ownership = 1
CreatureLevel = 3
Wibble = "x"
SubtileX = [4, 128]
SubtileY = [5, 128]
SubtileZ = [1, 0]
[[thing]]
ThingType = "Door"
Subtype = "oops"
Ownership = "bad"
"#;
    let f = TngFx::parse(src).unwrap();
    assert_eq!(f.form, FxForm::Array);
    assert_eq!(f.items.len(), 2);
    assert_eq!(f.items[0].class.as_ref().unwrap().id(), Some(5));
    assert!(f.items[0].extra.contains_key("Wibble"));
    assert_eq!(f.items[1].subtype, None);
    assert!(f.items[1].extra.contains_key("Subtype") && f.items[1].extra.contains_key("Ownership"));
    assert_eq!(f.warnings.len(), 2);
    let g = TngFx::parse(&f.to_text()).unwrap();
    assert_eq!(f.items, g.items);
    assert_eq!(f.common_extra, g.common_extra);
    assert!(TngFx::parse("x = 1").is_err());
    assert!(
        TngFx::parse("[common]\nThingsCount = 1\n")
            .unwrap()
            .warnings
            .len()
            == 1
    );

    let l = LegacyThingFields {
        x: 0x3D80,
        y: 0x0A80,
        z: 0x0500,
        oclass: 9,
        model: 3,
        owner: 4,
        range: 0,
        index: 309,
        params: [0, 1, 0, 0, 0, 0, 0, 0],
    };
    let t = FxThing::from_legacy(&l, ObjectRole::Plain);
    assert_eq!(t.x, Some(StlCoord::new(61, 128)));
    assert_eq!(t.door_locked, Some(1));
    assert_eq!(t.to_legacy(), Some(l));
    let cr = LegacyThingFields {
        x: 1,
        y: 2,
        z: 3,
        oclass: 5,
        model: 6,
        owner: 2,
        range: 0,
        index: 7,
        params: [0, 4, 0, 0, 0, 0, 0, 0],
    };
    let t = FxThing::from_legacy(&cr, ObjectRole::Plain);
    assert_eq!(t.creature_level, Some(5));
    assert_eq!(t.to_legacy(), Some(cr));
    let a = LegacyActionPointFields {
        x: 0x8580,
        y: 0x0C80,
        range: 0x03B7,
        num: 12,
    };
    assert_eq!(FxActionPoint::from_legacy(&a).to_legacy(), Some(a));
    assert_eq!(
        FxActionPoint::from_legacy(&a).range,
        Some(StlCoord::new(3, 183))
    );
    let li = LegacyLightFields {
        radius: 0x1000,
        intensity: 26,
        flags: 0,
        x: 1,
        y: 2,
        z: 3,
        is_dynamic: 1,
        attached_slb: 115,
    };
    assert_eq!(FxLight::from_legacy(&li).to_legacy(), Some(li));
}
