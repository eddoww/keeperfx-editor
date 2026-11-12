//! `kfx-editor`: the editor window and, outside Windows, the command line too. On Windows it is
//! built for the GUI subsystem, so starting it opens no console window; `kfx-editor-cli.exe` is
//! the console program for the command line there (see `kfx_editor::console`).
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    kfx_editor::main_entry(kfx_editor::Exe::Gui);
}
