//! Cross-checks the annotation index against what the C code registers
//! (`data/lua_c_api.json`, produced by `python3 -I tools/extract_lua_api.py`).
//!
//! Reports: stubs without a C function, C functions without a stub, class members both ways, and
//! the commented-out registrations. Prints the lists (run with `--nocapture`).
use kfx_formats::testutil;
use kfx_script::lua::index::ApiIndex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn load_c() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua_c_api.json");
    serde_json::from_str(
        &std::fs::read_to_string(p).expect("data/lua_c_api.json (run tools/extract_lua_api.py)"),
    )
    .unwrap()
}

fn names(v: &Value, commented: bool) -> BTreeSet<String> {
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["commented"].as_bool().unwrap() == commented)
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

fn index() -> ApiIndex {
    if let Some(d) = testutil::install_dir() {
        let r: PathBuf = d.join("fxdata/lua");
        if r.is_dir() {
            return ApiIndex::build(&[r.as_path()]);
        }
    }
    ApiIndex::stock()
}

/// Classes of the stub library that are all backed by `lua_api_things.c`.
const THING_FAMILY: &[&str] = &[
    "Thing",
    "Creature",
    "Object",
    "Corpse",
    "Effect",
    "EffectElem",
    "EffectGen",
    "Shot",
    "Trap",
];

