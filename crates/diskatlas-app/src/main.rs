use diskatlas_scanner::Scan;
mod analysis;
mod chart;
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

#[derive(Default, PartialEq)]
enum View {
    #[default]
    Sunburst,
    Treemap,
    Directory,
    LargestFiles,
}

struct CompletedScan {
    scan: Scan,
    children: Vec<Vec<usize>>,
    largest: analysis::LargestFiles,
    root_path: PathBuf,
}

struct App {
    path: String,
    result: Option<Scan>,
    receiver: Option<mpsc::Receiver<CompletedScan>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    root: usize,
    selected: Option<usize>,
    children: Vec<Vec<usize>>,
    chart_root: Option<usize>,
    segments: Vec<chart::Segment>,
    tiles: Vec<chart::Tile>,
    view: View,
    root_path: PathBuf,
    largest: analysis::LargestFiles,
    matches: Vec<usize>,
    search: String,
    minimum_bytes: u64,
    allocated_order: bool,
    filter_due: Option<Instant>,
    error: Option<String>,
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
            children: Vec::new(),
            chart_root: None,
            segments: Vec::new(),
            tiles: Vec::new(),
            view: View::default(),
            root_path: PathBuf::new(),
            largest: analysis::LargestFiles::default(),
            matches: Vec::new(),
            search: String::new(),
            minimum_bytes: 0,
            allocated_order: false,
            filter_due: None,
            error: None,
        }
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.receiver.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(completed)) => {
                self.result = Some(completed.scan);
                self.children = completed.children;
                self.largest = completed.largest;
                self.root_path = completed.root_path;
                self.matches.clear();
                self.filter_due = Some(Instant::now());
                self.chart_root = None;
                self.root = 0;
                self.selected = None;
                self.receiver = None;
            }
            // Sender dropped without a result: the worker panicked.
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.error = Some("Scan worker stopped unexpectedly; no results.".into());
            }
            _ => {}
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Disk Atlas");
            ui.horizontal(|ui| {
                ui.label("Directory");
                ui.text_edit_singleline(&mut self.path);
                if self.receiver.is_none() && ui.button("Scan").clicked() {
                    self.error = None;
                    let path = PathBuf::from(&self.path);
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.cancel = cancel.clone();
                    let progress = Arc::new(AtomicUsize::new(0));
                    self.progress = progress.clone();
                    let (tx, rx) = mpsc::channel();
                    self.receiver = Some(rx);
                    thread::spawn(move || {
                        let scan = diskatlas_scanner::scan_with_progress(&path, &cancel, &progress);
                        let children = chart::index(&scan);
                        let largest = analysis::LargestFiles::new(&scan);
                        let _ = tx.send(CompletedScan { scan, children, largest, root_path: path });
                    });
                }
                if self.receiver.is_some() && ui.button("Cancel").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if self.receiver.is_some() {
                ui.spinner();
                ui.label(format!(
                    "Scanning… {} entries",
                    self.progress.load(Ordering::Relaxed)
                ));
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(150));
            }
            if let Some(scan) = &self.result {
                ui.label(format!(
                    "{} entries · {} errors · {} extra hard links not counted{}",
                    scan.nodes.len(),
                    scan.errors.len(),
                    scan.hard_links_skipped,
                    if scan.cancelled { " · cancelled" } else { "" }
                ));
                ui.label("Allocated space is an estimate and may not equal recoverable space.");
                if !scan.errors.is_empty() {
                    ui.collapsing("Scan errors (paths may contain private information)", |ui| {
                        egui::ScrollArea::vertical().id_salt("errors").max_height(120.0).show_rows(ui, 20.0, scan.errors.len(), |ui, rows| {
                            for row in rows { ui.label(&scan.errors[row]); }
                        });
                    });
                }
                if !scan.nodes.is_empty() {
                    ui.horizontal(|ui| {
                        if ui.button("Up").clicked() {
                            self.root = scan.nodes[self.root].parent.unwrap_or(self.root);
                        }
                        let mut ancestors = Vec::new();
                        let mut current = Some(self.root);
                        while let Some(id) = current {
                            ancestors.push(id);
                            current = scan.nodes[id].parent;
                        }
                        ancestors.reverse();
                        for id in ancestors {
                            if ui.button(scan.nodes[id].name.to_string_lossy()).clicked() {
                                self.root = id;
                            }
                            ui.label("/");
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.view, View::Sunburst, "Sunburst");
                        ui.selectable_value(&mut self.view, View::Treemap, "Treemap");
                        ui.selectable_value(&mut self.view, View::Directory, "Directory");
                        ui.selectable_value(&mut self.view, View::LargestFiles, "Largest Files");
                    });
                    if matches!(self.view, View::Sunburst | View::Treemap) && self.chart_root != Some(self.root) {
                        self.segments = chart::layout(scan, &self.children, self.root, 3);
                        self.tiles = chart::treemap(scan, &self.children[self.root]);
                        self.chart_root = Some(self.root);
                    }
                    if self.view == View::Treemap {
                        let height = (ui.available_height() * 0.55).clamp(80.0, 360.0);
                        let (response, painter) = ui.allocate_painter(
                            egui::vec2(ui.available_width(), height),
                            egui::Sense::click(),
                        );
                        for tile in &self.tiles {
                            let rect = egui::Rect::from_min_max(
                                response.rect.min
                                    + response.rect.size()
                                        * egui::vec2(tile.min[0] as f32, tile.min[1] as f32),
                                response.rect.min
                                    + response.rect.size()
                                        * egui::vec2(tile.max[0] as f32, tile.max[1] as f32),
                            );
                            painter.rect_filled(rect, 0.0, node_color(tile.id));
                            painter.rect_stroke(
                                rect,
                                0.0,
                                egui::Stroke::new(
                                    if self.selected == Some(tile.id) {
                                        3.0
                                    } else {
                                        1.0
                                    },
                                    if self.selected == Some(tile.id) {
                                        egui::Color32::WHITE
                                    } else {
                                        ui.visuals().panel_fill
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            let n = &scan.nodes[tile.id];
                            if rect.width() > 90.0 && rect.height() > 24.0 {
                                painter.with_clip_rect(rect.shrink(3.0)).text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    n.name.to_string_lossy(),
                                    egui::FontId::proportional(14.0),
                                    egui::Color32::BLACK,
                                );
                            }
                            if response.hover_pos().is_some_and(|p| rect.contains(p)) {
                                response.clone().on_hover_text(format!(
                                    "{} · {} bytes",
                                    n.name.to_string_lossy(),
                                    n.apparent
                                ));
                                if response.clicked() {
                                    self.selected = Some(tile.id);
                                }
                                if response.double_clicked() && n.directory {
                                    self.root = tile.id;
                                }
                            }
                        }
                        if self.tiles.is_empty() {
                            ui.label("No nonzero entries; use the directory list below.");
                        }
                    }
                    if self.view == View::Sunburst {
                        let size = ui
                            .available_width()
                            .min((ui.available_height() * 0.6).clamp(80.0, 360.0));
                        let (response, painter) =
                            ui.allocate_painter(egui::vec2(size, size), egui::Sense::click());
                        let center = response.rect.center();
                        let ring = size / 8.0;
                        let pointer = response.hover_pos().map(|p| p - center);
                        for segment in &self.segments {
                            let inner = ring * (segment.depth as f32 + 1.0);
                            let outer = inner + ring;
                            let steps =
                                (((segment.end - segment.start) * 24.0).ceil() as usize).max(1);
                            let mut mesh = egui::Mesh::default();
                            let color = if self.selected == Some(segment.id) {
                                egui::Color32::WHITE
                            } else {
                                node_color(segment.id)
                            };
                            for step in 0..=steps {
                                let angle = segment.start
                                    + (segment.end - segment.start) * step as f32 / steps as f32
                                    - std::f32::consts::FRAC_PI_2;
                                let direction = egui::vec2(angle.cos(), angle.sin());
                                mesh.colored_vertex(center + direction * inner, color);
                                mesh.colored_vertex(center + direction * outer, color);
                                if step > 0 {
                                    let i = (step * 2) as u32;
                                    mesh.add_triangle(i - 2, i - 1, i);
                                    mesh.add_triangle(i - 1, i + 1, i);
                                }
                            }
                            painter.add(egui::Shape::mesh(mesh));
                            if let Some(p) = pointer {
                                let angle = (p.y.atan2(p.x) + std::f32::consts::FRAC_PI_2)
                                    .rem_euclid(std::f32::consts::TAU);
                                if p.length() >= inner
                                    && p.length() < outer
                                    && angle >= segment.start
                                    && angle < segment.end
                                {
                                    response.clone().on_hover_text(
                                        scan.nodes[segment.id].name.to_string_lossy(),
                                    );
                                    if response.clicked() {
                                        self.selected = Some(segment.id);
                                    }
                                    if response.double_clicked() && scan.nodes[segment.id].directory
                                    {
                                        self.root = segment.id;
                                    }
                                }
                            }
                        }
                        painter.text(
                            center,
                            egui::Align2::CENTER_CENTER,
                            "Up",
                            egui::FontId::proportional(14.0),
                            ui.visuals().text_color(),
                        );
                        if response.clicked() && pointer.is_some_and(|p| p.length() < ring) {
                            self.root = scan.nodes[self.root].parent.unwrap_or(self.root);
                        }
                    }
                    if let Some(id) = self.selected {
                        let n = &scan.nodes[id];
                        ui.label(format!(
                            "{} · {} bytes apparent · {} bytes allocated",
                            n.name.to_string_lossy(),
                            n.apparent,
                            n.allocated
                        ));
                        let path = analysis::node_path(scan, &self.root_path, id);
                        ui.label(path.to_string_lossy());
                        if ui.button("Show parent directory").clicked() {
                            self.root = n.parent.unwrap_or(id);
                            self.view = View::Directory;
                        }
                    }
                    if self.view == View::LargestFiles {
                        ui.label("All regular files in this scan; totals may include hard links and shared storage.");
                        ui.horizontal(|ui| {
                            ui.label("Filename contains");
                            if ui.text_edit_singleline(&mut self.search).changed() { self.filter_due = Some(Instant::now() + Duration::from_millis(200)); }
                            if ui.checkbox(&mut self.allocated_order, "Sort by allocated size").changed() { self.filter_due = Some(Instant::now()); }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Minimum bytes (selected metric)");
                            if ui.add(egui::DragValue::new(&mut self.minimum_bytes)).changed() { self.filter_due = Some(Instant::now() + Duration::from_millis(200)); }
                        });
                        if let Some(due) = self.filter_due {
                            if Instant::now() >= due {
                                self.matches = self.largest.filter(scan, self.allocated_order, &self.search, self.minimum_bytes);
                                self.filter_due = None;
                            } else { ui.ctx().request_repaint_after(due.saturating_duration_since(Instant::now())); ui.label("Updating filter…"); }
                        }
                        ui.label(format!("{} matches · largest first", self.matches.len()));
                        egui::ScrollArea::vertical().id_salt("largest").show_rows(ui, 20.0, self.matches.len(), |ui, rows| {
                            for row in rows {
                                let id = self.matches[row];
                                let node = &scan.nodes[id];
                                let response = ui.selectable_label(self.selected == Some(id), format!("{} · {} apparent · {} allocated bytes", node.name.to_string_lossy(), node.apparent, node.allocated));
                                if response.clicked() { self.selected = Some(id); }
                                response.on_hover_text(analysis::node_path(scan, &self.root_path, id).to_string_lossy());
                            }
                        });
                    } else {
                    ui.label("Children (double-click a directory to open):");
                    let children = &self.children[self.root];
                    egui::ScrollArea::vertical().show_rows(ui, 20.0, children.len(), |ui, rows| {
                        for row in rows {
                            let id = children[row];
                            let n = &scan.nodes[id];
                            let response = ui.selectable_label(
                                self.selected == Some(id),
                                format!("{} · {} bytes", n.name.to_string_lossy(), n.apparent),
                            );
                            if response.clicked() {
                                self.selected = Some(id);
                            }
                            if response.double_clicked() && n.directory {
                                self.root = id;
                            }
                        }
                    });
                    }
                }
            }
        });
    }
}
fn node_color(id: usize) -> egui::Color32 {
    egui::Color32::from_rgb(
        60 + (id.wrapping_mul(47) % 140) as u8,
        90 + (id.wrapping_mul(31) % 130) as u8,
        170,
    )
}
fn main() -> eframe::Result {
    eframe::run_native(
        "diskatlas",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::<App>::default())),
    )
}
