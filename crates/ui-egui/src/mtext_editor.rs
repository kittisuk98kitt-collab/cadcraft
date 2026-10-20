//! The in-place MTEXT editor: while MTEXT asks for its text, a text box opens on the canvas at
//! the text's location and width with a formatting toolbar (font, height, bold, italic,
//! underline, overline, colour, alignment, lists, tabs) and a ruler for the paragraph's indents
//! and tab stops. The text and its formats live in the engine's
//! [`cadcraft_engine::mtext_edit::Formatted`]; this file only draws and forwards edits.

use std::sync::Arc;

use cadcraft_engine::mtext_edit::{CharFmt, Flag, Formatted, ListStyle, MTextColor, ParaAlign, StackProps, TabKind};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Shape, Stroke, TextFormat, pos2, vec2};

use crate::CadApp;
use crate::theme::Tokens;

/// Smallest and largest size of the text in the box, in points: the box stays readable at any zoom.
const MIN_PX: f32 = 13.0;
const MAX_PX: f32 = 36.0;
/// Families the font list offers besides the one set in the text.
const FAMILIES: [&str; 13] = [
    "Arial",
    "Times New Roman",
    "Courier New",
    "Verdana",
    "Georgia",
    "Calibri",
    "Tahoma",
    "MS Gothic",
    "txt",
    "simplex",
    "romans",
    "isocp",
    "bigfont",
];

/// Format changes kept for the editor's Undo.
const MAX_UNDO: usize = 100;
/// The standard colours (ACI 1..=7) of the colour menu.
const SWATCHES: [u16; 7] = [1, 2, 3, 4, 5, 6, 7];
/// Ruler snap, in text heights.
const SNAP: f64 = 0.25;

#[derive(Clone)]
struct State {
    fmt: Formatted,
    /// Format of the next typed character when nothing is selected.
    typing: CharFmt,
    /// Caret the `typing` format was set for.
    typing_at: usize,
    /// Height of the toolbar and ruler above the text box (last frame).
    top: f32,
    /// Selection (characters) of the last frame, for toolbar actions of this one.
    sel: (usize, usize),
    /// A caret position to set on the text box in the next frame (after a toolbar insertion).
    caret: Option<usize>,
    /// Take the keyboard focus in the next frame (the editor just opened).
    focus: bool,
    /// Earlier states for Undo, and undone ones for Redo.
    undo: Vec<Formatted>,
    redo: Vec<Formatted>,
    /// The state Undo history last saw.
    snap: Formatted,
    /// Whether the ruler is shown.
    no_ruler: bool,
    /// Stack Properties open: the stack's character and the values being edited.
    stack_edit: Option<(usize, StackProps)>,
}

fn state_id() -> egui::Id {
    egui::Id::new("mtext_editor_state")
}

fn edit_id() -> egui::Id {
    egui::Id::new("mtext_editor_text")
}

