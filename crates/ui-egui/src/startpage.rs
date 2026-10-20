//! The Start tab: Get Started (Start Drawing with a template, Open Files, the sample drawing)
//! and Recent Documents, each with a thumbnail of the drawing as it was last seen, when it was
//! last opened and a pin that keeps it at the top.
//!
//! Thumbnails are the drawing's own lines (from the display list), simplified and stored with the
//! preferences, so no image files are written.

use std::collections::BTreeMap;
use std::sync::Arc;

use cadcraft_render::{DisplayList, Kind};
use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::CadApp;
use crate::theme::Tokens;

/// Most points kept in one thumbnail.
const MAX_THUMB_POINTS: usize = 6000;
/// Most recent documents remembered.
pub const MAX_RECENT_META: usize = 40;

/// What the Start tab remembers of a recent document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentMeta {
    /// When it was last opened (ms since 1970, UTC).
    #[serde(default)]
    pub opened: f64,
    #[serde(default)]
    pub pinned: bool,
    /// The thumbnail ([`encode`]): base64 of (r, g, b, point count u16, points u16 x/y)…
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thumb: String,
}

/// A decoded thumbnail: polylines in 0..1 (y up), with their colours.
pub type Thumb = Vec<(Color32, Vec<[f32; 2]>)>;

/// The drawing of `list` as a thumbnail: its polylines fitted into a square, small details left
/// out when there are too many points.
pub fn encode(list: &DisplayList) -> String {
    let b = list.bounds;
    if b.is_empty() || !b.width().is_finite() || !b.height().is_finite() {
        return String::new();
    }
    let size = b.width().max(b.height()).max(1e-9);
    let (cx, cy) = ((b.min.x + b.max.x) / 2.0, (b.min.y + b.max.y) / 2.0);
    let total: usize = list.prims.iter().filter(|p| p.kind == Kind::Polyline).map(|p| p.len as usize).sum();
    // Leave out the smallest polylines until the rest fit.
    let min_extent = if total > MAX_THUMB_POINTS { size * 0.004 } else { 0.0 };
    let q = |v: f64, c: f64| (((v - c) / size + 0.5).clamp(0.0, 1.0) * 65535.0).round() as u16;
    let mut out: Vec<u8> = Vec::new();
    let mut kept = 0usize;
    for p in list.prims.iter().filter(|p| p.kind == Kind::Polyline) {
        let pts = list.points(p);
        if pts.len() < 2 {
            continue;
        }
        if min_extent > 0.0 {
            let pb = cadcraft_geom::Bounds2::from_points(pts.iter().copied());
            if pb.width().max(pb.height()) < min_extent {
                continue;
            }
        }
        let step = if total > MAX_THUMB_POINTS { (pts.len() / 64).max(1) } else { 1 };
        let mut sel: Vec<cadcraft_geom::Vec2> = pts.iter().step_by(step).copied().collect();
        if let Some(l) = pts.last()
            && sel.last() != Some(l)
        {
            sel.push(*l);
        }
        let n = sel.len().min(u16::MAX as usize);
        if kept + n > MAX_THUMB_POINTS * 2 {
            break;
        }
        kept += n;
        let cadcraft_color::Rgb(r, g, bb) = p.color;
        out.extend([r, g, bb]);
        out.extend((n as u16).to_le_bytes());
        for v in sel.iter().take(n) {
            out.extend(q(v.x, cx).to_le_bytes());
            out.extend(q(v.y, cy).to_le_bytes());
        }
    }
    cadcraft_engine::cmd::file::base64_encode(&out)
}

