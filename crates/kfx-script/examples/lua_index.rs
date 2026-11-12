//! Builds the Lua annotation index of a folder of Lua stubs (an install's `fxdata/lua`, or the
//! `config/fxdata/lua` folder of the KeeperFX source) and writes it as JSON.
//!
//! ```text
//! cargo run -p kfx-script --example lua_index -- LUA_FOLDER OUT.json
//! ```
//!
//! `tools/sync_keeperfx.py` runs it to regenerate `data/lua_api.json`; the `lua_index_gen` test
//! checks that file against an install. The Lua files are only read and parsed, never run.
use kfx_script::lua::index::ApiIndex;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, out] = args.as_slice() else {
        eprintln!("usage: lua_index LUA_FOLDER OUT.json");
        std::process::exit(2);
    };
    let root = PathBuf::from(root);
    if !root.is_dir() {
        eprintln!("lua_index: {}: not a folder", root.display());
        std::process::exit(2);
    }
    let ix = ApiIndex::build(&[root.as_path()]);
    if ix.files.is_empty() {
        eprintln!("lua_index: {}: no Lua files", root.display());
        std::process::exit(1);
    }
    let json = ix.to_json();
    if let Err(e) = std::fs::write(out, &json) {
        eprintln!("lua_index: {out}: {e}");
        std::process::exit(1);
    }
    println!(
        "wrote {out}: {} files, {} functions, {} classes",
        ix.files.len(),
        ix.functions.len(),
        ix.classes.len()
    );
}
