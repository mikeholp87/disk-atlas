use diskatlas_scanner::{Scan, mounts};
mod analysis;
mod chart;
mod look;
mod settings;
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum View {
    #[default]
    Sunburst,
    Treemap,
    Directory,
    LargestFiles,
}

struct CompletedScan {
    scan: Arc<Scan>,
    children: Vec<Vec<usize>>,
    largest: Arc<analysis::LargestFiles>,
    root_path: PathBuf,
}

struct App {
    path: String,
    result: Option<Arc<Scan>>,
    receiver: Option<mpsc::Receiver<CompletedScan>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    root: usize,
    selected: Option<usize>,
    /// Node under the pointer last frame, in either the chart or a list.
    hover: Option<usize>,
    children: Vec<Vec<usize>>,
    chart_root: Option<usize>,
    segments: Vec<chart::Segment>,
    tiles: Vec<chart::Tile>,
    /// Children of the open folder, largest first.
    listing: Vec<usize>,
    view: View,
    root_path: PathBuf,
    largest: Arc<analysis::LargestFiles>,
    matches: Vec<usize>,
    // Result of the in-flight filter; replacing it discards any older job's result.
    filter_rx: Option<mpsc::Receiver<Vec<usize>>>,
    search: String,
    minimum_bytes: u64,
    allocated_order: bool,
    filter_due: Option<Instant>,
    show_errors: bool,
    error: Option<String>,
    volumes: Vec<mounts::Volume>,
    volumes_rx: Option<mpsc::Receiver<Vec<mounts::Volume>>>,
    /// Folders the next scan lists as empty leaves instead of entering. Session-only.
    exclusions: Vec<PathBuf>,
    exclude_input: String,
    /// What is currently on disk, so we only write when something changed.
    saved_settings: settings::Settings,
    last_folder: Option<String>,
    /// Last inner size while not maximized or fullscreen; written to disk on exit.
    window_size: Option<(u32, u32)>,
    theme: look::Theme,
    /// Mirror of egui's zoom factor in percent, so keyboard zoom (Ctrl +/-/0) is saved too.
    zoom: u32,
    reduced_motion: bool,
    /// Theme and motion last pushed to egui; they are re-applied only when they change.
    applied_look: Option<(look::Theme, bool)>,
    /// Id of the folder text field, so Ctrl+L can focus it.
    path_field: Option<egui::Id>,
}
impl Default for App {
    fn default() -> Self {
        Self {
            path: std::env::var("HOME").unwrap_or_else(|_| "/".into()),
            result: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicUsize::new(0)),
            root: 0,
            selected: None,
            hover: None,
            children: Vec::new(),
            chart_root: None,
            segments: Vec::new(),
            tiles: Vec::new(),
            listing: Vec::new(),
            view: View::default(),
            root_path: PathBuf::new(),
            largest: Arc::default(),
            matches: Vec::new(),
            filter_rx: None,
            search: String::new(),
            minimum_bytes: 0,
            allocated_order: false,
            filter_due: None,
            show_errors: false,
            error: None,
            volumes: Vec::new(),
            volumes_rx: Some(spawn_volumes()),
            exclusions: Vec::new(),
            exclude_input: String::new(),
            saved_settings: settings::Settings::default(),
            last_folder: None,
            window_size: None,
            theme: look::Theme::default(),
            zoom: 100,
            reduced_motion: false,
            applied_look: None,
            path_field: None,
        }
    }
}

/// `statvfs` can block on a dead network mount, so volumes are read off the UI thread.
fn spawn_volumes() -> mpsc::Receiver<Vec<mounts::Volume>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(mounts::volumes());
    });
    rx
}

/// Index in a list of `len` items after a keyboard step; none when the list is empty.
/// With nothing selected, Next goes to the first item and Prev to the last.
fn step_index(len: usize, current: Option<usize>, step: look::Step) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (step, current) {
        (look::Step::First, _) | (look::Step::Next, None) => 0,
        (look::Step::Last, _) | (look::Step::Prev, None) => last,
        (look::Step::Next, Some(i)) => (i + 1).min(last),
        (look::Step::Prev, Some(i)) => i.saturating_sub(1),
    })
}