/// The thumbnail [`encode`] made.
pub fn decode(s: &str) -> Thumb {
    let Some(bytes) = cadcraft_engine::cmd::file::base64_decode(s) else { return Vec::new() };
    let mut out = Vec::new();
    let mut i = 0usize;
    let u16_at = |k: usize| bytes.get(k..k + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    while let (Some(r), Some(g), Some(b), Some(n)) = (bytes.get(i), bytes.get(i + 1), bytes.get(i + 2), u16_at(i + 3)) {
        i += 5;
        let mut pts = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            let (Some(x), Some(y)) = (u16_at(i), u16_at(i + 2)) else { return out };
            pts.push([f32::from(x) / 65535.0, f32::from(y) / 65535.0]);
            i += 4;
        }
        out.push((Color32::from_rgb(*r, *g, *b), pts));
        if out.len() > 100_000 {
            break;
        }
    }
    out
}

/// Days since 1970-01-01 → (year, month 1..12, day).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];

/// Local day number (days since 1970) and seconds into that day of `ms`.
fn local(ms: f64, offset_min: i32) -> (i64, i64) {
    let secs = (ms / 1000.0).floor() as i64 + i64::from(offset_min) * 60;
    (secs.div_euclid(86_400), secs.rem_euclid(86_400))
}

/// "Tuesday, October 20, 2026 10:14:08 AM" in local time.
pub fn format_time(ms: f64, offset_min: i32) -> String {
    if !ms.is_finite() || ms <= 0.0 {
        return String::new();
    }
    let (day, s) = local(ms, offset_min);
    let (y, m, d) = civil(day);
    let wd = WEEKDAYS.get(day.rem_euclid(7) as usize).copied().unwrap_or("");
    let (h, mi, se) = (s / 3600, (s / 60) % 60, s % 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    let month = MONTHS.get((m as usize).saturating_sub(1)).copied().unwrap_or("");
    format!("{wd}, {month} {d}, {y} {h12}:{mi:02}:{se:02} {ampm}")
}

/// The group a recent document is listed under.
fn group_of(ms: f64, now: f64, offset_min: i32) -> &'static str {
    let (d, _) = local(ms, offset_min);
    let (today, _) = local(now, offset_min);
    match today - d {
        i64::MIN..=0 => "TODAY",
        1 => "YESTERDAY",
        2..=6 => "THIS WEEK",
        _ => "OLDER",
    }
}

