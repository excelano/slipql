# Releasing slipql

The release loop lives in `~/notes/releasing.md` — the ordered steps, the apt
step, crates.io, the winget submission, the spent-tag rule, and the standing
facts about tokens and secrets. Failure recipes are in
`~/notes/build_release_gotchas.md`. This file carries what is true of slipql
and not of its siblings.

| | |
|---|---|
| Loop | cargo-dist |
| Version lives in | `Cargo.toml` |
| `apt-ship` argument | `slipql` |
| Packages per release | 2, amd64 arm64 |
| crates | `slipql` |
| winget package | `Excelano.slipql` — `slipql-x86_64-pc-windows-msvc.zip` |

**The crate, the command, the Homebrew formula, and the apt package are all
`slipql`.** One crate holds both the library and the command; the command sits
behind the default `cli` feature, so `cargo install slipql` gets the command and
an embedding program takes `default-features = false` and gets the library
alone. cargo-dist's tarballs and installer are named after it:
`slipql-installer.sh`, `slipql-<target>.tar.xz`.

**The release builds** the five platform tarballs, the shell and PowerShell
installers, the Homebrew formula, and the checksums, then creates the GitHub
Release. The `.deb` packages come from the separately dispatched `deb.yml`.

**The floor follows `slpc`.** `rust-version` in `Cargo.toml` is `slpc`'s, which
is `zip`'s, and rises when theirs does. Raise it here when a dependency bump
makes CI's msrv job fail, not before.