/// The ACI index or true colour as a screen colour.
fn color32(c: MTextColor) -> Color32 {
    let rgb = match c {
        MTextColor::Aci(v) => cadcraft_color::aci_rgb(u8::try_from(v).unwrap_or(7)),
        MTextColor::Rgb(v) => cadcraft_color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
    };
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// Show the editor while MTEXT collects its text; clears its state when it isn't running.
pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    let mtext = app.session.editing_mtext();
    let asking = app.session.current_prompt().is_some_and(|p| p.accept.text && !p.accept.point);
    let (Some(mt), true) = (mtext, asking) else {
        ctx.data_mut(|d| d.remove::<State>(state_id()));
        return;
    };
    let t = Tokens::get();
    let (rect, scale) = app.canvas.xf.map_or((ctx.content_rect(), 20.0), |x| (x.rect, x.scale.max(1e-6)));
    let mut st = ctx.data_mut(|d| d.get_temp::<State>(state_id())).unwrap_or_else(|| {
        // Editing an object's text (double-click, TEXTEDIT): its contents; MTEXT: empty.
        let mut fmt = if mt.contents.is_empty() { Formatted::default() } else { Formatted::from_contents(&mt.contents) };
        if let Some(f) = app.session.doc().ok().and_then(|d| d.text_style(&mt.style).map(|s| s.font.clone())) {
            // A family name, not a font file (`txt.shx`, `arial.ttf`).
            if !f.is_empty() && !f.contains('.') {
                fmt.base_font = f;
            }
        }
        // Typing goes into the box at once, after the text being edited.
        let end = fmt.len();
        let snap = fmt.clone();
        State {
            fmt,
            typing: CharFmt::default(),
            typing_at: end,
            top: 70.0,
            sel: (end, end),
            caret: Some(end),
            focus: true,
            undo: Vec::new(),
            redo: Vec::new(),
            snap,
            no_ruler: false,
            stack_edit: None,
        }
    });
    // Undo history: each change since the last frame is one step.
    if st.fmt != st.snap {
        let prev = std::mem::replace(&mut st.snap, st.fmt.clone());
        st.undo.push(prev);
        if st.undo.len() > MAX_UNDO {
            st.undo.remove(0);
        }
        st.redo.clear();
    }
    let styles: Vec<(String, String)> =
        app.session.doc().map(|d| d.text_styles.iter().map(|s| (s.name.clone(), s.font.clone())).collect()).unwrap_or_default();
    let base_h = if mt.height > 0.0 { mt.height } else { 0.2 };
    let px = ((base_h * scale) as f32).clamp(MIN_PX, MAX_PX);
    let width = if mt.width > 0.0 { ((mt.width * scale) as f32).clamp(280.0, rect.width().max(280.0) * 0.9) } else { 380.0 };
    // The box's top-left corner on screen (rotation is ignored while editing).
    let code = usize::from(mt.attach.clamp(1, 9) - 1);
    let (col, row) = ((code % 3) as f64, (code / 3) as f64);
    let ww = f64::from(width) / scale;
    let corner = cadcraft_geom::Vec2::new(mt.insert.x - col * ww / 2.0, mt.insert.y + row * base_h * 2.0);
    let at = app.canvas.xf.map_or(rect.center(), |x| x.to_screen(corner));
    let max_x = (rect.right() - width - 8.0).max(rect.left());
    // The whole editor (toolbar, box, stacks, buttons) stays inside the drawing area.
    let pos = pos2(at.x.clamp(rect.left(), max_x), at.y.clamp(rect.top() + st.top, (rect.bottom() - 190.0).max(rect.top() + st.top)));

    let mut done = Done::No;
    let top_start = pos.y - st.top;
    egui::Area::new(egui::Id::new("mtext_editor_area")).order(egui::Order::Foreground).fixed_pos(pos2(pos.x, top_start)).show(ctx, |ui| {
        egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.accent)).inner_margin(egui::Margin::same(0)).show(ui, |ui| {
            ui.set_width(width);
            let y0 = ui.cursor().top();
            toolbar(ui, &mut st, base_h, &mt.style, &styles);
            format_row(ui, &mut st);
            if !st.no_ruler {
                ruler(ui, &mut st, px, width, base_h);
            }
            st.top = ui.cursor().top() - y0;
            text_box(ui, &mut st, base_h, px, width, &t);
            stacks_row(ui, &mut st, &t, base_h);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if ui.button(crate::tl!("OK")).clicked() {
                    done = Done::Ok;
                }
                if ui.button(crate::tl!("Cancel")).clicked() {
                    done = Done::Cancel;
                }
                let finish = if ui.ctx().os().is_mac() { crate::tl!("⌘↩ to finish") } else { crate::tl!("Ctrl+Enter to finish") };
                ui.label(RichText::new(finish).small().color(t.text_dim));
            });
        });
    });
    let stack_open = st.stack_edit.is_some();
    stack_dialog(ctx, &mut st);
    if stack_open {
        // Esc closes Stack Properties, not the editor.
    } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        done = Done::Cancel;
    } else if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
        done = Done::Ok;
    }
    match done {
        Done::No => {
            ctx.data_mut(|d| d.insert_temp(state_id(), st));
        }
        Done::Ok => {
            ctx.data_mut(|d| d.remove::<State>(state_id()));
            // The editor holds the whole contents: one input, then Enter ends MTEXT's text entry.
            if !st.fmt.is_empty() {
                let _ = app.session.input(cadcraft_engine::Input::Text(st.fmt.to_contents()));
            }
            if app.session.running.as_ref().is_some_and(|r| r.id == "mtext" || r.id == "textedit") {
                let _ = app.session.input(cadcraft_engine::Input::Enter);
            }
        }
        Done::Cancel => {
            ctx.data_mut(|d| d.remove::<State>(state_id()));
            app.session.cancel();
        }
    }
}

#[derive(PartialEq)]
enum Done {
    No,
    Ok,
    Cancel,
}

/// Change the selected characters' format, or the next typed ones' when nothing is selected.
fn change(st: &mut State, f: impl Fn(&mut CharFmt)) {
    let (a, b) = st.sel;
    if a == b {
        f(&mut st.typing);
    } else {
        st.fmt.edit(a.min(b)..a.max(b), f);
    }
}

fn current(st: &State) -> CharFmt {
    let (a, b) = st.sel;
    if a == b { st.typing.clone() } else { st.fmt.fmt_at(a.min(b)) }
}

fn toggle(st: &mut State, flag: Flag) {
    let (a, b) = st.sel;
    if a == b {
        let on = !flag_of(&st.typing, flag);
        change(st, |c| set_flag(c, flag, on));
    } else {
        st.fmt.toggle(a.min(b)..a.max(b), flag);
    }
}