/// The Start page.
pub fn show(app: &mut CadApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let r = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, t.chrome);
    let mut action: Option<(&str, serde_json::Value)> = None;
    // Two columns, centred, as wide as fits.
    let total_w = (r.width() - 64.0).clamp(320.0, 900.0);
    let narrow = r.width() < 720.0;
    let left_w = if narrow { total_w } else { 280.0 };
    let x0 = r.center().x - total_w / 2.0;
    let top = r.top() + 36.0;
    let heading = egui::FontId::proportional(19.0);
    // Get Started
    p.text(pos2(x0, top), egui::Align2::LEFT_TOP, crate::tl!("Get Started"), heading.clone(), t.text_dim);
    let card = Rect::from_min_size(pos2(x0, top + 30.0), vec2(left_w.min(280.0), 172.0));
    let resp = ui.interact(card, ui.id().with("start_drawing"), Sense::click());
    draw_start_card(&p, card, resp.hovered());
    if resp.on_hover_text(crate::tl!("Start a new drawing with the current template")).clicked() {
        action = Some(("new", json!({})));
    }
    // Templates
    let tpl = Rect::from_min_size(pos2(card.left(), card.bottom()), vec2(card.width(), 26.0));
    p.rect_filled(tpl, 0.0, Color32::from_gray(222));
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tpl.shrink2(vec2(6.0, 2.0))));
    egui::ComboBox::from_id_salt("start_templates").width(tpl.width() - 14.0).selected_text(crate::tl!("Templates")).show_ui(&mut tui, |ui| {
        if ui.button(crate::tl!("ALSYS default (mm)")).clicked() {
            action = Some(("new", json!({ "template": cadcraft_engine::cmd::file::template::BUILTIN })));
        }
        if ui.button(crate::tl!("Metric, no template (acadiso)")).clicked() {
            action = Some(("new", json!({ "template": ".", "metric": true })));
        }
        if ui.button(crate::tl!("Imperial, no template (acad)")).clicked() {
            action = Some(("new", json!({ "template": ".", "metric": false })));
        }
        if ui.button(crate::tl!("Browse templates...")).clicked() {
            action = Some(("ui.open", json!({})));
        }
    });
    let mut y = tpl.bottom() + 18.0;
    for (label, glyph, cmd) in [
        (crate::tl!("Open Files..."), "folder", "ui.open"),
        (crate::tl!("Open Sample Drawing..."), "sample", "ui.sample"),
        (crate::tl!("New Drawing (metric)"), "new", "new.metric"),
    ] {
        let row = Rect::from_min_size(pos2(card.left(), y), vec2(card.width(), 30.0));
        let resp = ui.interact(row, ui.id().with(("start_link", cmd)), Sense::click());
        draw_link_icon(&p, Rect::from_min_size(row.min + vec2(6.0, 5.0), vec2(26.0, 20.0)), glyph, t.text_dim);
        let col = if resp.hovered() { t.text } else { t.text_dim };
        p.text(pos2(row.left() + 44.0, row.center().y), egui::Align2::LEFT_CENTER, label, crate::theme::body(), col);
        if resp.clicked() {
            action = Some(match cmd {
                "new.metric" => ("new", json!({ "metric": true })),
                c => (c, json!({})),
            });
        }
        y += 36.0;
    }
    p.text(
        pos2(card.left(), y + 10.0),
        egui::Align2::LEFT_TOP,
        crate::tf!("CADCraft {version} · Kitt Edition", version = env!("CARGO_PKG_VERSION")),
        crate::theme::small(),
        t.text_faint,
    );
    // Recent Documents
    let (rx, ry) = if narrow { (x0, y + 50.0) } else { (x0 + left_w + 60.0, top) };
    let rw = if narrow { total_w } else { (total_w - left_w - 60.0).max(240.0) };
    p.text(pos2(rx, ry), egui::Align2::LEFT_TOP, crate::tl!("Recent Documents"), heading, t.text_dim);
    let list_rect = Rect::from_min_max(pos2(rx, ry + 30.0), pos2(rx + rw, r.bottom() - 40.0));
    recent_list(app, ui, list_rect, &mut action);
    // View: large, small images, list.
    let vy = r.bottom() - 26.0;
    for (k, tip) in [crate::tl!("Large images"), crate::tl!("Small images"), crate::tl!("List")].into_iter().enumerate() {
        let b = Rect::from_center_size(pos2(rx + rw / 2.0 + (k as f32 - 1.0) * 30.0, vy), vec2(22.0, 18.0));
        let resp = ui.interact(b, ui.id().with(("recent_view", k)), Sense::click());
        let on = app.ui.recent_view == k as u8;
        if on || resp.hovered() {
            p.rect_filled(b, 3.0, t.control_hover);
        }
        let c = if on { t.text } else { t.text_faint };
        let rows = [3, 2, 4][k];
        for i in 0..rows {
            let yy = b.top() + 4.0 + i as f32 * (10.0 / rows as f32 + 1.0);
            if k < 2 {
                p.rect_filled(Rect::from_min_size(pos2(b.left() + 5.0, yy), vec2(5.0, 10.0 / rows as f32)), 0.0, c);
                p.line_segment([pos2(b.left() + 12.0, yy + 1.0), pos2(b.right() - 5.0, yy + 1.0)], Stroke::new(1.0, c));
            } else {
                p.line_segment([pos2(b.left() + 5.0, yy + 1.0), pos2(b.right() - 5.0, yy + 1.0)], Stroke::new(1.0, c));
            }
        }
        if resp.on_hover_text(tip).clicked() {
            app.ui.recent_view = k as u8;
        }
    }
    if let Some((cmd, params)) = action {
        match cmd {
            "new" => {
                let _ = app.run("new", params);
                app.ui.start_tab = false;
            }
            "open.recent" => {
                if let Some(path) = params.as_str() {
                    open_recent(app, path);
                }
            }
            c => app.start(c),
        }
    }
}