impl App {
    fn poll(&mut self) {
        match self.volumes_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(volumes)) => {
                self.volumes = volumes;
                self.volumes_rx = None;
            }
            // Reader thread panicked: keep the previous list.
            Some(Err(mpsc::TryRecvError::Disconnected)) => self.volumes_rx = None,
            _ => {}
        }
        match self.receiver.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(completed)) => {
                self.result = Some(completed.scan);
                self.children = completed.children;
                self.largest = completed.largest;
                self.root_path = completed.root_path;
                self.matches.clear();
                self.filter_rx = None; // stale ids from the previous scan
                self.filter_due = Some(Instant::now());
                self.chart_root = None;
                self.root = 0;
                self.selected = None;
                self.hover = None;
                self.receiver = None;
            }
            // Sender dropped without a result: the worker panicked.
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.error = Some("Scan worker stopped unexpectedly; no results.".into());
            }
            _ => {}
        }
        match self.filter_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(matches)) => {
                self.matches = matches;
                self.filter_rx = None;
            }
            // Filter thread panicked: keep the previous matches.
            Some(Err(mpsc::TryRecvError::Disconnected)) => self.filter_rx = None,
            _ => {}
        }
    }

    /// Global keys. They are ignored while a text field has focus, so typing is never hijacked.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        if ctx.text_edit_focused() {
            return;
        }
        let (up, locate) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Backspace)
                    || i.consume_key(Modifiers::ALT, Key::ArrowUp),
                i.consume_key(Modifiers::COMMAND, Key::L),
            )
        });
        if up
            && let Some(scan) = self.result.as_deref()
            && let Some(parent) = scan.nodes.get(self.root).and_then(|n| n.parent)
        {
            self.root = parent;
        }
        if let (true, Some(id)) = (locate, self.path_field) {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    fn from_settings(saved: settings::Settings) -> Self {
        let default = Self::default();
        // Reopen the last folder only if it still exists; otherwise keep the default.
        let path = saved
            .last_folder
            .clone()
            .filter(|d| std::path::Path::new(d).is_dir())
            .unwrap_or_else(|| default.path.clone());
        Self {
            path,
            view: saved.view,
            exclusions: saved.exclusions.clone(),
            last_folder: saved.last_folder.clone(),
            theme: saved.theme,
            zoom: saved.zoom,
            reduced_motion: saved.reduced_motion,
            saved_settings: saved,
            ..default
        }
    }

    /// The window size is carried over from disk here: it changes every frame of a resize, so
    /// it is only written on exit (see `on_exit`).
    fn current_settings(&self) -> settings::Settings {
        settings::Settings {
            view: self.view,
            exclusions: self.exclusions.clone(),
            last_folder: self.last_folder.clone(),
            window: self.saved_settings.window,
            theme: self.theme,
            zoom: self.zoom,
            reduced_motion: self.reduced_motion,
        }
    }

    fn persist_settings(&mut self) {
        let now = self.current_settings();
        if now != self.saved_settings {
            if let Err(e) = settings::save(&now) {
                self.error = Some(format!("Could not save settings: {e}"));
            }
            // Also after a failure: retrying every frame would spam the error and the disk.
            self.saved_settings = now;
        }
    }

    fn add_exclusion(&mut self) {
        self.push_exclusion(PathBuf::from(self.exclude_input.trim()));
        self.exclude_input.clear();
    }

    /// Rebuilding from components drops a trailing slash, so "/a/b/" equals "/a/b".
    fn push_exclusion(&mut self, path: PathBuf) {
        let path: PathBuf = path.components().collect();
        if !path.is_absolute() {
            self.error = Some("Excluded folder must be an absolute path.".into());
        } else if !self.exclusions.contains(&path) {
            self.exclusions.push(path);
        }
    }

    fn exclusions_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(format!("Excluded folders ({})", self.exclusions.len()))
            .id_salt("exclusions")
            .show(ui, |ui| {
                ui.small("Skipped by the next scan and shown as empty folders.");
                let mut remove = None;
                for (i, path) in self.exclusions.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.label(path.to_string_lossy());
                    });
                }
                if let Some(i) = remove {
                    self.exclusions.remove(i);
                }
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.exclude_input)
                            .hint_text("/absolute/path"),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Add").clicked() || enter {
                        self.add_exclusion();
                    }
                });
            });
    }

    fn appearance_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Appearance")
            .id_salt("appearance")
            .show(ui, |ui| {
                egui::ComboBox::from_label("Theme")
                    .selected_text(self.theme.label())
                    .show_ui(ui, |ui| {
                        for theme in look::Theme::ALL {
                            ui.selectable_value(&mut self.theme, theme, theme.label());
                        }
                    });
                // A combo box rather than a slider: resizing the UI under a dragged slider jitters.
                egui::ComboBox::from_label("Size")
                    .selected_text(format!("{}%", self.zoom))
                    .show_ui(ui, |ui| {
                        for percent in [75, 100, 125, 150, 175, 200, 250] {
                            if ui
                                .selectable_label(self.zoom == percent, format!("{percent}%"))
                                .clicked()
                            {
                                ui.ctx().set_zoom_factor(percent as f32 / 100.0);
                            }
                        }
                    });
                ui.checkbox(&mut self.reduced_motion, "Reduce motion")
                    .on_hover_text("No animations or spinners.");
                ui.small("Ctrl+plus, Ctrl+minus and Ctrl+0 also change the size.");
            });
    }

    fn volumes_panel(&mut self, ui: &mut egui::Ui) {
        self.appearance_ui(ui);
        self.exclusions_ui(ui);
        ui.separator();
        let mut scan_mount = None;
        ui.horizontal(|ui| {
            ui.heading("Volumes");
            if self.volumes_rx.is_none() && ui.button("Refresh").clicked() {
                self.volumes_rx = Some(spawn_volumes());
            }
        });
        if self.volumes_rx.is_some() {
            ui.label("Reading mounts…");
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        egui::ScrollArea::vertical()
            .id_salt("volumes")
            .show(ui, |ui| {
                for v in &self.volumes {
                    let c = v.capacity;
                    ui.separator();
                    ui.label(egui::RichText::new(v.mount.mount_point.to_string_lossy()).strong());
                    ui.small(format!(
                        "{} · {}",
                        v.mount.fs_type,
                        v.mount.source.to_string_lossy()
                    ));
                    ui.add(
                        egui::ProgressBar::new((c.used() as f64 / c.total as f64) as f32).text(
                            format!("{} used of {}", look::size(c.used()), look::size(c.total)),
                        ),
                    );
                    ui.small(format!("{} available", look::size(c.available)));
                    if v.mount.fs_type == "btrfs" {
                        ui.small("Btrfs: free space is an estimate")
                            .on_hover_text("Metadata, RAID profiles and snapshots make Btrfs free space approximate.");
                    }
                    if self.receiver.is_none() && ui.small_button("Scan this volume").clicked() {
                        scan_mount = Some(v.mount.mount_point.clone());
                    }
                }
            });
        if let Some(mount_point) = scan_mount {
            // ponytail: the path box is a UTF-8 String, so a non-UTF-8 mount point is mangled here.
            self.path = mount_point.to_string_lossy().into_owned();
            self.start_scan();
        }
    }

    fn start_scan(&mut self) {
        self.error = None;
        let path = PathBuf::from(&self.path);
        // Stored absolute so a relative entry such as `.` still means the same folder next run.
        self.last_folder = std::path::absolute(&path)
            .ok()
            .and_then(|p| p.to_str().map(String::from));
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let progress = Arc::new(AtomicUsize::new(0));
        self.progress = progress.clone();
        let exclude = self.exclusions.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        thread::spawn(move || {
            let scan = Arc::new(diskatlas_scanner::scan_with_progress(
                &path, &cancel, &progress, &exclude,
            ));
            let children = chart::index(&scan);
            let largest = Arc::new(analysis::LargestFiles::new(&scan));
            let _ = tx.send(CompletedScan {
                scan,
                children,
                largest,
                root_path: path,
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, scan: Option<&Scan>) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let parent = scan.and_then(|s| s.nodes.get(self.root)?.parent);
            if ui
                .add_enabled(parent.is_some(), egui::Button::new("Up"))
                .clicked()
            {
                self.root = parent.unwrap_or(self.root);
            }
            let scanning = self.receiver.is_some();
            let width = ui.available_width() - if scanning { 80.0 } else { 70.0 };
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .desired_width(width.max(120.0))
                    .hint_text("Folder to scan"),
            );
            self.path_field = Some(edit.id);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if scanning {
                if ui.button("Cancel").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            } else if ui.button("Scan").clicked() || enter {
                self.start_scan();
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let Some(scan) = scan.filter(|s| !s.nodes.is_empty()) else {
            ui.add_space(2.0);
            return;
        };
        ui.horizontal(|ui| {
            let mut ancestors = Vec::new();
            let mut current = Some(self.root);
            while let Some(id) = current {
                ancestors.push(id);
                current = scan.nodes[id].parent;
            }
            ancestors.reverse();
            let last = ancestors.len() - 1;
            for (i, id) in ancestors.into_iter().enumerate() {
                let label = scan.nodes[id].name.to_string_lossy();
                let label = if label.is_empty() { "/".into() } else { label };
                let text = if i == last {
                    egui::RichText::new(label).strong()
                } else {
                    label.into()
                };
                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                    self.root = id;
                }
                if i != last {
                    ui.weak("›");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.view, View::LargestFiles, "Largest files");
                ui.selectable_value(&mut self.view, View::Directory, "Directory");
                ui.selectable_value(&mut self.view, View::Treemap, "Treemap");
                ui.selectable_value(&mut self.view, View::Sunburst, "Sunburst");
            });
        });
        ui.add_space(2.0);
    }

    fn status(&mut self, ui: &mut egui::Ui, scan: Option<&Scan>) {
        ui.horizontal(|ui| {
            if self.receiver.is_some() {
                if !self.reduced_motion {
                    ui.spinner();
                }
                ui.label(format!(
                    "Scanning… {} entries",
                    self.progress.load(Ordering::Relaxed)
                ));
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            } else if let Some(scan) = scan.filter(|s| !s.nodes.is_empty()) {
                let n = &scan.nodes[self.root];
                ui.label(format!(
                    "{} in this folder · {} entries scanned",
                    look::size(n.apparent),
                    scan.nodes.len()
                ));
                if scan.cancelled {
                    ui.label("Scan cancelled; results are partial.");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !scan.errors.is_empty()
                        && ui.button(format!("{} errors", scan.errors.len())).clicked()
                    {
                        self.show_errors = true;
                    }
                    if scan.other_device_skipped > 0 {
                        ui.weak(format!(
                            "{} folders on other filesystems not entered",
                            scan.other_device_skipped
                        ))
                        .on_hover_text(
                            "Mounted volumes and, on Btrfs, nested subvolumes. Scan them separately.",
                        );
                    }
                    if scan.hard_links_skipped > 0 {
                        ui.weak(format!(
                            "{} extra hard links not counted",
                            scan.hard_links_skipped
                        ));
                    }
                });
            } else {
                ui.weak("Choose a folder and select Scan.");
            }
        });
        if let Some(scan) = scan {
            let mut open = self.show_errors;
            egui::Window::new("Scan errors")
                .open(&mut open)
                .default_size([520.0, 260.0])
                .show(ui.ctx(), |ui| {
                    ui.weak("Paths may contain private information.");
                    egui::ScrollArea::vertical().show_rows(
                        ui,
                        20.0,
                        scan.errors.len(),
                        |ui, rows| {
                            for row in rows {
                                ui.label(&scan.errors[row]);
                            }
                        },
                    );
                });
            self.show_errors = open;
        }
    }

    /// Details of the selected item, or of the open folder, plus its contents beside a chart.
    fn details(&mut self, ui: &mut egui::Ui, scan: &Scan) {
        let id = self.selected.unwrap_or(self.root);
        let n = &scan.nodes[id];
        ui.add_space(6.0);
        let title = n.name.to_string_lossy();
        ui.add(
            egui::Label::new(
                egui::RichText::new(if title.is_empty() { "/".into() } else { title }).heading(),
            )
            .wrap(),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    analysis::node_path(scan, &self.root_path, id).to_string_lossy(),
                )
                .small()
                .weak(),
            )
            .wrap(),
        );
        egui::Grid::new("facts")
            .num_columns(2)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                ui.weak("Size");
                ui.label(look::size(n.apparent));
                ui.end_row();
                ui.weak("Allocated");
                ui.label(look::size(n.allocated));
                ui.end_row();
                if n.directory {
                    ui.weak("Items");
                    ui.label(self.children[id].len().to_string());
                    ui.end_row();
                }
                if id != self.root {
                    ui.weak("Of folder");
                    let whole = scan.nodes[self.root].apparent;
                    ui.label(format!(
                        "{:.1}%",
                        if whole == 0 {
                            0.0
                        } else {
                            n.apparent as f64 * 100.0 / whole as f64
                        }
                    ));
                    ui.end_row();
                }
            });
        if self.selected.is_some() && ui.button("Show parent directory").clicked() {
            self.root = n.parent.unwrap_or(id);
            self.view = View::Directory;
        }
        // Node 0 is the scan root: excluding it would leave nothing to scan.
        if n.directory && id != 0 {
            let path = analysis::node_path(scan, &self.root_path, id);
            // `absolute` handles a relative scan root such as `.` without resolving symlinks.
            let path: PathBuf = std::path::absolute(&path)
                .unwrap_or(path)
                .components()
                .collect();
            if self.exclusions.contains(&path) {
                ui.weak("Excluded from the next scan");
            } else if ui
                .button("Exclude this folder")
                .on_hover_text("Skip it in the next scan. Saved in settings.")
                .clicked()
            {
                self.push_exclusion(path);
            }
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Allocated size is an estimate and may not equal recoverable space.",
                )
                .small()
                .weak(),
            )
            .wrap(),
        );
        if scan.root_fs.as_deref() == Some("btrfs") {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Btrfs: snapshots and reflinks share data, so sizes here can add up to \
                         more than the space actually used, and deleting a file may free less \
                         than shown. Compression can make allocated smaller than size. Nested \
                         subvolumes, such as snapshots, are not entered.",
                    )
                    .small()
                    .color(ui.visuals().warn_fg_color),
                )
                .wrap(),
            );
        }
        if matches!(self.view, View::Sunburst | View::Treemap) {
            ui.separator();
            self.listing_rows(ui, scan, "side");
        }
    }

    fn listing_rows(&mut self, ui: &mut egui::Ui, scan: &Scan, salt: &str) {
        let whole = scan.nodes[self.root].apparent.max(1);
        let mut hover = None;
        let mut enter = None;
        egui::ScrollArea::vertical()
            .id_salt(salt)
            .auto_shrink(false)
            .show_rows(ui, 24.0, self.listing.len(), |ui, rows| {
                for row in rows {
                    let id = self.listing[row];
                    let n = &scan.nodes[id];
                    let swatch = look::swatch(scan, &self.segments, id);
                    let r = look::row(
                        ui,
                        &n.name.to_string_lossy(),
                        n.apparent,
                        (n.apparent as f64 / whole as f64) as f32,
                        Some(swatch),
                        self.selected == Some(id),
                    );
                    if r.clicked() {
                        self.selected = Some(id);
                    }
                    // Moving focus with the keyboard selects the row and keeps it in view.
                    if r.gained_focus() {
                        self.selected = Some(id);
                        r.scroll_to_me(None);
                    }
                    let enter_key = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (r.double_clicked() || enter_key) && n.directory {
                        enter = Some(id);
                    }
                    if r.hovered() {
                        hover = Some(id);
                    }
                }
            });
        if let Some(id) = enter {
            self.root = id;
            self.selected = None;
        }
        self.hover = hover;
    }

    fn largest_files(&mut self, ui: &mut egui::Ui, scan: &Arc<Scan>) {
        ui.horizontal(|ui| {
            let search =
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Filename contains"));
            if search.changed() {
                self.filter_due = Some(Instant::now() + Duration::from_millis(200));
            }
            if ui
                .checkbox(&mut self.allocated_order, "Rank by allocated size")
                .changed()
            {
                self.filter_due = Some(Instant::now());
            }
            ui.label("Minimum bytes");
            if ui
                .add(egui::DragValue::new(&mut self.minimum_bytes))
                .changed()
            {
                self.filter_due = Some(Instant::now() + Duration::from_millis(200));
            }
        });
        if let Some(due) = self.filter_due {
            if Instant::now() >= due {
                // ponytail: superseded jobs run to completion; add a cancel flag if many pile up.
                let (tx, rx) = mpsc::channel();
                let (scan, largest) = (scan.clone(), self.largest.clone());
                let (allocated, query, minimum) = (
                    self.allocated_order,
                    self.search.clone(),
                    self.minimum_bytes,
                );
                thread::spawn(move || {
                    let _ = tx.send(largest.filter(&scan, allocated, &query, minimum));
                });
                self.filter_rx = Some(rx);
                self.filter_due = None;
            } else {
                ui.ctx()
                    .request_repaint_after(due.saturating_duration_since(Instant::now()));
            }
        }
        if self.filter_due.is_some() || self.filter_rx.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
        let metric = |id: usize| {
            if self.allocated_order {
                scan.nodes[id].allocated
            } else {
                scan.nodes[id].apparent
            }
        };
        let top = self.matches.first().map_or(1, |&id| metric(id).max(1));
        ui.weak(format!(
            "{} files in this scan, largest first{}",
            self.matches.len(),
            if self.filter_due.is_some() || self.filter_rx.is_some() {
                " · updating…"
            } else {
                ""
            }
        ));
        egui::ScrollArea::vertical()
            .id_salt("largest")
            .auto_shrink(false)
            .show_rows(ui, 24.0, self.matches.len(), |ui, rows| {
                for row in rows {
                    let id = self.matches[row];
                    let bytes = metric(id);
                    let label = scan.nodes[id].name.to_string_lossy();
                    let r = look::row(
                        ui,
                        &label,
                        bytes,
                        (bytes as f64 / top as f64) as f32,
                        None,
                        self.selected == Some(id),
                    );
                    if r.clicked() {
                        self.selected = Some(id);
                    }
                    r.on_hover_text(
                        analysis::node_path(scan, &self.root_path, id).to_string_lossy(),
                    );
                }
            });
    }

    fn central(&mut self, ui: &mut egui::Ui, scan: Option<&Arc<Scan>>) {
        let Some(scan) = scan.filter(|s| !s.nodes.is_empty()) else {
            ui.centered_and_justified(|ui| {
                if self.receiver.is_some() {
                    if self.reduced_motion {
                        ui.label("Scanning…");
                    } else {
                        ui.spinner();
                    }
                } else {
                    ui.weak("Scan a folder to see where its space goes.");
                }
            });
            return;
        };
        let event = match self.view {
            View::Sunburst => {
                let (hover, event) = look::sunburst(
                    ui,
                    scan,
                    self.root,
                    &self.segments,
                    self.selected,
                    self.hover,
                );
                self.hover = hover;
                event
            }
            View::Treemap => {
                if self.tiles.is_empty() {
                    ui.weak("No nonzero entries in this folder; use the Directory view.");
                }
                let (hover, event) =
                    look::treemap(ui, scan, self.root, &self.tiles, self.selected, self.hover);
                self.hover = hover;
                event
            }
            View::Directory => {
                self.listing_rows(ui, scan, "directory");
                look::Event::None
            }
            View::LargestFiles => {
                self.largest_files(ui, scan);
                look::Event::None
            }
        };
        match event {
            look::Event::Enter(id) => {
                self.root = id;
                self.selected = None;
            }
            look::Event::Select(id) => self.selected = Some(id),
            look::Event::Up => self.root = scan.nodes[self.root].parent.unwrap_or(self.root),
            look::Event::Step(step) => {
                let current = self
                    .selected
                    .and_then(|id| self.listing.iter().position(|&x| x == id));
                if let Some(next) = step_index(self.listing.len(), current, step) {
                    self.selected = Some(self.listing[next]);
                }
            }
            look::Event::Open => {
                if let Some(id) = self.selected.filter(|&id| scan.nodes[id].directory) {
                    self.root = id;
                    self.selected = None;
                }
            }
            look::Event::None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        if self.applied_look != Some((self.theme, self.reduced_motion)) {
            look::set_theme(&ctx, self.theme);
            look::set_motion(&ctx, self.reduced_motion);
            self.applied_look = Some((self.theme, self.reduced_motion));
        }
        self.zoom = (ctx.zoom_factor() * 100.0).round() as u32;
        let scan = self.result.clone();
        if let Some(scan) = scan.as_deref().filter(|s| !s.nodes.is_empty())
            && self.chart_root != Some(self.root)
        {
            self.segments = chart::layout(scan, &self.children, self.root, 4);
            self.tiles = chart::treemap(scan, &self.children[self.root]);
            self.listing = self.children[self.root].clone();
            self.listing
                .sort_unstable_by_key(|&id| (std::cmp::Reverse(scan.nodes[id].apparent), id));
            self.chart_root = Some(self.root);
        }
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, scan.as_deref()));
        egui::Panel::bottom("status").show(ui, |ui| self.status(ui, scan.as_deref()));
        egui::Panel::left("volumes")
            .default_size(240.0)
            .size_range(180.0..=400.0)
            .show(ui, |ui| self.volumes_panel(ui));
        if let Some(scan) = scan.as_deref().filter(|s| !s.nodes.is_empty()) {
            egui::Panel::right("details")
                .default_size(300.0)
                .size_range(220.0..=480.0)
                .show(ui, |ui| self.details(ui, scan));
        }
        egui::CentralPanel::default().show(ui, |ui| self.central(ui, scan.as_ref()));
        self.persist_settings();
        // `viewport().inner_rect` is None on Wayland, so use the content rect: its size is the
        // window's inner size in points.
        let maximized = ui.ctx().input(|i| {
            i.viewport().maximized == Some(true) || i.viewport().fullscreen == Some(true)
        });
        if !maximized {
            // Content size is in zoomed points; the window is sized in unzoomed ones.
            let size = ui.ctx().content_rect().size() * ui.ctx().zoom_factor();
            self.window_size = Some((size.x.round() as u32, size.y.round() as u32));
        }
    }

    fn on_exit(&mut self) {
        let now = settings::Settings {
            window: self.window_size.or(self.saved_settings.window),
            ..self.current_settings()
        };
        if now != self.saved_settings {
            let _ = settings::save(&now); // exiting: nowhere left to report an error
        }
    }
}

