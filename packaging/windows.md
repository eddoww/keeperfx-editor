# Windows packaging

The Windows release has two downloads, both built by `.github/workflows/release.yml` on the
`windows-latest` runner (MSVC toolchain, `x86_64-pc-windows-msvc`):

* `kfx-editor-<tag>-windows-x86_64-setup.exe`, the installer (Inno Setup), recommended for
  players;
* `kfx-editor-<tag>-windows-x86_64.zip`, the portable version.

Both are listed in the release's `SHA256SUMS.txt`, which the editor's self-update requires.

## Two executables

`cargo build --release -p kfx-editor` builds two programs from the same code
(`crates/kfx-editor/src/lib.rs`):

* `kfx-editor.exe` (`src/main.rs`) is built for the GUI subsystem
  (`#![cfg_attr(windows, windows_subsystem = "windows")]`), so starting the editor opens no
  console window. It has no console to print to: given a command (`validate`, `--help`, ...) it
  runs the same command through `kfx-editor-cli.exe` without a console window, writes the output
  to `cli-output.log` in the configuration folder and shows it in a small window.
* `kfx-editor-cli.exe` (`src/bin/kfx-editor-cli.rs`) is a console program for terminals and
  scripts.

On Linux and macOS both programs are built too, but the release archives ship only `kfx-editor`,
which does both.

Panics of the window are written to `crashes\` in the configuration folder
(`%APPDATA%\KeeperFX\kfx-editor\config`), and the next start shows the log's path.

## Icon and version information

`crates/kfx-editor/build.rs` embeds `packaging/icons/kfx-editor.ico` and version information
(product name, file description, version from `Cargo.toml`) into both executables with the
`winresource` crate. It runs only when building on Windows for an MSVC target, where the Windows
SDK's resource compiler (`rc.exe`) is found; anywhere else, or when the resource compiler is
missing, it prints a warning and the build continues without the resources.

## The installer

`packaging/windows/installer.iss` is an Inno Setup 6 script (6.3 or newer). The workflows compile
it with `packaging/windows/build-installer.ps1` and try it with
`packaging/windows/smoke-test-installer.ps1`: silent install into a temporary folder, the
expected files and the Start menu shortcut, `kfx-editor-cli.exe --version` and `update --check`
(offline or rate-limited is accepted), silent uninstall, and the folder and shortcut gone. `ci.yml`
does the same on every push (the installer is uploaded as a build artifact, nothing is
published), so a broken installer shows up before a tag. GitHub's Windows runner images ship Inno
Setup with `iscc` on `PATH`; the build script installs it with Chocolatey only if it is missing.

To build it by hand on Windows, stage the files and run the script:

```powershell
cargo build --release -p kfx-editor
$stage = "stage\kfx-editor-v0.1.2-windows-x86_64"
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\kfx-editor.exe, target\release\kfx-editor-cli.exe, README.md, CHANGELOG.md, LICENSE, THIRD_PARTY.md $stage
Copy-Item -Recurse docs "$stage\docs"
./packaging/windows/build-installer.ps1 -Version 0.1.2 -SourceDir $stage -OutputDir out -OutputBase kfx-editor-v0.1.2-windows-x86_64-setup
```

What the installer does:

* **Per user by default, no administrator prompt** (`PrivilegesRequired=lowest`): installs into
  `%LOCALAPPDATA%\Programs\KeeperFX Map Editor`, so the editor's self-update can replace it
  without elevation. The standard dialog (or `/ALLUSERS`) installs for all users into Program
  Files instead; upgrades keep the mode chosen first.
* Start menu shortcut; optional desktop shortcut (task `desktopicon`).
* Optional "Open with KeeperFX Map Editor" for `.slb` files (task `slbassoc`): the ProgID
  `KeeperFX.MapEditor.slb` and an `OpenWithProgids` value under `Software\Classes` of the current
  user (all users: the machine). `.slb` is a generic extension, so the default program of `.slb`
  is not changed. Uninstalling removes these keys.
* An Apps & features entry with the version, the publisher "KeeperFX Map Editor project", links to
  the repository and an uninstaller. The licence (`LICENSE`) is shown during installation.
* Upgrades in place: the `AppId` GUID in the script identifies the installation and must never
  change. A running editor is closed through the Restart Manager before its files are replaced
  (`CloseApplications`).
* A **Launch** checkbox on the last page.
* A `setup-install` marker file in the install folder: it tells the editor to update itself by
  downloading the newer setup and running it silently (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART
  /RELAUNCH /OPEN="<map>"`, the last two read by the script to start the editor again on the open
  map), so that the version in Apps & features stays right. Portable (zip) copies replace their
  files themselves instead.

Silent install and uninstall:

```text
kfx-editor-<tag>-windows-x86_64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
    [/DIR="x:\folder"] [/CURRENTUSER | /ALLUSERS] [/TASKS="desktopicon,slbassoc"] [/LOG="setup.log"]
"<install folder>\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
```

`/SILENT` shows only a progress window; `/VERYSILENT` shows nothing. The uninstaller removes the
program files, the shortcuts, the registry entries it made and the leftovers of the self-update
(`.kfx-editor-previous`, `.kfx-update-staging`); the editor's configuration folder (settings,
backups) is kept.

## Not signed

Neither the executables nor the installer are code-signed, so Windows SmartScreen warns on the
first start ("Windows protected your PC": **More info**, **Run anyway**). The README, the getting
started page of the manual and the release notes say so. What signing would need is described in
[CONTRIBUTING.md](../CONTRIBUTING.md#code-signing).

## Playing from the editor

On Windows the game is started directly (`keeperfx.exe`, found in the install folder); no
`wine` is involved. Preferences > Play can set the executable, extra arguments and a window size.