fn flag_of(c: &CharFmt, f: Flag) -> bool {
    match f {
        Flag::Bold => c.bold,
        Flag::Italic => c.italic,
        Flag::Underline => c.underline,
        Flag::Overline => c.overline,
        Flag::Strike => c.strike,
    }
}

fn set_flag(c: &mut CharFmt, f: Flag, on: bool) {
    match f {
        Flag::Bold => c.bold = on,
        Flag::Italic => c.italic = on,
        Flag::Underline => c.underline = on,
        Flag::Overline => c.overline = on,
        Flag::Strike => c.strike = on,
    }
}

/// A text style's font as a family name (`arial.ttf` → `arial`, `txt.shx` → `txt`).
fn family_of(font: &str) -> String {
    let f = font.trim();
    let stem = f.rsplit(['/', '\\']).next().unwrap_or(f);
    stem.rsplit_once('.').map_or(stem, |(a, _)| a).to_string()
}

fn toolbar(ui: &mut egui::Ui, st: &mut State, base_h: f64, style: &str, styles: &[(String, String)]) {
    let cur = current(st);
    let range = st.sel.0.min(st.sel.1)..st.sel.0.max(st.sel.1);
    let mut refocus = false;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        // Text style: its font for the whole text.
        egui::ComboBox::from_id_salt("mtext_style")
            .width(90.0)
            .selected_text(style)
            .show_ui(ui, |ui| {
                for (name, font) in styles {
                    if ui.selectable_label(name.eq_ignore_ascii_case(style), name).clicked() {
                        let fam = family_of(font);
                        if !fam.is_empty() {
                            st.fmt.base_font = fam;
                            let all = 0..st.fmt.len();
                            st.fmt.edit(all, |c| c.font = None);
                            st.typing.font = None;
                        }
                        refocus = true;
                    }
                }
            })
            .response
            .on_hover_text(crate::tl!("Text style"));
        // Font
        let shown = cur.font.clone().unwrap_or_else(|| st.fmt.base_font.clone());
        egui::ComboBox::from_id_salt("mtext_font").width(120.0).selected_text(shown.clone()).show_ui(ui, |ui| {
            for fam in FAMILIES.iter().map(|s| s.to_string()).chain(cur.font.clone().filter(|f| !FAMILIES.contains(&f.as_str()))) {
                if ui.selectable_label(shown == fam, &fam).clicked() {
                    change(st, |c| c.font = Some(fam.clone()));
                    refocus = true;
                }
            }
        });
        // Height
        let mut h = cur.height.unwrap_or(base_h);
        if ui
            .add(egui::DragValue::new(&mut h).speed(base_h * 0.02).range(base_h * 0.05..=base_h * 50.0).max_decimals(4))
            .on_hover_text(crate::tl!("Text height"))
            .changed()
        {
            let none = (h - base_h).abs() < 1e-9;
            change(st, |c| c.height = (!none).then_some(h));
        }
        ui.separator();
        for (flag, label, tip) in [
            (Flag::Bold, RichText::new("B").strong(), crate::tl!("Bold")),
            (Flag::Italic, RichText::new("I").italics(), crate::tl!("Italic")),
            (Flag::Underline, RichText::new("U").underline(), crate::tl!("Underline")),
            (Flag::Strike, RichText::new("S").strikethrough(), crate::tl!("Strikethrough")),
            (Flag::Overline, RichText::new("O"), crate::tl!("Overline")),
        ] {
            if ui.selectable_label(flag_of(&cur, flag), label).on_hover_text(tip).clicked() {
                toggle(st, flag);
                refocus = true;
            }
        }
        // Colour
        let swatch = cur.color.map_or(ui.visuals().text_color(), color32);
        ui.menu_button(RichText::new(" A ").color(swatch).strong(), |ui| {
            if ui.button(crate::tl!("ByLayer")).clicked() {
                change(st, |c| c.color = None);
                refocus = true;
                ui.close();
            }
            ui.horizontal(|ui| {
                for v in SWATCHES {
                    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    ui.painter().rect_filled(r, 2.0, color32(MTextColor::Aci(v)));
                    if resp.clicked() {
                        change(st, |c| c.color = Some(MTextColor::Aci(v)));
                        refocus = true;
                        ui.close();
                    }
                }
            });
            let mut rgb = cur.color.map_or([255, 255, 255], |c| {
                let k = color32(c);
                [k.r(), k.g(), k.b()]
            });
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                let v = (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                change(st, |c| c.color = Some(MTextColor::Rgb(v)));
            }
        })
        .response
        .on_hover_text(crate::tl!("Text color"));
        ui.separator();
        if ui.add_enabled(!st.undo.is_empty(), egui::Button::new("<")).on_hover_text(crate::tl!("Undo")).clicked() {
            if let Some(prev) = st.undo.pop() {
                st.redo.push(std::mem::replace(&mut st.fmt, prev));
                st.snap = st.fmt.clone();
                clamp_sel(st);
            }
            refocus = true;
        }
        if ui.add_enabled(!st.redo.is_empty(), egui::Button::new(">")).on_hover_text(crate::tl!("Redo")).clicked() {
            if let Some(next) = st.redo.pop() {
                st.undo.push(std::mem::replace(&mut st.fmt, next));
                st.snap = st.fmt.clone();
                clamp_sel(st);
            }
            refocus = true;
        }
        ui.menu_button(RichText::new("A").strikethrough(), |ui| {
            let all = 0..st.fmt.len();
            let r = if range.is_empty() { all.clone() } else { range.clone() };
            if ui.button(crate::tl!("Remove Character Formatting")).clicked() {
                st.fmt.edit(r.clone(), |c| *c = CharFmt::default());
                st.typing = CharFmt::default();
                refocus = true;
                ui.close();
            }
            if ui.button(crate::tl!("Remove Paragraph Formatting")).clicked() {
                st.fmt.edit_paras(r.clone(), |p| *p = Default::default());
                refocus = true;
                ui.close();
            }
            if ui.button(crate::tl!("Remove All Formatting")).clicked() {
                st.fmt.edit(all.clone(), |c| *c = CharFmt::default());
                st.fmt.edit_paras(all.clone(), |p| *p = Default::default());
                st.typing = CharFmt::default();
                refocus = true;
                ui.close();
            }
        })
        .response
        .on_hover_text(crate::tl!("Remove formatting"));
        if ui.selectable_label(!st.no_ruler, "Ruler").on_hover_text(crate::tl!("Ruler")).clicked() {
            st.no_ruler = !st.no_ruler;
        }
        ui.separator();
        // Paragraph alignment
        let para = st.fmt.para(st.fmt.paragraph_of(range.start));
        for (al, glyph, tip) in [
            (ParaAlign::Left, "L", crate::tl!("Left")),
            (ParaAlign::Center, "C", crate::tl!("Center")),
            (ParaAlign::Right, "R", crate::tl!("Right")),
            (ParaAlign::Justify, "J", crate::tl!("Justify")),
            (ParaAlign::Distributed, "D", crate::tl!("Distribute")),
        ] {
            let on = para.align == al;
            if ui.selectable_label(on, glyph).on_hover_text(tip).clicked() {
                st.fmt.edit_paras(range.clone(), |p| p.align = if on { ParaAlign::Default } else { al });
                refocus = true;
            }
        }
        ui.separator();
        // Lists and special characters
        if ui.button("• List").on_hover_text(crate::tl!("Bulleted list")).clicked() {
            st.fmt.make_list(range.clone(), ListStyle::Bullet, 3.0 * base_h);
            refocus = true;
        }
        if ui.button("1. List").on_hover_text(crate::tl!("Numbered list")).clicked() {
            st.fmt.make_list(range.clone(), ListStyle::Number, 3.0 * base_h);
            refocus = true;
        }
        ui.separator();
        // Stack: the selection ("+0.033^0", "1/2") as stacked text, as AutoCAD's b/a button.
        let sel_text: String = st.fmt.text().chars().skip(range.start).take(range.len()).collect();
        let can_stack = sel_text.contains(['^', '/', '#']) && !sel_text.contains('\n');
        if ui
            .add_enabled(can_stack, egui::Button::new(RichText::new("b/a")))
            .on_hover_text(crate::tl!("Stack (select +0.033^0, 1/2 or 1#2)"))
            .clicked()
        {
            st.fmt.stack(range.clone());
            st.caret = Some(range.start + 1);
            refocus = true;
        }
        let has_sel = !range.is_empty();
        if ui.add_enabled(has_sel, egui::Button::new("x²")).on_hover_text(crate::tl!("Superscript")).clicked() {
            let new: String = st
                .fmt
                .text()
                .chars()
                .take(range.start)
                .chain(sel_text.chars())
                .chain(std::iter::once('^'))
                .chain(st.fmt.text().chars().skip(range.end))
                .collect();
            if st.fmt.set_text(&new, &current(st)) {
                st.fmt.stack(range.start..range.end + 1);
            }
            refocus = true;
        }
        if ui.add_enabled(has_sel, egui::Button::new("x₂")).on_hover_text(crate::tl!("Subscript")).clicked() {
            let new: String = st
                .fmt
                .text()
                .chars()
                .take(range.start)
                .chain(std::iter::once('^'))
                .chain(sel_text.chars())
                .chain(st.fmt.text().chars().skip(range.end))
                .collect();
            if st.fmt.set_text(&new, &current(st)) {
                st.fmt.stack(range.start..range.end + 1);
            }
            refocus = true;
        }
        if ui.add_enabled(has_sel, egui::Button::new("AA")).on_hover_text(crate::tl!("Uppercase")).clicked() {
            replace_sel(st, &range, &sel_text.to_uppercase());
            refocus = true;
        }
        if ui.add_enabled(has_sel, egui::Button::new("aa")).on_hover_text(crate::tl!("Lowercase")).clicked() {
            replace_sel(st, &range, &sel_text.to_lowercase());
            refocus = true;
        }
        ui.menu_button(" @ ", |ui| {
            for (sym, name) in [
                ("°", crate::tl!("Degree %%d")),
                ("±", crate::tl!("Plus/Minus %%p")),
                ("Ø", crate::tl!("Diameter %%c")),
                ("≈", crate::tl!("Almost equal")),
                ("∠", crate::tl!("Angle")),
                ("℄", crate::tl!("Center line")),
                ("Δ", crate::tl!("Delta")),
                ("≠", crate::tl!("Not equal")),
                ("Ω", crate::tl!("Ohm")),
                ("²", crate::tl!("Squared")),
                ("³", crate::tl!("Cubed")),
                ("\u{a0}", crate::tl!("Non-breaking space")),
            ] {
                if ui.button(format!("{sym}   {name}")).clicked() {
                    insert(st, sym);
                    refocus = true;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(crate::tl!("Symbol"));
        for (glyph, tip, code) in [
            ("Tab", crate::tl!("Insert tab"), "\t"),
            ("°", crate::tl!("Degree"), "°"),
            ("±", crate::tl!("Plus/minus"), "±"),
            ("Ø", crate::tl!("Diameter"), "Ø"),
        ] {
            if ui.button(glyph).on_hover_text(tip).clicked() {
                insert(st, code);
                refocus = true;
            }
        }
    });
    if refocus {
        ui.ctx().memory_mut(|m| m.request_focus(edit_id()));
    }
}

/// Keep the selection inside the text (after Undo or Redo).
fn clamp_sel(st: &mut State) {
    let n = st.fmt.len();
    st.sel = (st.sel.0.min(n), st.sel.1.min(n));
    st.typing_at = st.typing_at.min(n);
    st.caret = Some(st.sel.1);
}

/// The second toolbar row: paragraph spacing, oblique angle, tracking and width factor.
fn format_row(ui: &mut egui::Ui, st: &mut State) {
    let cur = current(st);
    let range = st.sel.0.min(st.sel.1)..st.sel.0.max(st.sel.1);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        let para = st.fmt.para(st.fmt.paragraph_of(range.start));
        ui.menu_button(crate::tl!("Paragraph"), |ui| {
            egui::Grid::new("mtext_para").num_columns(2).show(ui, |ui| {
                for (k, label) in [crate::tl!("Space before"), crate::tl!("Space after")].into_iter().enumerate() {
                    ui.label(label);
                    let mut v = if k == 0 { para.before } else { para.after };
                    if ui.add(egui::DragValue::new(&mut v).speed(0.05).range(0.0..=10.0).max_decimals(2)).changed() {
                        st.fmt.edit_paras(range.clone(), |p| if k == 0 { p.before = v } else { p.after = v });
                    }
                    ui.end_row();
                }
            });
        })
        .response
        .on_hover_text(crate::tl!("Paragraph spacing"));
        ui.separator();
        // Oblique angle, tracking and width factor of the selection (or the next typed text).
        let mut q = cur.oblique.unwrap_or(0.0);
        ui.label("0/").on_hover_text(crate::tl!("Oblique Angle"));
        if ui.add(egui::DragValue::new(&mut q).speed(0.5).range(-85.0..=85.0).max_decimals(1)).on_hover_text(crate::tl!("Oblique Angle")).changed() {
            change(st, |c| c.oblique = (q.abs() > 1e-9).then_some(q));
        }
        let mut t = cur.track.unwrap_or(1.0);
        ui.label("a-b").on_hover_text(crate::tl!("Tracking"));
        if ui.add(egui::DragValue::new(&mut t).speed(0.01).range(0.75..=4.0).max_decimals(3)).on_hover_text(crate::tl!("Tracking")).changed() {
            change(st, |c| c.track = ((t - 1.0).abs() > 1e-9).then_some(t));
        }
        let mut w = cur.width.unwrap_or(1.0);
        ui.label("W").on_hover_text(crate::tl!("Width Factor"));
        if ui.add(egui::DragValue::new(&mut w).speed(0.01).range(0.1..=10.0).max_decimals(3)).on_hover_text(crate::tl!("Width Factor")).changed() {
            change(st, |c| c.width = ((w - 1.0).abs() > 1e-9).then_some(w));
        }
    });
}

