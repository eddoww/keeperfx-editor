# Updates

The editor can look for new versions, download them and install them. It only ever talks to
GitHub, where the releases of the editor are published:
<https://github.com/eddoww/keeperfx-editor/releases>.

## What happens, and when

* **Nothing is sent before you agree.** The first time the editor window starts (after the
  KeeperFX folder question), a notice explains that the editor would ask GitHub once a day
  whether a new version exists, with two buttons: **Check for updates once a day** and **Turn
  update checks off**. Until you answer, no request is made.
* **Once a day at most.** With automatic checks on, the editor asks GitHub for its list of
  releases when the window starts, at most once every 24 hours. The request goes to
  `api.github.com` and carries what any web request carries (your IP address) plus the editor's
  version in the `User-Agent` header. No account, no identifier, nothing about your maps.
* **You decide.** When a newer version exists, a small banner says so (top right). **Details...**
  opens the Updates window; **Later** hides the banner until the next start; **Skip this version**
  never mentions that version again (a newer one is offered as usual).
* **Help > Check for updates...** asks immediately, whatever the settings, and opens the Updates
  window.

## The Updates window

It shows this version, the new one, and the release notes from GitHub (as plain text: nothing in
them is clickable or run). The buttons:

* **Update now**: downloads the new version (with a progress bar and **Cancel**), checks it (see
  [Security](#security)), then installs it. If the open map has unsaved changes, the usual question
  comes first (Save, Discard or Cancel). After installing, the editor closes and the new version
  starts, opening the same map again.
* **Later** and **Skip this version**: as in the banner.
* **Roll back to X.Y.Z**: shown while the previous version is still kept (see below).
* **Open the download page**: instead of Update now when this copy cannot update itself (see
  below).

A failed download or installation changes nothing: the current version keeps working, and the
reason is shown in the window. An interrupted download continues where it stopped next time.

## Preferences > Updates

* **Check for updates automatically** (on by default, after the first-run notice).
* **Channel**: *pre-release* (the default) offers every published release; *stable* only the
  releases that are not marked as pre-releases. Every release so far is a pre-release, so on the
  stable channel no update is offered until a stable release exists.
* **Download and install automatically** (off by default): a new version is downloaded in the
  background and installed when you close the editor; the next start runs it. When off, you are
  always asked first.
* The skipped versions (with **Forget**), **Check now**, the time of the last check and the last
  error.

To turn update checks off, untick **Check for updates automatically**. The editor then contacts
GitHub only when you use **Check now** or Help > Check for updates.

## When the editor does not update itself

In these cases the Updates window only says that a new version exists and offers its download
page:

* the folder of the editor cannot be written (for example a system folder);
* the editor was installed by a package manager (a `managed-install` file next to the program, or
  in `<prefix>/share/kfx-editor/` for a program in `<prefix>/bin`; the Arch Linux package has
  one): update it with the package manager;
* it runs from a read-only place: a disk image or AppImage, or a macOS application that has not
  been moved out of its download folder (macOS "App Translocation"). Move the application into
  Applications or another folder of yours first;
* it is a development build (built from source in debug mode, or a version with a suffix such as
  `-dev`);
* no release is built for the operating system or processor.

## What is installed, and the way back

* **Windows, installed with the setup program**: the editor downloads the new setup program,
  checks it, and runs it silently; it closes the editor, replaces the files, updates the entry in
  Apps & features, and starts the editor again.
* **Windows zip, Linux, macOS**: the editor replaces its own files: the program and the files
  shipped with it (README, licence, changelog, `docs/`), or the whole `KeeperFX Map Editor.app`
  when it runs from the application bundle. Files of yours in the same folder are not touched.
  The swap is all or nothing: if any step fails, everything is put back as it was.

With the zip, Linux and macOS downloads, the previous version is kept in a hidden folder
`.kfx-editor-previous` next to the program (next to the `.app` on macOS) until the new version has started and run for a few seconds. Until then,
the Updates window offers **Roll back to X.Y.Z**, which puts the previous version back and starts
it. If the new version does not start at all, roll back by hand: close the editor, then move the
items inside `.kfx-editor-previous` (except `.version` and `.items`) back into the folder,
replacing the new ones. You can also always download any earlier version from the releases page; with the Windows
installer, that is the way back (run the older setup; it installs over the newer version).

## On the command line

```sh
kfx-editor update --check     # prints this version and the newest; exit code 0: up to date,
                              # 10: an update is available, 1: error (offline, rate-limited, ...)
kfx-editor update             # downloads, checks and installs the newest version
kfx-editor update --restart path/to/map00001.slb   # ... then starts it on that map
kfx-editor update --channel stable --check
kfx-editor --version
```

On Windows use `kfx-editor-cli.exe` for these (see [Command line](cli.md)).

## Security

Every download is checked before anything is installed:

* only HTTPS connections to GitHub's own hosts are made (`api.github.com`, `github.com`, and
  `release-assets.githubusercontent.com` / `objects.githubusercontent.com`, where GitHub serves
  release files), redirects included;
* the size of the file must match the release, and its SHA-256 checksum must match the line for
  it in the `SHA256SUMS.txt` published in the same release. A release without that file, or a
  file not listed in it, is refused; so is a mismatch, and the download is deleted;
* archives are unpacked with the same checks as Workshop downloads: no absolute paths, no `..`,
  no links, size limits.

What this protects against: damaged or truncated downloads, and anything between you and GitHub
that would alter the files. What it does not protect against: someone who takes over the GitHub
account or repository of the editor could publish a release with matching checksums. The
releases are not signed with a separate offline key yet; [SECURITY.md](../../SECURITY.md)
describes the trust model and how signing could be added.

The Windows and macOS programs are not code-signed either: Windows SmartScreen warns the first
time ("Windows protected your PC": choose **More info**, then **Run anyway**), and macOS needs a
right-click and **Open** the first time. See [Getting started](getting-started.md).

## Files

In the editor's configuration folder (Preferences > Files shows it):

* `update-state.json`: time of the last check, the ETag of GitHub's last answer, the back-off
  after errors or rate limits; `update-releases.json`: the last list of releases;
* `update-downloads/`: downloads in progress (removed after installing or at the next start);
* `update-health.json`: present between installing an update and the new version's first clean
  start;
* `crashes/`: crash logs. If the editor stops because of an error, it writes a log there and says
  so at the next start, with the path, to attach to a bug report.
