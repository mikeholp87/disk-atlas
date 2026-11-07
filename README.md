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
**Directory** to change views. Click a chart segment or directory row to select
it; double-click a directory to enter it. Use **Up** or a breadcrumb to return
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

Scans show results after traversal completes. Traversal currently uses one
worker, follows a symlink only if it is the scan root, and stays on the root device. Mount-aware bind
mount handling, exclusions, hard-link deduplication, pause/resume, and incremental
progress are unfinished. Filtering is debounced but currently runs on the UI
thread; multi-million-file indexes still need background filtering and latency
measurements. Cancellation cannot interrupt a blocked filesystem
system call. Allocated size includes directory metadata and may count shared
storage multiple times; it is not a recoverable-space estimate.

The treemap shows immediate children using a binary partition. The sunburst
shows up to three levels. Full chart accessibility, keyboard drill-down, advanced search,
history, settings, drive discovery, cleanup, and installable packages remain
unfinished. Chart mesh generation and very wide directories need measurement
and further work. Desktop interaction has not been verified across KDE, GNOME,
Wayland, and X11. This build is not production ready.