#[test]
fn lua_c_crosscheck() {
    let c = load_c();
    let ix = index();
    let c_live = names(&c["globals"], false);
    let c_commented = names(&c["globals"], true);

    // stubs = functions defined in bindings/ (`---@meta` files describing the C API)
    let stubs: BTreeMap<&String, bool> = ix
        .functions
        .iter()
        .filter(|(_, f)| {
            f.source
                .as_ref()
                .map_or(false, |s| s.file.starts_with("bindings/"))
        })
        .map(|(n, _)| (n, true))
        .collect();
    let stub_names: BTreeSet<String> = stubs.keys().map(|s| s.to_string()).collect();
    let no_c: Vec<&String> = stub_names.iter().filter(|n| !c_live.contains(*n)).collect();
    let no_stub: Vec<&String> = c_live
        .iter()
        .filter(|n| !ix.functions.contains_key(*n))
        .collect();
    let stub_for_commented: Vec<&String> = c_commented
        .iter()
        .filter(|n| ix.functions.contains_key(*n))
        .collect();
    let case_only: Vec<(&String, &String)> = no_stub
        .iter()
        .filter_map(|n| {
            ix.functions
                .keys()
                .find(|k| k.eq_ignore_ascii_case(n))
                .map(|k| (*n, k))
        })
        .collect();

    println!(
        "C live globals: {}, commented-out: {}, stubs (bindings/): {}",
        c_live.len(),
        c_commented.len(),
        stub_names.len()
    );
    println!("\nstubs with no C function ({}): {:?}", no_c.len(), no_c);
    println!(
        "\nC functions with no stub ({}): {:?}",
        no_stub.len(),
        no_stub
    );
    println!(
        "\ncommented-out registrations ({}): {:?}",
        c_commented.len(),
        c_commented
    );
    println!(
        "commented-out registrations that still have a stub: {:?}",
        stub_for_commented
    );
    println!("name differs only by case: {:?}", case_only);

    // class members
    let c_fields = |cls: &str, which: &str| -> BTreeSet<String> {
        c["fields"][cls][which]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| e["name"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let c_methods = |cls: &str, commented: bool| -> BTreeSet<String> {
        c["methods"]
            .get(cls)
            .map(|v| names(v, commented))
            .unwrap_or_default()
    };
    let families: Vec<(&str, Vec<&str>)> = vec![
        ("Player", vec!["Player"]),
        ("Camera", vec!["Camera"]),
        ("Room", vec!["Room"]),
        ("Slab", vec!["Slab"]),
        ("Map", vec!["Map"]),
        ("Thing", THING_FAMILY.to_vec()),
    ];
    let mut total_stub_members = 0usize;
    let mut total_unmatched = 0usize;
    for (cname, stub_classes) in &families {
        let get = c_fields(cname, "get");
        let set = c_fields(cname, "set");
        let live_methods = c_methods(cname, false);
        let comm_methods = c_methods(cname, true);
        let mut stub_fields: BTreeSet<String> = BTreeSet::new();
        let mut stub_methods: BTreeSet<String> = BTreeSet::new();
        for sc in stub_classes {
            for cd in ix.class_chain(sc) {
                if cd.name != *sc && !stub_classes.contains(&cd.name.as_str()) {
                    continue;
                }
                stub_fields.extend(cd.fields.keys().cloned());
                stub_methods.extend(
                    cd.methods
                        .keys()
                        .filter(|m| !m.starts_with("__") && !m.starts_with("On"))
                        .cloned(),
                );
            }
        }
        let mut c_all_fields: BTreeSet<String> = get.union(&set).cloned().collect();
        // raw table fields written by lua_push<Class> (lua_params.c): slb_x, room_idx, ThingIndex,
        // ...
        if let Some(raw) = c["raw_fields"][cname].as_array() {
            c_all_fields.extend(
                raw.iter()
                    .map(|n| n.as_str().unwrap().to_string())
                    .filter(|n| !n.starts_with("__")),
            );
        }
        let dynamic = c["fields"][cname]["dynamic_player_variables"]
            .as_bool()
            .unwrap_or(false);
        let stub_fields_no_c: Vec<&String> = stub_fields
            .iter()
            .filter(|f| {
                !c_all_fields.contains(*f)
                    && !(dynamic
                        && f.chars()
                            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_'))
            })
            .collect();
        let c_fields_no_stub: Vec<&String> = c_all_fields
            .iter()
            .filter(|f| !stub_fields.contains(*f) && !stub_methods.contains(*f))
            .collect();
        let stub_methods_no_c: Vec<&String> = stub_methods
            .iter()
            .filter(|m| !live_methods.contains(*m) && !c_all_fields.contains(*m))
            .collect();
        let c_methods_no_stub: Vec<&String> = live_methods
            .iter()
            .filter(|m| !stub_methods.contains(*m))
            .collect();
        println!("\n== class {cname} (stub classes {stub_classes:?}) ==");
        println!(
            "  stub fields with no C field ({}): {:?}",
            stub_fields_no_c.len(),
            stub_fields_no_c
        );
        println!(
            "  C fields with no stub field ({}): {:?}",
            c_fields_no_stub.len(),
            c_fields_no_stub
        );
        println!(
            "  stub methods with no C method ({}): {:?}",
            stub_methods_no_c.len(),
            stub_methods_no_c
        );
        println!(
            "  C methods with no stub method ({}): {:?}",
            c_methods_no_stub.len(),
            c_methods_no_stub
        );
        println!(
            "  commented-out methods: {:?}{}",
            comm_methods,
            if dynamic {
                "   (Player variables like MONEY are dynamic: parse_get_varib)"
            } else {
                ""
            }
        );
        total_stub_members += stub_fields.len() + stub_methods.len();
        total_unmatched += stub_fields_no_c.len() + stub_methods_no_c.len();
    }

    // hooks: library hook stubs and the C list
    let c_hooks: BTreeSet<String> = c["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["name"].as_str().unwrap().to_string())
        .collect();
    let ours: BTreeSet<String> = kfx_script::lua::hooks::HOOKS
        .iter()
        .map(|h| h.name.to_string())
        .collect();
    let missing: Vec<&String> = c_hooks
        .iter()
        .filter(|h| !ours.contains(*h) && !h.starts_with("OnModGameStart_"))
        .collect();
    let extra: Vec<&String> = ours.iter().filter(|h| !c_hooks.contains(*h)).collect();
    println!("\nhooks in C but not in hooks.rs: {missing:?}; in hooks.rs but not in C: {extra:?}");

    // sanity of the extractor: most stubs must match a C registration
    assert!(c_live.len() > 100, "extractor lost registrations");
    // A hook new upstream is listed by the sync report and only printed here (the editor works
    // without knowing it); a hook the editor offers that the game no longer calls is an error.
    assert!(
        extra.is_empty(),
        "hooks.rs offers hooks src/lua_triggers.c no longer calls: {extra:?}"
    );
    assert!(
        no_c.len() * 4 < stub_names.len().max(1),
        "too many unmatched stubs: extractor or index broken"
    );
    assert!(
        total_unmatched * 3 < total_stub_members.max(1),
        "too many unmatched class members"
    );
}
