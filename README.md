# diskatlas

A Linux disk usage analyzer written in Rust with egui/eframe and wgpu.

This is an early development build. It supports a background, read-only scan,
apparent and allocated size totals, cancellation, a three-ring sunburst, a
treemap of the current directory, a virtualized directory list, and a Largest
Files view with cached apparent/allocated size rankings and filename/minimum-size
filters. Scan errors can be expanded in the interface.

## Run

Requires Rust 1.96 or newer and a Linux graphical desktop with working graphics
drivers. Both Wayland and X11 are enabled.

```sh
cargo run -p diskatlas
```

Enter a directory path and click **Scan**. Select **Sunburst**, **Treemap**, or
**Directory** to change views. The sunburst follows Filelight: rings are coloured
by angle, tiny items fold into a grey group, and the centre circle shows the open
folder. Click a folder in a chart to open it and a file to select it; click the
centre to go up. In the directory list, click a row to select it and double-click a directory to enter it.
`diskatlas <folder>` starts scanning immediately. Use **Up** or a breadcrumb to return
to an ancestor. Empty and zero-size entries remain available in the list.

**Largest Files** searches regular files across the entire scan. Its search is
case insensitive and matches the displayed filename. Minimum size uses the
selected sorting metric. Select a result to inspect its path, then use **Show
parent directory** to locate it in the directory view.

If the default Cargo cache is read-only in your environment:

```sh
CARGO_HOME=/tmp/diskatlas-cargo cargo run -p diskatlas
```

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`diskatlas-scanner` owns traversal and metadata aggregation. The app's `chart`
module computes geometry without starting the GUI. Scan results and the child
index are produced on a worker thread; chart layouts are cached for the current
directory.

## Current limitations

Results appear after traversal completes; the spinner shows a live entry count.
Traversal uses one worker, follows a symlink only if it is the scan root, and
never descends into another device or into any mount point listed in
`/proc/self/mountinfo` (bind mounts, pseudo-filesystems); such mount points are
listed as empty leaves. Extra hard links to a file already counted add 0 bytes.
Largest Files filtering runs on a background thread. The left panel lists mounted volumes with capacity and lets you exclude absolute
folder paths from the next scan. The chosen view, the exclusions, the last scanned folder (reopened if it
still exists) and the window size (written on exit) are saved to
`$XDG_CONFIG_HOME/diskatlas/settings.conf` (default `~/.config`), together with the
theme (system, light, dark, high contrast), the UI size (also Ctrl+plus, Ctrl+minus,
Ctrl+0) and a reduce-motion option, all under Appearance in the left panel. Chart
segment colours are the same in every theme, and there is no screen-reader support
for the charts beyond the folder list. Pause/resume,
name-pattern exclusions, and unique-size accounting for reflinks and shared
extents are unfinished; on Btrfs the app only warns that shared data, snapshots and
compression make sizes approximate, and it does not enter nested subvolumes (this
warning logic has been tested on synthetic mount tables, not on a real Btrfs volume). Cancellation cannot interrupt a blocked filesystem
system call. Allocated size includes directory metadata and may count shared
storage multiple times; it is not a recoverable-space estimate.

The treemap shows immediate children using a binary partition. The sunburst
shows up to three levels. Full chart accessibility, keyboard drill-down, advanced search,
history, settings, drive discovery, cleanup, and installable packages remain
unfinished. Chart mesh generation and very wide directories need measurement
and further work. Desktop interaction has not been verified across KDE, GNOME,
Wayland, and X11. This build is not production ready.

## Debian package

`packaging/build-deb.sh` builds the release binary and assembles
`target/deb/diskatlas_<version>_<arch>.deb` with `dpkg-deb` (no debhelper needed).
It installs the binary, `diskatlas.desktop`, AppStream metadata, a scalable icon and
the copyright/changelog files. The maintainer comes from `DEBFULLNAME`/`DEBEMAIL`, or
`git config user.name`/`user.email`. Install with `sudo apt install ./target/deb/*.deb`.

The project is MIT licensed (see `LICENSE`; the package copyright file uses Debian's
name for it, Expat). Still open before a public release: pick a reverse-DNS AppStream
id (the metadata validator warns about it), build in a clean chroot, and, for
inclusion in Debian itself, a `debian/` source
package with vendored crates. The binary is built for the distribution it is built on:
its `libc6` requirement is computed from the build host.

## Continuous integration

`.github/workflows/ci.yml` runs on pushes to `main` and on pull requests. The `test` job
checks formatting, runs clippy with warnings denied, and runs the tests, all on a pinned
Rust toolchain. The `package` job then builds the `.deb` on Ubuntu 24.04, runs `lintian`
(advisory), installs the package and checks the installed files and library resolution,
and uploads it as the `diskatlas-deb` artifact. Actions are pinned to commit SHAs. A
package built there requires glibc 2.39 or newer, so it installs on older systems than
one built on a bleeding-edge machine. The workflow has not been run on GitHub yet.