/// Replace the selected characters with `s` (same length: case changes), keeping formats.
fn replace_sel(st: &mut State, range: &std::ops::Range<usize>, s: &str) {
    let chars: Vec<char> = st.fmt.text().chars().collect();
    let new: String = chars.iter().take(range.start).copied().chain(s.chars()).chain(chars.iter().skip(range.end).copied()).collect();
    let keep: Vec<CharFmt> = (range.start..range.end).map(|i| st.fmt.fmt_at(i)).collect();
    if st.fmt.set_text(&new, &current(st)) && s.chars().count() == keep.len() {
        for (k, f) in keep.into_iter().enumerate() {
            st.fmt.edit(range.start + k..range.start + k + 1, |c| *c = f.clone());
        }
    }
}

/// The stacked texts (tolerances, fractions) of the text, each with Unstack.
/// Stack Properties: the stack's upper and lower text, style, position and size.
fn stack_dialog(ctx: &egui::Context, st: &mut State) {
    let Some((i, mut p)) = st.stack_edit.take() else { return };
    let mut answer: Option<bool> = None;
    let mut keep = true;
    egui::Window::new(crate::tl!("Stack Properties"))
        .id(egui::Id::new("mtext_stack_props"))
        .open(&mut keep)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.group(|ui| {
                        ui.label(crate::tl!("Text"));
                        egui::Grid::new("stack_text").num_columns(2).show(ui, |ui| {
                            ui.label(crate::tl!("Upper"));
                            ui.add(egui::TextEdit::singleline(&mut p.upper).desired_width(140.0));
                            ui.end_row();
                            ui.label(crate::tl!("Lower"));
                            ui.add(egui::TextEdit::singleline(&mut p.lower).desired_width(140.0));
                            ui.end_row();
                        });
                    });
                    ui.group(|ui| {
                        ui.label(crate::tl!("Appearance"));
                        egui::Grid::new("stack_look").num_columns(2).show(ui, |ui| {
                            ui.label(crate::tl!("Style"));
                            let styles = [
                                ('^', crate::tl!("Tolerance")),
                                ('/', crate::tl!("Fraction (Horizontal)")),
                                ('#', crate::tl!("Fraction (Diagonal)")),
                            ];
                            let cur = styles.iter().find(|(c, _)| *c == p.sep).map_or(styles[0].1, |(_, l)| *l);
                            egui::ComboBox::from_id_salt("stack_style").width(150.0).selected_text(cur).show_ui(ui, |ui| {
                                for (c, l) in styles {
                                    ui.selectable_value(&mut p.sep, c, l);
                                }
                            });
                            ui.end_row();
                            ui.label(crate::tl!("Position"));
                            let pos = [(2u8, crate::tl!("Top")), (1, crate::tl!("Center")), (0, crate::tl!("Bottom"))];
                            let cur = pos.iter().find(|(a, _)| *a == p.align).map_or(pos[1].1, |(_, l)| *l);
                            egui::ComboBox::from_id_salt("stack_pos").width(150.0).selected_text(cur).show_ui(ui, |ui| {
                                for (a, l) in pos {
                                    ui.selectable_value(&mut p.align, a, l);
                                }
                            });
                            ui.end_row();
                            ui.label(crate::tl!("Text size"));
                            let mut pct = (p.size * 100.0).round() as i32;
                            egui::ComboBox::from_id_salt("stack_size").width(150.0).selected_text(format!("{pct}%")).show_ui(ui, |ui| {
                                for v in [50, 60, 70, 80, 90, 100] {
                                    ui.selectable_value(&mut pct, v, format!("{v}%"));
                                }
                            });
                            p.size = f64::from(pct.clamp(25, 100)) / 100.0;
                            ui.end_row();
                        });
                    });
                });
                ui.vertical(|ui| {
                    if ui.add_sized([90.0, 24.0], egui::Button::new(crate::tl!("OK"))).clicked() {
                        answer = Some(true);
                    }
                    if ui.add_sized([90.0, 24.0], egui::Button::new(crate::tl!("Cancel"))).clicked() {
                        answer = Some(false);
                    }
                    ui.add_space(8.0);
                    if ui.add_sized([90.0, 24.0], egui::Button::new(crate::tl!("Defaults"))).clicked() {
                        p.sep = '^';
                        p.align = 1;
                        p.size = 0.7;
                    }
                });
            });
        });
    if !keep || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(false);
    }
    match answer {
        Some(true) => {
            st.fmt.set_stack(i, &p);
            st.caret = Some(i + 1);
        }
        Some(false) => {}
        None => st.stack_edit = Some((i, p)),
    }
}

