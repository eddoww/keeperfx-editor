//! `kfx-editor-cli`: the same program as `kfx-editor`, built as a console program, so that the
//! command line (`validate`, `info`, `update --check`, ...) prints to the terminal on Windows.
//! The Windows release ships it next to `kfx-editor.exe`.

fn main() {
    kfx_editor::main_entry(kfx_editor::Exe::Cli);
}