fn open_recent(app: &mut CadApp, path: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        // The browser can't reopen a file by name: pick it again.
        let _ = path;
        app.start("ui.open");
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.open_path(path);
    }
}

/// The recent documents, pinned first, in groups by when they were opened.
fn recent_list(app: &mut CadApp, ui: &mut egui::Ui, rect: Rect, action: &mut Option<(&str, serde_json::Value)>) {
    let t = Tokens::get();
    let now = crate::now_ms();
    let offset = app.ui.utc_offset_min;
    let mut items: Vec<(String, RecentMeta)> =
        app.ui.recent_files.iter().map(|p| (p.clone(), app.ui.recent_meta.get(p).cloned().unwrap_or_default())).collect();
    items.sort_by(|a, b| b.1.pinned.cmp(&a.1.pinned).then(b.1.opened.total_cmp(&a.1.opened)));
    if items.is_empty() {
        ui.painter().text(
            rect.center_top() + vec2(0.0, 60.0),
            egui::Align2::CENTER_CENTER,
            crate::tl!("There is nothing here yet."),
            egui::FontId::proportional(16.0),
            t.text_dim,
        );
        return;
    }
    let view = app.ui.recent_view;
    let (row_h, thumb_w) = match view {
        0 => (100.0, 140.0),
        1 => (58.0, 76.0),
        _ => (34.0, 0.0),
    };
    let mut toggle_pin: Option<String> = None;
    let mut open: Option<String> = None;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    egui::ScrollArea::vertical().id_salt("recent_docs").max_height(rect.height()).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.set_width(rect.width());
        let mut last_group = "";
        for (path, meta) in &items {
            let group = if meta.pinned { "PINNED" } else { group_of(meta.opened.max(1.0), now, offset) };
            if (meta.opened > 0.0 || meta.pinned) && group != last_group {
                {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(crate::i18n::t(group)).small().strong().color(t.text_faint));
                    ui.separator();
                    last_group = group;
                }
            }
            let (row, resp) = ui.allocate_exact_size(vec2(rect.width() - 12.0, row_h), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(row, 3.0, t.control_hover.gamma_multiply(0.5));
            }
            let mut tx = row.left() + 8.0;
            if thumb_w > 0.0 {
                let tr = Rect::from_min_size(pos2(row.left() + 4.0, row.top() + 4.0), vec2(thumb_w, row_h - 8.0));
                draw_thumb(app, p, tr, path, &meta.thumb);
                tx = tr.right() + 12.0;
            }
            let name = crate::menus::file_name(path);
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(a, _)| a).to_string();
            let clip = p.with_clip_rect(Rect::from_min_max(pos2(tx, row.top()), pos2(row.right() - 26.0, row.bottom())));
            let when = format_time(meta.opened, offset);
            if view == 2 {
                clip.text(pos2(tx, row.center().y), egui::Align2::LEFT_CENTER, &stem, crate::theme::body(), t.text);
                clip.text(pos2(row.right() - 230.0, row.center().y), egui::Align2::LEFT_CENTER, &when, crate::theme::small(), t.text_faint);
            } else {
                clip.text(pos2(tx, row.top() + 10.0), egui::Align2::LEFT_TOP, &stem, egui::FontId::proportional(15.0), t.text);
                if !when.is_empty() {
                    clip.text(
                        pos2(tx, row.top() + 30.0),
                        egui::Align2::LEFT_TOP,
                        crate::tf!("Last Opened {when}", when = when),
                        crate::theme::small(),
                        t.text_dim,
                    );
                }
            }
            // Pin
            let pin = Rect::from_center_size(pos2(row.right() - 14.0, row.top() + 14.0), vec2(18.0, 18.0));
            let pr = ui.interact(pin, ui.id().with(("pin", path)), Sense::click());
            draw_pin(p, pin, meta.pinned, if pr.hovered() || meta.pinned { t.text } else { t.text_faint });
            if pr.on_hover_text(crate::tl!("Pin to the list")).clicked() {
                toggle_pin = Some(path.clone());
            } else if resp.on_hover_text(path.as_str()).clicked() {
                open = Some(path.clone());
            }
        }
    });
    if let Some(path) = toggle_pin {
        let m = app.ui.recent_meta.entry(path).or_default();
        m.pinned = !m.pinned;
    }
    if let Some(path) = open {
        *action = Some(("open.recent", json!(path)));
    }
}