fn stacks_row(ui: &mut egui::Ui, st: &mut State, t: &Tokens, base_h: f64) {
    let stacks: Vec<(usize, String)> = st
        .fmt
        .text()
        .chars()
        .enumerate()
        .filter_map(|(i, c)| {
            let code = st.fmt.atom_code(c)?;
            let body = code.get(code.find("\\S")? + 2..)?;
            Some((i, body.get(..body.find(';').unwrap_or(body.len()))?.to_string()))
        })
        .collect();
    if stacks.is_empty() {
        return;
    }
    let mut unstack = None;
    let mut props = None;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(crate::tl!("Stacked:")).small().color(t.text_dim));
        for (i, body) in &stacks {
            let shown = body.replacen(['^', '/', '#'], " / ", 1);
            if ui.small_button(shown.clone()).on_hover_text(crate::tl!("Stack Properties")).clicked() {
                props = Some(*i);
            }
            if ui.small_button("x").on_hover_text(crate::tl!("Unstack")).clicked() {
                unstack = Some(*i);
            }
        }
    });
    if let Some(i) = unstack {
        st.fmt.unstack(i);
    }
    if let Some(i) = props
        && let Some(p) = st.fmt.stack_props(i, base_h)
    {
        st.stack_edit = Some((i, p));
    }
}

