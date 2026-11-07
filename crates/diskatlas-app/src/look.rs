//! Visual identity: KDE Breeze Dark chrome and Filelight-style charts.
use crate::chart::{Segment, Tile};
use diskatlas_scanner::Scan;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui,
    epaint::{Hsva, TextShape},
    vec2,
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

const ACCENT: Color32 = Color32::from_rgb(0x3d, 0xae, 0xe9);
const LABEL: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x20);
/// Hole radius in rings; the centre circle is a little wider than one ring.
const HOLE: f32 = 1.15;

pub enum Event {
    None,
    Enter(usize),
    Select(usize),
    Up,
    /// Keyboard: move the selection within the open folder.
    Step(Step),
    /// Keyboard: open the selected folder.
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Next,
    Prev,
    First,
    Last,
}

/// Accessibility and keyboard handling shared by both charts. The painted chart is one
/// focusable widget: it gets an accessible name, a focus ring, and, while focused, the arrow
/// keys (which egui would otherwise use to move focus away).
fn chart_access(ui: &Ui, response: &egui::Response, label: String) -> Event {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::GraphicsDocument, true, &label));
    if !response.has_focus() {
        return Event::None;
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            response.id,
            egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
    ui.painter().rect_stroke(
        response.rect.shrink(1.0),
        3.0,
        Stroke::new(2.0, ui.visuals().selection.stroke.color),
        egui::StrokeKind::Inside,
    );
    use egui::{Key, Modifiers};
    ui.input_mut(|i| {
        let mut pressed = |key| i.consume_key(Modifiers::NONE, key);
        if pressed(Key::ArrowDown) || pressed(Key::ArrowRight) {
            Event::Step(Step::Next)
        } else if pressed(Key::ArrowUp) || pressed(Key::ArrowLeft) {
            Event::Step(Step::Prev)
        } else if pressed(Key::Home) {
            Event::Step(Step::First)
        } else if pressed(Key::End) {
            Event::Step(Step::Last)
        } else if pressed(Key::Enter) {
            Event::Open
        } else if pressed(Key::Backspace) {
            Event::Up
        } else {
            Event::None
        }
    })
}