fn main() -> eframe::Result {
    let saved = settings::load();
    let size = saved
        .window
        .map_or([1100.0, 740.0], |(w, h)| [w as f32, h as f32]);
    let viewport = egui::ViewportBuilder::default()
        .with_title("Disk Atlas")
        .with_app_id("diskatlas")
        .with_inner_size(size)
        .with_min_inner_size([640.0, 420.0]);
    eframe::run_native(
        "Disk Atlas",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            look::apply(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(saved.zoom as f32 / 100.0);
            let mut app = App::from_settings(saved);
            // `diskatlas <folder>` scans immediately, like `filelight <folder>`.
            if let Some(path) = std::env::args().nth(1) {
                app.path = path;
                app.start_scan();
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_steps_stay_in_range() {
        use look::Step::*;
        assert_eq!(step_index(0, None, Next), None, "empty list");
        assert_eq!(step_index(3, None, Next), Some(0));
        assert_eq!(step_index(3, None, Prev), Some(2));
        assert_eq!(
            step_index(3, Some(0), Prev),
            Some(0),
            "clamped at the start"
        );
        assert_eq!(step_index(3, Some(2), Next), Some(2), "clamped at the end");
        assert_eq!(step_index(3, Some(1), Next), Some(2));
        assert_eq!(step_index(3, Some(1), First), Some(0));
        assert_eq!(step_index(3, Some(1), Last), Some(2));
    }

    #[test]
    fn exclusions_are_absolute_and_deduplicated() {
        let mut app = App::default();
        for input in ["/a/b/", " /a/b ", "relative/dir", ""] {
            app.exclude_input = input.into();
            app.add_exclusion();
        }
        assert_eq!(app.exclusions, [PathBuf::from("/a/b")]);
        assert!(app.error.is_some(), "relative path must be rejected");
    }
}