/// A recent document's thumbnail (decoded once, then cached).
fn draw_thumb(app: &mut CadApp, p: &egui::Painter, r: Rect, path: &str, data: &str) {
    p.rect_filled(r, 2.0, Color32::from_rgb(33, 40, 48));
    p.rect_stroke(r, 2.0, Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Inside);
    if data.is_empty() {
        // A blank sheet with a drawing frame.
        let s = r.shrink(r.height() * 0.22);
        p.rect_stroke(s, 0.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
        return;
    }
    let key = (path.to_string(), data.len());
    let th = match app.ui.thumb_cache.get(&key.0) {
        Some((n, th)) if *n == key.1 => th.clone(),
        _ => {
            let th = Arc::new(decode(data));
            if app.ui.thumb_cache.len() > 64 {
                app.ui.thumb_cache.clear();
            }
            app.ui.thumb_cache.insert(key.0.clone(), (key.1, th.clone()));
            th
        }
    };
    let inner = r.shrink(6.0);
    let side = inner.width().min(inner.height());
    let sq = Rect::from_center_size(inner.center(), vec2(inner.width().max(side), side));
    let to = |q: &[f32; 2]| -> Pos2 { pos2(sq.center().x + (q[0] - 0.5) * sq.height(), sq.center().y - (q[1] - 0.5) * sq.height()) };
    let clip = p.with_clip_rect(r.shrink(1.0));
    for (c, pts) in th.iter() {
        // Colour 7 shows white on the dark thumbnail.
        let c = if *c == Color32::BLACK { Color32::WHITE } else { *c };
        let line: Vec<Pos2> = pts.iter().map(to).collect();
        clip.add(egui::Shape::line(line, Stroke::new(0.8, c)));
    }
}

/// The Start Drawing card: an original sketch of buildings and a floor plan.
fn draw_start_card(p: &egui::Painter, r: Rect, hover: bool) {
    let bg_top = Color32::from_gray(if hover { 255 } else { 246 });
    let bg_bottom = Color32::from_gray(if hover { 214 } else { 200 });
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), bg_top);
    mesh.colored_vertex(r.right_top(), bg_top);
    mesh.colored_vertex(r.right_bottom(), bg_bottom);
    mesh.colored_vertex(r.left_bottom(), bg_bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
    let ink = Stroke::new(1.0, Color32::from_gray(120));
    let faint = Stroke::new(0.8, Color32::from_gray(165));
    let o = r.min;
    let at = |x: f32, y: f32| pos2(o.x + x * r.width() / 280.0, o.y + y * r.height() / 172.0);
    // Floor plan (top right).
    for (a, b) in [((150.0, 22.0), (262.0, 22.0)), ((262.0, 22.0), (262.0, 92.0)), ((262.0, 92.0), (150.0, 92.0)), ((150.0, 92.0), (150.0, 22.0))] {
        p.line_segment([at(a.0, a.1), at(b.0, b.1)], faint);
    }
    for (a, b) in [((196.0, 22.0), (196.0, 60.0)), ((150.0, 60.0), (230.0, 60.0)), ((230.0, 60.0), (230.0, 92.0)), ((214.0, 22.0), (214.0, 44.0))] {
        p.line_segment([at(a.0, a.1), at(b.0, b.1)], faint);
    }
    p.line_segment([at(150.0, 14.0), at(262.0, 14.0)], faint);
    p.line_segment([at(270.0, 22.0), at(270.0, 92.0)], faint);
    // Three blocks in parallel projection (left).
    let block = |x: f32, base: f32, w: f32, h: f32, d: f32| {
        let (dx, dy) = (d * 0.8, -d * 0.45);
        let f = [at(x, base), at(x + w, base), at(x + w, base - h), at(x, base - h)];
        p.add(egui::Shape::convex_polygon(f.to_vec(), Color32::from_gray(236), ink));
        let side = [at(x + w, base), at(x + w + dx, base + dy), at(x + w + dx, base - h + dy), at(x + w, base - h)];
        p.add(egui::Shape::convex_polygon(side.to_vec(), Color32::from_gray(205), ink));
        let roof = [at(x, base - h), at(x + w, base - h), at(x + w + dx, base - h + dy), at(x + dx, base - h + dy)];
        p.add(egui::Shape::convex_polygon(roof.to_vec(), Color32::from_gray(250), ink));
        // Windows.
        let rows = (h / 12.0) as i32;
        for k in 0..rows.min(8) {
            let yy = base - 8.0 - k as f32 * 12.0;
            p.line_segment([at(x + 4.0, yy), at(x + w - 4.0, yy)], faint);
        }
    };
    block(14.0, 128.0, 30.0, 70.0, 22.0);
    block(66.0, 122.0, 26.0, 92.0, 20.0);
    block(98.0, 138.0, 32.0, 48.0, 22.0);
    p.line_segment([at(4.0, 140.0), at(150.0, 140.0)], faint);
    p.text(
        r.right_bottom() - vec2(10.0, 12.0),
        egui::Align2::RIGHT_BOTTOM,
        crate::tl!("Start Drawing"),
        egui::FontId::proportional(26.0),
        Color32::from_gray(if hover { 90 } else { 120 }),
    );
}