fn chart_label(kind: &str, scan: &Scan, root: usize) -> String {
    format!(
        "{kind} chart of {}, {} in total. Arrow keys select an item, Enter opens a folder, \
         Backspace goes up.",
        name(scan, root),
        size(scan.nodes[root].apparent)
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
    HighContrast,
}

impl Theme {
    pub const ALL: [Theme; 4] = [
        Theme::System,
        Theme::Light,
        Theme::Dark,
        Theme::HighContrast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
            Theme::HighContrast => "High contrast",
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn round_corners(v: &mut egui::Visuals) {
    let w = &mut v.widgets;
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = egui::CornerRadius::same(3);
    }
}

fn breeze_dark() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = rgb(0x31, 0x36, 0x3b);
    v.window_fill = rgb(0x31, 0x36, 0x3b);
    v.extreme_bg_color = rgb(0x23, 0x26, 0x29);
    v.faint_bg_color = rgb(0x2a, 0x2e, 0x32);
    v.hyperlink_color = ACCENT;
    v.error_fg_color = rgb(0xff, 0x80, 0x80); // egui's pure red is too dark on this background
    v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let line = Stroke::new(1.0, rgb(0x4d, 0x4d, 0x4d));
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, rgb(0x3f, 0x44, 0x49));
    w.noninteractive.fg_stroke.color = rgb(0xef, 0xf0, 0xf1);
    for (state, fill) in [
        (&mut w.inactive, rgb(0x3b, 0x40, 0x45)),
        (&mut w.hovered, rgb(0x47, 0x4e, 0x55)),
        (&mut w.active, rgb(0x2a, 0x6f, 0x96)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = line;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    round_corners(&mut v);
    v
}

fn breeze_light() -> egui::Visuals {
    let accent = rgb(0x1b, 0x6f, 0xa3); // ACCENT is too pale for text on white
    let mut v = egui::Visuals::light();
    v.panel_fill = rgb(0xef, 0xf0, 0xf1);
    v.window_fill = rgb(0xef, 0xf0, 0xf1);
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = rgb(0xe6, 0xe8, 0xea);
    v.hyperlink_color = accent;
    v.warn_fg_color = rgb(0x9a, 0x4d, 0x00);
    v.error_fg_color = rgb(0xb0, 0x1c, 0x1c);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, accent);
    let line = Stroke::new(1.0, rgb(0xbc, 0xc0, 0xc4));
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, rgb(0xd5, 0xd8, 0xdb));
    for (state, fill) in [
        (&mut w.inactive, rgb(0xe3, 0xe5, 0xe7)),
        (&mut w.hovered, rgb(0xd5, 0xd9, 0xdd)),
        (&mut w.active, rgb(0xb7, 0xd8, 0xee)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = line;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, accent);
    w.active.bg_stroke = Stroke::new(1.0, accent);
    round_corners(&mut v);
    v
}

/// Black and white with heavy outlines; text contrast is above WCAG AAA (7:1).
fn high_contrast() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(Color32::WHITE);
    v.panel_fill = Color32::BLACK;
    v.window_fill = Color32::BLACK;
    v.extreme_bg_color = Color32::BLACK;
    v.faint_bg_color = rgb(0x1a, 0x1a, 0x1a);
    v.hyperlink_color = rgb(0x66, 0xd9, 0xff);
    v.warn_fg_color = rgb(0xff, 0xd0, 0x00);
    v.error_fg_color = rgb(0xff, 0x80, 0x80);
    v.selection.bg_fill = rgb(0x00, 0x55, 0xa5);
    v.selection.stroke = Stroke::new(2.0, Color32::WHITE);
    let line = Stroke::new(1.5, Color32::WHITE);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, rgb(0x99, 0x99, 0x99));
    w.noninteractive.fg_stroke = line;
    for (state, fill) in [
        (&mut w.inactive, rgb(0x14, 0x14, 0x14)),
        (&mut w.hovered, rgb(0x33, 0x33, 0x33)),
        (&mut w.active, rgb(0x00, 0x55, 0xa5)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = line;
        state.fg_stroke = line;
    }
    round_corners(&mut v);
    v
}

/// Fonts and spacing, applied once. The system Noto Sans is used when installed.
pub fn apply(ctx: &egui::Context) {
    if let Ok(bytes) = std::fs::read("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf") {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "noto".into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            list.insert(0, "noto".into());
        }
        ctx.set_fonts(fonts);
    }
    // Every theme has its own style, so spacing and text sizes go to all of them.
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 4.0);
        for (style, size) in [
            (egui::TextStyle::Body, 14.0),
            (egui::TextStyle::Button, 14.0),
            (egui::TextStyle::Heading, 18.0),
            (egui::TextStyle::Small, 12.0),
        ] {
            if let Some(f) = s.text_styles.get_mut(&style) {
                f.size = size;
            }
        }
    });
}

/// High contrast is a dark theme with different colours, so it takes the dark slot.
pub fn set_theme(ctx: &egui::Context, theme: Theme) {
    let dark = if theme == Theme::HighContrast {
        high_contrast()
    } else {
        breeze_dark()
    };
    ctx.set_visuals_of(egui::Theme::Dark, dark);
    ctx.set_visuals_of(egui::Theme::Light, breeze_light());
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark | Theme::HighContrast => egui::ThemePreference::Dark,
    });
}

/// No hover/fade transitions and no animated scrolling.
pub fn set_motion(ctx: &egui::Context, reduced: bool) {
    let default = egui::Style::default();
    ctx.all_styles_mut(|s| {
        s.animation_time = if reduced { 0.0 } else { default.animation_time };
        s.scroll_animation = if reduced {
            egui::style::ScrollAnimation::none()
        } else {
            default.scroll_animation
        };
    });
}

/// IEC sizes with one decimal, as KDE's own applications show them.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let (mut value, mut unit) = (bytes as f64 / 1024.0, 0);
    while value >= 1023.95 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[derive(Clone, Copy)]