/// Insert `s` at the caret (over the selection).
fn insert(st: &mut State, s: &str) {
    let (a, b) = (st.sel.0.min(st.sel.1), st.sel.0.max(st.sel.1));
    let chars: Vec<char> = st.fmt.text().chars().collect();
    let new: String = chars.iter().take(a).chain(s.chars().collect::<Vec<_>>().iter()).chain(chars.iter().skip(b)).collect();
    if st.fmt.set_text(&new, &st.typing.clone()) {
        let at = a + s.chars().count();
        st.sel = (at, at);
        st.typing_at = at;
        st.caret = Some(at);
    }
}

/// The ruler: `px` points per text height (`base_h` drawing units); paragraph indents and tab
/// stops are in drawing units, as `\p` codes are.
fn ruler(ui: &mut egui::Ui, st: &mut State, px: f32, width: f32, base_h: f64) {
    let t = Tokens::get();
    let range = st.sel.0.min(st.sel.1)..st.sel.0.max(st.sel.1);
    let p = st.fmt.para(st.fmt.paragraph_of(range.start));
    let (rect, bg) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, t.control);
    let tick = |u: f64| rect.left() + (u as f32) * px;
    let x_of = |v: f64| tick(v / base_h);
    let units = (f64::from(width) / f64::from(px)).floor() as i32;
    for u in 0..=units.clamp(0, 400) {
        let x = tick(f64::from(u));
        let major = u % 5 == 0;
        painter.line_segment([pos2(x, rect.bottom() - if major { 7.0 } else { 3.0 }), pos2(x, rect.bottom())], Stroke::new(1.0, t.text_faint));
        if major && u > 0 {
            painter.text(pos2(x + 2.0, rect.top() + 1.0), Align2::LEFT_TOP, u.to_string(), FontId::proportional(8.0), t.text_faint);
        }
    }
    let to_units = |x: f32| ((f64::from(x - rect.left()) / f64::from(px) / SNAP).round() * SNAP).clamp(0.0, 400.0) * base_h;
    // Tab stops: an L at the stop; a right-click removes one.
    for (pos, _kind) in &p.tabs {
        let x = x_of(*pos);
        painter.add(Shape::line(
            vec![pos2(x, rect.bottom() - 9.0), pos2(x, rect.bottom() - 2.0), pos2(x + 4.0, rect.bottom() - 2.0)],
            Stroke::new(1.5, t.accent),
        ));
    }
    let hover = bg.hover_pos();
    // Indent markers: the first line's above, the left indent's below.
    let first_abs = p.left + p.first;
    let mut drag = None;
    for (k, (u, up)) in [(first_abs, false), (p.left, true)].into_iter().enumerate() {
        let x = x_of(u);
        let y = if up { rect.bottom() - 1.0 } else { rect.top() + 1.0 };
        let dir = if up { -1.0 } else { 1.0 };
        let hit = Rect::from_center_size(pos2(x, y + dir * 4.0), vec2(10.0, 10.0));
        let r = ui.interact(hit, ui.id().with(("mtext_indent", k)), Sense::drag());
        let col = if r.hovered() || r.dragged() { t.accent } else { t.icon };
        painter.add(Shape::convex_polygon(vec![pos2(x - 4.0, y), pos2(x + 4.0, y), pos2(x, y + dir * 8.0)], col, Stroke::NONE));
        if r.dragged()
            && let Some(m) = r.interact_pointer_pos()
        {
            drag = Some((k, to_units(m.x)));
        }
    }
    if let Some((k, u)) = drag {
        if k == 0 {
            st.fmt.set_indents(range.clone(), u - p.left, p.left);
        } else {
            st.fmt.set_indents(range.clone(), p.first, u);
        }
    } else if bg.clicked()
        && let Some(m) = hover
    {
        st.fmt.add_tab(range.clone(), to_units(m.x), TabKind::Left);
    } else if bg.secondary_clicked()
        && let Some(m) = hover
    {
        st.fmt.remove_tab(range, to_units(m.x), 0.6 * base_h);
    }
    bg.on_hover_text(crate::tl!("Click the ruler to add a tab stop, right-click to remove one"));
}

