# Windows packaging notes

There is no Windows installer. CI builds and tests the editor on Windows (x86_64, MSVC), and the
release workflow publishes a zip with the executable and the documentation.

## Build

```powershell
rustup toolchain install stable        # Rust 1.99 or newer (rust-version in Cargo.toml)
cargo build --release -p kfx-editor
```

CI uses the MSVC toolchain (`x86_64-pc-windows-msvc`, which needs the Visual Studio Build Tools);
the GNU toolchain (`x86_64-pc-windows-gnu`, MinGW) is not tested. The binary is
`target\release\kfx-editor.exe`. It is built as a console program (there is no
`windows_subsystem` attribute), which is what the command line modes (`validate`, `info`,
`script-check`, `export-image`, ...) need; starting the editor window from Explorer therefore also
opens a console window. If that is unwanted, a release can add `#![windows_subsystem = "windows"]`
to `main.rs`, but then the command line modes print nothing when started from a terminal; a
second launcher executable would avoid both.

## What to ship

A zip with:

* `kfx-editor.exe`
* `LICENSE`, `THIRD_PARTY.md`, `README.md`, `CHANGELOG.md` and the user manual (`docs/manual`).

The editor needs a KeeperFX install and ships no game assets. The first run asks for the folder
(or set the `KFX_INSTALL` environment variable). Settings, backups and autosaves are written to the
platform configuration folder, `%APPDATA%\KeeperFX\kfx-editor\config`.

## Optional: associate `mapNNNNN.slb`

The Linux packaging registers a MIME type for `map?????.slb`. On Windows `.slb` is a generic
extension, so associating it with the editor affects every `.slb` file; do it per user and only
if the user asks. A per-user registration (adjust the path) looks like:

```reg
Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\Classes\KeeperFX.Map\shell\open\command]
@="\"C:\\Tools\\kfx-editor\\kfx-editor.exe\" \"%1\""

[HKEY_CURRENT_USER\Software\Classes\.slb]
@="KeeperFX.Map"
```

The editor accepts the path of any file of the level as its first argument and opens the level.

## Playing from the editor

On Windows the game is started directly (`keeperfx.exe`, found in the install folder); no
`wine` is involved. Preferences > Play can set the executable, extra arguments and a window size.