pub enum Kind {
    Folder,
    File,
    Group,
}

/// Filelight's rainbow scheme: hue follows the segment's angle and each ring
/// darkens slightly; files are nearly grey. Returns (fill, outline).
pub fn colors(turn: f32, depth: usize, kind: Kind) -> (Color32, Color32) {
    let hsv = |s: f32, v: f32| {
        Color32::from(Hsva::new(
            turn.rem_euclid(1.0),
            s.clamp(0.0, 1.0),
            v.clamp(0.0, 1.0),
            1.0,
        ))
    };
    let s1 = 160.0 / 255.0;
    let v1 = 1.0 / (1.0 + 0.04 * depth as f32);
    let (v2, s2) = (v1 * 0.5, s1 + 0.5 * (1.0 - s1));
    match kind {
        Kind::Folder => (hsv(s1, v1), hsv(s2, v2)),
        Kind::File => (hsv(17.0 / 255.0, v1), hsv(17.0 / 255.0, v2)),
        Kind::Group => (hsv(s2, v2.max(90.0 / 255.0)), hsv(17.0 / 255.0, v1)),
    }
}

fn kind(scan: &Scan, seg: &Segment) -> Kind {
    if seg.hidden > 0 {
        Kind::Group
    } else if scan.nodes[seg.id].directory {
        Kind::Folder
    } else {
        Kind::File
    }
}

/// Fill colour for a segment found by id, so list swatches match the rings.
pub fn swatch(scan: &Scan, segments: &[Segment], id: usize) -> Color32 {
    segments
        .iter()
        .find(|s| s.id == id && s.hidden == 0)
        .map_or(Color32::GRAY, |s| {
            colors(s.start / TAU, s.depth, kind(scan, s)).0
        })
}

fn lighten(c: Color32) -> Color32 {
    c.lerp_to_gamma(Color32::WHITE, 0.35)
}

fn percent(part: u64, whole: u64) -> String {
    let p = if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    };
    if p > 0.0 && p < 0.1 {
        "<0.1%".into()
    } else {
        format!("{p:.1}%")
    }
}

fn name(scan: &Scan, id: usize) -> String {
    let n = scan.nodes[id].name.to_string_lossy();
    if n.is_empty() {
        "/".into()
    } else {
        n.into_owned()
    }
}

fn tooltip(ui: &mut Ui, scan: &Scan, id: usize, group: Option<(usize, u64)>, whole: u64) {
    match group {
        Some((count, bytes)) => {
            ui.strong(format!("{count} smaller items"));
            ui.label(format!("{} · {}", size(bytes), percent(bytes, whole)));
        }
        None => {
            let n = &scan.nodes[id];
            ui.strong(name(scan, id));
            ui.label(format!(
                "{} · {}",
                size(n.apparent),
                percent(n.apparent, whole)
            ));
            ui.weak(if n.directory {
                "Click to open this folder"
            } else {
                "File"
            });
        }
    }
}

fn text_at(painter: &egui::Painter, pos: Pos2, angle: f32, text: String, font: FontId) {
    let galley = painter.layout_no_wrap(text, font, LABEL);
    painter.add(
        TextShape::new(pos, galley, LABEL).with_angle_and_anchor(angle, Align2::CENTER_CENTER),
    );
}

fn text_width(painter: &egui::Painter, text: &str, font: &FontId) -> f32 {
    painter
        .layout_no_wrap(text.to_owned(), font.clone(), LABEL)
        .size()
        .x
}

