# Packaging

| File | Purpose |
|---|---|
| `org.keeperfx.MapEditor.desktop` | freedesktop launcher; `Exec=kfx-editor %f`, `MimeType=application/x-keeperfx-map` |
| `org.keeperfx.MapEditor.mime.xml` | shared-mime-info type for `map?????.slb` (install to `/usr/share/mime/packages/`) |
| `org.keeperfx.MapEditor.metainfo.xml` | AppStream metadata, declares the media type |
| `PKGBUILD` | Arch Linux package that builds from a local checkout of this repository |
| `windows.md` | Windows build and packaging notes |
| `macos/Info.plist` | bundle metadata for `KeeperFX Map Editor.app` (the release workflow builds the unsigned bundle) |

The desktop entry passes `desktop-file-validate`, and the MIME file compiles with
`update-mime-database`. No icon is shipped yet: `Icon=org.keeperfx.MapEditor` falls back to a
generic icon.

The component id `org.keeperfx.MapEditor` follows the reverse-DNS convention of AppStream and
freedesktop. AppStream expects the id to belong to a domain the project controls; a distribution
that publishes the metadata in a store may need to change it.

## Release archives

`.github/workflows/release.yml` builds the archives attached to a GitHub release when a `v*` tag
is pushed: `kfx-editor-<tag>-linux-x86_64.tar.gz` (with the desktop entry and the MIME file),
`kfx-editor-<tag>-windows-x86_64.zip`, and `kfx-editor-<tag>-macos-arm64.tar.gz` and
`-macos-x86_64.tar.gz` (with the executable and an unsigned application bundle). Each holds the
README, changelog, licence, `THIRD_PARTY.md` and the user manual, and the release lists their
SHA-256 checksums.

The tests a package build can run without game data are described in
[CONTRIBUTING.md](../CONTRIBUTING.md#testing).