fn draw_link_icon(p: &egui::Painter, r: Rect, glyph: &str, c: Color32) {
    let s = Stroke::new(1.3, c);
    match glyph {
        "folder" => {
            let pts = vec![
                r.left_bottom(),
                r.left_top() + vec2(0.0, 3.0),
                r.left_top() + vec2(8.0, 3.0),
                r.left_top() + vec2(11.0, 6.0),
                r.right_top() + vec2(-3.0, 6.0),
                r.right_bottom() + vec2(-3.0, 0.0),
            ];
            p.add(egui::Shape::closed_line(pts, s));
            p.line_segment([r.left_bottom(), r.left_top() + vec2(5.0, 10.0)], s);
            p.line_segment([r.left_top() + vec2(5.0, 10.0), r.right_top() + vec2(0.0, 10.0)], s);
            p.line_segment([r.right_top() + vec2(0.0, 10.0), r.right_bottom() + vec2(-3.0, 0.0)], s);
        }
        "sample" => {
            let page = Rect::from_min_size(r.min + vec2(6.0, 0.0), vec2(14.0, r.height()));
            p.rect_stroke(page, 1.0, s, egui::StrokeKind::Inside);
            for k in 0..3 {
                let y = page.top() + 6.0 + k as f32 * 4.0;
                p.line_segment([pos2(page.left() + 3.0, y), pos2(page.right() - 3.0, y)], Stroke::new(1.0, c));
            }
        }
        _ => {
            let page = Rect::from_min_size(r.min + vec2(6.0, 0.0), vec2(14.0, r.height()));
            p.rect_stroke(page, 1.0, s, egui::StrokeKind::Inside);
            p.line_segment([page.center() - vec2(4.0, 0.0), page.center() + vec2(4.0, 0.0)], s);
            p.line_segment([page.center() - vec2(0.0, 4.0), page.center() + vec2(0.0, 4.0)], s);
        }
    }
}