/// Concentric-ring chart. Returns the hovered node and what the pointer did.
pub fn sunburst(
    ui: &mut Ui,
    scan: &Scan,
    root: usize,
    segs: &[Segment],
    selected: Option<usize>,
    list_hover: Option<usize>,
) -> (Option<usize>, Event) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click());
    let key_event = chart_access(ui, &response, chart_label("Sunburst", scan, root));
    let painter = ui.painter_at(rect);
    let whole = scan.nodes[root].apparent;
    let visible = segs.iter().map(|s| s.depth + 1).max().unwrap_or(1);
    let ring =
        ((rect.width().min(rect.height()) / 2.0 - 12.0) / (visible as f32 + HOLE)).clamp(8.0, 90.0);
    let hole = ring * HOLE;
    let center = rect.center();
    let inner_of = |depth: usize| hole + ring * depth as f32;
    let rel = response.hover_pos().map(|p| p - center);
    let in_hole = rel.is_some_and(|p| p.length() < hole);
    let hit = rel.filter(|p| p.length() >= hole).and_then(|p| {
        let depth = ((p.length() - hole) / ring) as usize;
        let a = (p.y.atan2(p.x) + FRAC_PI_2).rem_euclid(TAU);
        segs.iter()
            .position(|s| s.depth == depth && a >= s.start && a < s.end)
    });
    for (i, s) in segs.iter().enumerate() {
        let (inner, outer) = (inner_of(s.depth), inner_of(s.depth + 1));
        let steps = (((s.end - s.start) * outer / 5.0).ceil() as usize).clamp(1, 180);
        let (mut fill, pen) = colors(s.start / TAU, s.depth, kind(scan, s));
        if hit == Some(i) || (s.hidden == 0 && list_hover == Some(s.id)) {
            fill = lighten(fill);
        }
        let mut mesh = egui::Mesh::default();
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for step in 0..=steps {
            let a = s.start + (s.end - s.start) * step as f32 / steps as f32 - FRAC_PI_2;
            let dir = vec2(a.cos(), a.sin());
            near.push(center + dir * inner);
            far.push(center + dir * outer);
            mesh.colored_vertex(near[step], fill);
            mesh.colored_vertex(far[step], fill);
            if step > 0 {
                let k = (step * 2) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        painter.add(Shape::mesh(mesh));
        let outline: Vec<Pos2> = far
            .iter()
            .copied()
            .chain(near.iter().rev().copied())
            .collect();
        painter.add(Shape::closed_line(outline.clone(), Stroke::new(1.0, pen)));
        if s.hidden == 0 && selected == Some(s.id) {
            painter.add(Shape::closed_line(
                outline,
                Stroke::new(2.5, Color32::WHITE),
            ));
        }
    }
    for s in segs.iter().filter(|s| s.hidden == 0) {
        let mid = (s.start + s.end) / 2.0 - FRAC_PI_2;
        let (inner, outer) = (inner_of(s.depth), inner_of(s.depth + 1));
        let radius = (inner + outer) / 2.0;
        let arc = radius * (s.end - s.start);
        if arc < 13.0 {
            continue;
        }
        let font = FontId::proportional(if s.depth == 0 { 14.0 } else { 12.5 });
        let label = name(scan, s.id);
        let width = text_width(&painter, &label, &font);
        let pos = center + vec2(mid.cos(), mid.sin()) * radius;
        // A straight line along a curve sags by width²/8r; keep that inside the ring.
        if arc >= width + 8.0 && width * width / (8.0 * radius) <= ring * 0.3 {
            // Along the ring, turned so the text never reads upside down.
            let flip = if mid.sin() > 0.0 { PI } else { 0.0 };
            text_at(&painter, pos, mid + FRAC_PI_2 - flip, label, font);
        } else if ring >= width + 8.0 {
            let flip = if mid.cos() < 0.0 { PI } else { 0.0 };
            text_at(&painter, pos, mid + flip, label, font);
        }
    }
    // Centre: the hovered item, else the open folder; click it to go up.
    let can_up = scan.nodes[root].parent.is_some();
    let hole_fill = if in_hole && can_up {
        ui.visuals().widgets.inactive.bg_fill
    } else {
        ui.visuals().panel_fill
    };
    painter.circle(
        center,
        hole - 2.0,
        hole_fill,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
    let (title, detail) = match hit.map(|i| &segs[i]) {
        Some(s) if s.hidden > 0 => (format!("{} smaller items", s.hidden), size(s.size)),
        Some(s) => (
            name(scan, s.id),
            format!("{} · {}", size(s.size), percent(s.size, whole)),
        ),
        None => (name(scan, root), size(whole)),
    };
    let fg = ui.visuals().text_color();
    let wrap = hole * 1.55;
    let head = painter.layout(title, FontId::proportional(14.0), fg, wrap);
    let sub = painter.layout(
        detail,
        FontId::proportional(12.5),
        fg.gamma_multiply(0.7),
        wrap,
    );
    let total = head.size().y + sub.size().y + 2.0;
    let top = center.y - total / 2.0;
    painter.galley(
        Pos2::new(center.x - head.size().x / 2.0, top),
        head.clone(),
        fg,
    );
    painter.galley(
        Pos2::new(center.x - sub.size().x / 2.0, top + head.size().y + 2.0),
        sub,
        fg,
    );
    if in_hole && can_up {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let hovered = hit.map(|i| &segs[i]);
    if hovered.is_some_and(|s| s.hidden == 0 && scan.nodes[s.id].directory) {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let group = hovered.filter(|s| s.hidden > 0).map(|s| (s.hidden, s.size));
    let id = hovered.map(|s| s.id);
    let response = if let Some(s) = hovered {
        response.on_hover_ui_at_pointer(|ui| tooltip(ui, scan, s.id, group, whole))
    } else {
        response
    };
    let event = if !response.clicked() {
        Event::None
    } else if in_hole {
        if can_up { Event::Up } else { Event::None }
    } else {
        match hovered {
            Some(s) if s.hidden == 0 && scan.nodes[s.id].directory => Event::Enter(s.id),
            Some(s) if s.hidden == 0 => Event::Select(s.id),
            _ => Event::None,
        }
    };
    // A pointer action wins over a key pressed in the same frame.
    let event = if matches!(event, Event::None) {
        key_event
    } else {
        event
    };
    (id.filter(|_| group.is_none()), event)
}

/// Area chart of the open folder's immediate children.
pub fn treemap(
    ui: &mut Ui,
    scan: &Scan,
    root: usize,
    tiles: &[Tile],
    selected: Option<usize>,
    list_hover: Option<usize>,
) -> (Option<usize>, Event) {
    let rect = ui.available_rect_before_wrap().shrink(4.0);
    let response = ui.allocate_rect(rect, Sense::click());
    let key_event = chart_access(ui, &response, chart_label("Treemap", scan, root));
    let painter = ui.painter_at(rect);
    let whole = scan.nodes[root].apparent;
    let place = |t: &Tile| {
        Rect::from_min_max(
            rect.min + rect.size() * vec2(t.min[0] as f32, t.min[1] as f32),
            rect.min + rect.size() * vec2(t.max[0] as f32, t.max[1] as f32),
        )
    };
    let pointer = response.hover_pos();
    let hit = pointer.and_then(|p| tiles.iter().position(|t| place(t).contains(p)));
    for (i, t) in tiles.iter().enumerate() {
        let n = &scan.nodes[t.id];
        let rect = place(t).shrink(1.0);
        let k = if n.directory {
            Kind::Folder
        } else {
            Kind::File
        };
        let (mut fill, pen) = colors(i as f32 / tiles.len() as f32, 0, k);
        if hit == Some(i) || list_hover == Some(t.id) {
            fill = lighten(fill);
        }
        painter.rect_filled(rect, 2.0, fill);
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, pen), egui::StrokeKind::Inside);
        if selected == Some(t.id) {
            painter.rect_stroke(
                rect,
                2.0,
                Stroke::new(2.5, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
        if rect.width() > 70.0 && rect.height() > 22.0 {
            let clip = painter.with_clip_rect(rect.shrink(4.0));
            clip.text(
                rect.left_top() + vec2(7.0, 5.0),
                Align2::LEFT_TOP,
                name(scan, t.id),
                FontId::proportional(14.0),
                LABEL,
            );
            if rect.height() > 42.0 {
                clip.text(
                    rect.left_top() + vec2(7.0, 23.0),
                    Align2::LEFT_TOP,
                    size(n.apparent),
                    FontId::proportional(12.5),
                    LABEL.gamma_multiply(0.8),
                );
            }
        }
    }
    let hovered = hit.map(|i| tiles[i].id);
    if hovered.is_some_and(|id| scan.nodes[id].directory) {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let response = match hovered {
        Some(id) => response.on_hover_ui_at_pointer(|ui| tooltip(ui, scan, id, None, whole)),
        None => response,
    };
    let event = match hovered {
        Some(id) if response.clicked() => {
            if scan.nodes[id].directory {
                Event::Enter(id)
            } else {
                Event::Select(id)
            }
        }
        _ => key_event,
    };
    (hovered, event)
}

/// One list row: swatch, name, size, and a faint bar showing the share of `fraction`.
pub fn row(
    ui: &mut Ui,
    label: &str,
    bytes: u64,
    fraction: f32,
    swatch: Option<Color32>,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::ListBoxOption,
            true,
            selected,
            format!(
                "{label}, {}, {:.1}% of folder",
                size(bytes),
                fraction.clamp(0.0, 1.0) * 100.0
            ),
        )
    });
    let p = ui.painter_at(rect);
    let v = ui.visuals();
    if selected {
        p.rect_filled(rect, 3.0, v.selection.bg_fill);
    } else if response.hovered() {
        p.rect_filled(rect, 3.0, v.widgets.hovered.weak_bg_fill);
    }
    let bar = Rect::from_min_size(
        rect.left_bottom() - vec2(0.0, 3.0),
        vec2(rect.width() * fraction.clamp(0.0, 1.0), 2.0),
    );
    p.rect_filled(bar, 1.0, swatch.unwrap_or(ACCENT).gamma_multiply(0.8));
    if response.has_focus() {
        p.rect_stroke(
            rect.shrink(1.0),
            3.0,
            Stroke::new(2.0, v.selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
    let mut x = rect.left() + 8.0;
    if let Some(c) = swatch {
        p.rect_filled(
            Rect::from_center_size(Pos2::new(x + 5.0, rect.center().y), vec2(10.0, 10.0)),
            2.0,
            c,
        );
        x += 18.0;
    }
    let fg = v.text_color();
    let value = p.layout_no_wrap(
        size(bytes),
        FontId::proportional(13.0),
        fg.gamma_multiply(0.75),
    );
    let right = rect.right() - 8.0 - value.size().x;
    p.galley(
        Pos2::new(right, rect.center().y - value.size().y / 2.0),
        value,
        fg,
    );
    p.with_clip_rect(Rect::from_min_max(
        rect.min,
        Pos2::new(right - 8.0, rect.max.y),
    ))
    .text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        fg,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.x contrast ratio between two opaque colours.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let lin = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
        };
        let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn theme_text_colours_meet_wcag() {
        // (visuals, minimum ratio): AA for normal text, AAA for high contrast.
        for (name, v, min) in [
            ("dark", breeze_dark(), 4.5),
            ("light", breeze_light(), 4.5),
            ("high contrast", high_contrast(), 7.0),
        ] {
            for (what, fg) in [
                ("text", v.text_color()),
                ("hyperlink", v.hyperlink_color),
                ("warning", v.warn_fg_color),
                ("error", v.error_fg_color),
            ] {
                let ratio = contrast(fg, v.panel_fill);
                assert!(ratio >= min, "{name} {what}: {ratio:.2} < {min}");
            }
        }
    }

    #[test]
    fn high_contrast_takes_the_dark_slot() {
        let ctx = egui::Context::default();
        set_theme(&ctx, Theme::HighContrast);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            Color32::BLACK
        );
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::Dark
        );
        set_theme(&ctx, Theme::Dark);
        assert_ne!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            Color32::BLACK
        );
        set_theme(&ctx, Theme::System);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
    }

    #[test]
    fn reduced_motion_disables_animation_and_can_be_undone() {
        let ctx = egui::Context::default();
        set_motion(&ctx, true);
        assert_eq!(ctx.style_of(egui::Theme::Light).animation_time, 0.0);
        set_motion(&ctx, false);
        assert!(ctx.style_of(egui::Theme::Light).animation_time > 0.0);
    }

    // --- accessibility and keyboard ---

    use diskatlas_scanner::Node;

    /// root(30) -> a.bin(20), b.bin(10)
    fn tiny_scan() -> Scan {
        let node = |name: &str, parent, directory, size| Node {
            name: name.into(),
            parent,
            directory,
            regular_file: !directory,
            apparent: size,
            allocated: size,
        };
        Scan {
            nodes: vec![
                node("root", None, true, 30),
                node("a.bin", Some(0), false, 20),
                node("b.bin", Some(0), false, 10),
            ],
            ..Scan::default()
        }
    }

    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        run: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0));
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, run);
        out.textures_delta.clear(); // nothing renders these; egui panics if they are dropped unapplied
        out
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn screen_readers_get_named_chart_and_rows() {
        let scan = tiny_scan();
        let children = crate::chart::index(&scan);
        let segs = crate::chart::layout(&scan, &children, 0, 2);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let out = frame(&ctx, vec![], |ui| {
            sunburst(ui, &scan, 0, &segs, None, None);
        });
        let update = out
            .platform_output
            .accesskit_update
            .expect("accesskit tree");
        let chart = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == egui::Role::GraphicsDocument)
            .expect("chart node");
        let label = chart.1.label().unwrap();
        assert!(
            label.starts_with("Sunburst chart of root, 30 B in total"),
            "{label}"
        );
        assert!(label.contains("Arrow keys"), "{label}");

        let out = frame(&ctx, vec![], |ui| {
            row(ui, "a.bin", 2048, 0.5, None, true);
        });
        let update = out.platform_output.accesskit_update.unwrap();
        let row_node = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == egui::Role::ListBoxOption)
            .expect("row node");
        assert_eq!(row_node.1.label(), Some("a.bin, 2.0 KiB, 50.0% of folder"));
    }

    #[test]
    fn focused_chart_turns_keys_into_navigation_events() {
        let scan = tiny_scan();
        let children = crate::chart::index(&scan);
        let segs = crate::chart::layout(&scan, &children, 0, 2);
        let ctx = egui::Context::default();
        let run = |ctx: &egui::Context, events: Vec<egui::Event>, focus: bool| {
            let mut got = Event::None;
            frame(ctx, events, |ui| {
                if focus {
                    let id = ui.next_auto_id();
                    ui.memory_mut(|m| m.request_focus(id));
                }
                got = sunburst(ui, &scan, 0, &segs, None, None).1;
                // Another focusable widget, so that unlocked arrow keys would move focus to it.
                ui.add(egui::Button::new("neighbour"));
            });
            got
        };
        // Without focus, keys do nothing.
        assert!(matches!(
            run(&ctx, vec![key(egui::Key::ArrowDown)], false),
            Event::None
        ));
        run(&ctx, vec![], true); // give the chart focus
        run(&ctx, vec![], false); // frame in which it installs its arrow-key lock
        for (k, expect) in [
            (egui::Key::ArrowDown, Some(Step::Next)),
            (egui::Key::ArrowRight, Some(Step::Next)),
            (egui::Key::ArrowUp, Some(Step::Prev)),
            (egui::Key::Home, Some(Step::First)),
            (egui::Key::End, Some(Step::Last)),
        ] {
            match (run(&ctx, vec![key(k)], false), expect) {
                (Event::Step(got), Some(want)) => assert_eq!(got, want, "{k:?}"),
                (other, _) => panic!("{k:?} gave {}", name_of(&other)),
            }
        }
        assert!(matches!(
            run(&ctx, vec![key(egui::Key::Enter)], false),
            Event::Open
        ));
        assert!(matches!(
            run(&ctx, vec![key(egui::Key::Backspace)], false),
            Event::Up
        ));
    }

    fn name_of(e: &Event) -> &'static str {
        match e {
            Event::None => "None",
            Event::Enter(_) => "Enter",
            Event::Select(_) => "Select",
            Event::Up => "Up",
            Event::Step(_) => "Step",
            Event::Open => "Open",
        }
    }
}
