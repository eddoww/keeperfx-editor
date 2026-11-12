# Security

The editor reads files from your KeeperFX install and from downloaded Workshop archives, can talk
to a running game over a local TCP connection, and downloads and installs its own updates from
GitHub. Problems in any of these, such as an archive that writes outside the folder it is
installed into, or an update installed without passing its checks, are security issues.

## Supported versions

Only the latest release receives fixes.

## Reporting a problem

Please report security problems privately, using "Report a vulnerability" on the repository's
Security tab on GitHub, rather than in a public issue. Include the editor version, your
operating system and the steps or file needed to reproduce the problem.

## How updates are trusted

The self-update (`crates/kfx-update`, described for users in `docs/manual/updates.md`):

* talks only HTTPS (TLS through rustls, with the Mozilla root certificates of `webpki-roots`) and
  only to GitHub's hosts: `api.github.com`, `github.com`, `release-assets.githubusercontent.com`
  and `objects.githubusercontent.com`; every redirect is checked against that list;
* installs a file only when its size matches the release and its SHA-256 matches the line for it
  in the `SHA256SUMS.txt` of the same release; a release without that file, a file it does not
  list, or a mismatch is refused, and the download is deleted;
* unpacks archives with the checks used for Workshop downloads (no absolute paths, no `..`, no
  links or special files, size limits) into a staging folder, and swaps the files in all or
  nothing, keeping the previous version until the new one has started;
* never updates a development build, a package-manager install (`managed-install` marker) or a
  read-only location.

**The trust anchor is the GitHub account and repository.** The checksums come from the same
release as the files, over the same HTTPS connection, so they protect against damaged or
truncated downloads and against tampering between GitHub and the user, but not against someone
who controls the repository or the maintainer's GitHub account: such a person could publish a
release with matching checksums, and editors with automatic installation turned on would install
it. Releases are not signed with an offline key. The Windows and macOS programs are not
code-signed either.

Signatures could be added without changing this design: the maintainer would sign
`SHA256SUMS.txt` with a key kept offline (for example with minisign) and publish the signature as
a release asset, the public key would be compiled into the editor, and a signature check would be
added as an implementation of the `Verifier` trait in `crates/kfx-update/src/verify.rs`, which
already receives the sums file before it is trusted. Because every download is checked against
the sums file, signing that one file covers all of them. Editors released before that change
would keep trusting the checksums alone.

