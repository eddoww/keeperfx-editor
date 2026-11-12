//! Builds the Lua annotation index of the stock install.
//!
//! Regenerate the shipped `data/lua_api.json` (embedded by `ApiIndex::stock`) with:
//!
//! ```text
//! KFX_LUA_REGEN=1 cargo test -p kfx-script --test lua_index_gen -- --nocapture
//! ```
//!
//! Without `KFX_LUA_REGEN` the test only checks that the shipped file loads and (when an install is
//! present) that it still matches a fresh build.
use kfx_formats::testutil;
use kfx_script::lua::index::ApiIndex;
use std::path::PathBuf;
use std::time::Instant;

fn data_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/lua_api.json")
}

fn lua_root() -> Option<PathBuf> {
    let d = testutil::install_dir()?;
    let r = testutil::resolve_ci(&d, "fxdata/lua").or_else(|| Some(d.join("fxdata/lua")))?;
    r.is_dir().then_some(r)
}

#[test]
fn build_and_check_stock_index() {
    let Some(root) = lua_root() else {
        eprintln!("skip: no install");
        return;
    };
    let t = Instant::now();
    let ix = ApiIndex::build(&[root.as_path()]);
    let dt = t.elapsed();
    println!(
        "index build: {:?} ({} files, {} functions, {} classes, {} aliases, {} globals, {} enums)",
        dt,
        ix.files.len(),
        ix.functions.len(),
        ix.classes.len(),
        ix.aliases.len(),
        ix.globals.len(),
        ix.enums.len()
    );
    let json = ix.to_json();
    if std::env::var_os("KFX_LUA_REGEN").is_some() {
        std::fs::write(data_file(), &json).unwrap();
        println!("wrote {} ({} bytes)", data_file().display(), json.len());
    } else {
        let shipped = std::fs::read_to_string(data_file()).unwrap();
        let shipped_ix = ApiIndex::from_json(&shipped).expect("data/lua_api.json parses");
        assert_eq!(
            shipped_ix, ix,
            "data/lua_api.json is stale: regenerate with KFX_LUA_REGEN=1 (see the file docs)"
        );
    }
}