/// A push pin: upright when pinned, lying when not.
fn draw_pin(p: &egui::Painter, r: Rect, pinned: bool, c: Color32) {
    let s = Stroke::new(1.4, c);
    let m = r.center();
    if pinned {
        p.line_segment([m + vec2(0.0, -7.0), m + vec2(0.0, 1.0)], Stroke::new(4.0, c));
        p.line_segment([m + vec2(-5.0, 1.0), m + vec2(5.0, 1.0)], s);
        p.line_segment([m + vec2(0.0, 1.0), m + vec2(0.0, 8.0)], s);
    } else {
        p.line_segment([m + vec2(-7.0, 0.0), m + vec2(1.0, 0.0)], Stroke::new(4.0, c));
        p.line_segment([m + vec2(1.0, -5.0), m + vec2(1.0, 5.0)], s);
        p.line_segment([m + vec2(1.0, 0.0), m + vec2(8.0, 0.0)], s);
    }
}

/// Per frame: the thumbnail of a document just opened or saved, from what the canvas shows.
pub fn capture(app: &mut CadApp) {
    let Some(path) = app.ui.thumb_pending.clone() else {
        track_saves(app);
        return;
    };
    let Ok(st) = app.session.state() else { return };
    if st.path.as_deref() != Some(path.as_str()) && st.path.as_deref().map(crate::menus::file_name) != Some(crate::menus::file_name(&path)) {
        app.ui.thumb_pending = None;
        return;
    }
    if app.canvas.zoom_pending || app.canvas.building() {
        return;
    }
    let Some(list) = app.canvas.list.as_ref() else { return };
    let thumb = encode(list);
    if !thumb.is_empty() {
        app.ui.recent_meta.entry(path).or_default().thumb = thumb;
    }
    app.ui.thumb_pending = None;
}

/// A save (the drawing turning clean) refreshes its thumbnail.
fn track_saves(app: &mut CadApp) {
    let Ok(st) = app.session.state() else { return };
    let dirty = st.is_dirty();
    let path = st.path.clone();
    if app.ui.last_dirty
        && !dirty
        && let Some(p) = path.filter(|p| app.ui.recent_files.contains(p))
    {
        app.ui.thumb_pending = Some(p);
    }
    app.ui.last_dirty = dirty;
}

/// Remember that `path` was opened now.
pub fn opened(app: &mut CadApp, path: &str) {
    let m = app.ui.recent_meta.entry(path.to_string()).or_default();
    m.opened = crate::now_ms();
    app.ui.thumb_pending = Some(path.to_string());
    // Forget documents no longer listed.
    let listed: std::collections::HashSet<&String> = app.ui.recent_files.iter().collect();
    let keep: BTreeMap<String, RecentMeta> =
        app.ui.recent_meta.iter().filter(|(k, v)| listed.contains(k) || v.pinned).map(|(k, v)| (k.clone(), v.clone())).collect();
    app.ui.recent_meta = keep;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_local_time() {
        // 2026-10-20 03:14:08 UTC is 10:14:08 AM in Bangkok (UTC+7).
        let ms = 1_792_466_048_000.0;
        assert_eq!(format_time(ms, 420), "Tuesday, October 20, 2026 10:14:08 AM");
        assert_eq!(group_of(ms - 86_400_000.0, ms, 420), "YESTERDAY");
        assert_eq!(group_of(ms, ms, 420), "TODAY");
    }

    #[test]
    fn thumbnails_round_trip() {
        let mut d = cadcraft_doc::Drawing::default();
        d.add(
            &cadcraft_doc::Space::Model,
            Default::default(),
            cadcraft_doc::EntityKind::Line(cadcraft_doc::Line {
                a: cadcraft_geom::Vec3::new(0.0, 0.0, 0.0),
                b: cadcraft_geom::Vec3::new(100.0, 50.0, 0.0),
            }),
        )
        .unwrap();
        let list = cadcraft_render::build(&d, &cadcraft_doc::Space::Model, &Default::default());
        let th = decode(&encode(&list));
        assert_eq!(th.len(), 1);
        let pts = &th[0].1;
        assert!((pts[0][0] - 0.0).abs() < 1e-3 && (pts[1][0] - 1.0).abs() < 1e-3);
        assert!(decode("not base64 !!").is_empty());
    }
}