fn text_box(ui: &mut egui::Ui, st: &mut State, base_h: f64, px: f32, width: f32, t: &Tokens) {
    let id = edit_id();
    if let Some(at) = st.caret.take()
        && let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), id)
    {
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
        s.store(ui.ctx(), id);
    }
    let fmt = st.fmt.clone();
    let default_color = t.text;
    let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| -> Arc<egui::Galley> {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        let s = text.as_str();
        let mut run = String::new();
        let mut cur: Option<CharFmt> = None;
        let flush = |job: &mut LayoutJob, run: &mut String, f: &CharFmt| {
            if run.is_empty() {
                return;
            }
            let size = f.height.map_or(px, |h| (px * (h / base_h) as f32).clamp(6.0, 120.0));
            let color = f.color.map_or(default_color, color32);
            job.append(
                run,
                0.0,
                TextFormat {
                    font_id: FontId::proportional(size),
                    color,
                    underline: if f.underline { Stroke::new(1.0, color) } else { Stroke::NONE },
                    strikethrough: if f.strike { Stroke::new(1.0, color) } else { Stroke::NONE },
                    italics: f.italic || f.oblique.is_some_and(|q| q.abs() > 1.0),
                    // The editor has one weight of each font: bold is shown wider, as is tracking.
                    extra_letter_spacing: if f.bold { size * 0.06 } else { 0.0 } + f.track.map_or(0.0, |t| (t as f32 - 1.0) * size * 0.3),
                    ..TextFormat::default()
                },
            );
            run.clear();
        };
        for (i, c) in s.chars().enumerate() {
            let f = fmt.fmt_at(i);
            if cur.as_ref() != Some(&f) {
                if let Some(p) = &cur {
                    flush(&mut job, &mut run, p);
                }
                cur = Some(f);
            }
            // Codes kept whole show as one mark: a stack (tolerance, fraction) as ⁄, others as ◇.
            run.push(match fmt.atom_code(c) {
                Some(code) if code.starts_with("\\S") => '¦',
                Some(_) => '¤',
                None => c,
            });
        }
        if let Some(p) = &cur {
            flush(&mut job, &mut run, p);
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut buf = st.fmt.text().to_string();
    let out = egui::TextEdit::multiline(&mut buf)
        .id(id)
        .desired_width(width)
        .desired_rows(5)
        .margin(egui::Margin::symmetric(4, 4))
        .lock_focus(true)
        .char_limit(cadcraft_engine::mtext_edit::MAX_CHARS)
        .layouter(&mut layouter)
        .show(ui);
    if std::mem::take(&mut st.focus) || (!out.response.has_focus() && buf.is_empty() && st.fmt.is_empty()) {
        out.response.request_focus();
    }
    if out.response.changed() {
        let typing = st.typing.clone();
        st.fmt.set_text(&buf, &typing);
    }
    if let Some(r) = out.cursor_range {
        let (a, b) = (r.primary.index.0, r.secondary.index.0);
        st.sel = (a, b);
        if a == b {
            if out.response.changed() {
                st.typing_at = a;
            } else if a != st.typing_at {
                // The caret moved: typing continues the formatting of the character before it.
                st.typing = st.fmt.fmt_at(a.saturating_sub(1));
                st.typing_at = a;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_engine::Session;

    use crate::Services;

    #[test]
    fn editor_opens_only_for_mtext_text_entry_and_finishes_with_formatted_contents() {
        let ctx = egui::Context::default();
        let mut app = CadApp::new(Session::new(), Services::default());
        let frame = |app: &mut CadApp| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))), ..Default::default() };
            for _ in 0..2 {
                let mut out = ctx.run_ui(input.clone(), |ui| show(app, ui.ctx()));
                out.textures_delta.clear();
            }
        };
        frame(&mut app);
        assert!(ctx.data_mut(|d| d.get_temp::<State>(state_id())).is_none());
        app.start("mtext");
        assert!(app.session.editing_mtext().is_none(), "no box yet");
        app.session.input(cadcraft_engine::Input::Point(cadcraft_geom::Vec2::new(1.0, 1.0))).unwrap();
        app.session.input(cadcraft_engine::Input::Point(cadcraft_geom::Vec2::new(6.0, 0.0))).unwrap();
        let m = app.session.editing_mtext().expect("box known");
        assert!((m.width - 5.0).abs() < 1e-9, "{}", m.width);
        frame(&mut app);
        let mut st = ctx.data_mut(|d| d.get_temp::<State>(state_id())).expect("editor open");
        st.fmt.set_text("hello", &CharFmt::default());
        st.fmt.toggle(0..5, Flag::Bold);
        let contents = st.fmt.to_contents();
        app.session.input(cadcraft_engine::Input::Text(contents.clone())).unwrap();
        app.session.input(cadcraft_engine::Input::Enter).unwrap();
        let d = app.session.doc().unwrap();
        let found = d.model.iter().any(|e| matches!(&e.kind, cadcraft_engine::doc::EntityKind::MText(m) if m.contents == contents));
        assert!(found, "{contents}");
    }
}
