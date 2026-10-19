//! The Enhanced Attribute Editor: a block reference's attributes as a Tag / Prompt / Value table,
//! the selected one's value edited below it. Double-clicking a block reference with attributes
//! opens it (as in AutoCAD). Apply and OK change the values through ATTEDIT, so each change is
//! one undo step and scriptable; the dialog itself never touches the drawing.

use cadcraft_engine::doc::{EntityKind, Handle};
use egui::{RichText, vec2};
use serde_json::{Map, Value, json};

use crate::CadApp;
use crate::theme::Tokens;

/// One attribute row: tag, prompt, value as edited.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub tag: String,
    pub prompt: String,
    pub value: String,
    /// The value in the drawing (to tell what changed).
    pub stored: String,
}

/// The dialog's state while it is open.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub handle: Option<Handle>,
    pub block: String,
    pub rows: Vec<Row>,
    pub selected: usize,
    /// Focus the value field on the next frame.
    focus: bool,
}

/// The editable attributes of block reference `h` (constant ones have no value to edit).
fn rows_of(app: &CadApp, h: Handle) -> Option<(String, Vec<Row>)> {
    let d = app.session.doc().ok()?;
    let EntityKind::Insert(i) = &d.entity(h)?.kind else { return None };
    // A dynamic block reference names its dynamic block, not its anonymous one (`*U12`).
    let block = cadcraft_engine::doc::dynblock::custom_properties(d, h).map_or_else(|| i.block.clone(), |(n, _)| n);
    // Attributes in a file rarely carry their prompt: it lives on the block's attribute definition.
    let def_prompt = |tag: &str| -> String {
        d.block(&i.block)
            .and_then(|b| {
                b.entities.iter().find_map(|e| match &e.kind {
                    EntityKind::AttDef(a) if a.tag.eq_ignore_ascii_case(tag) => Some(a.prompt.clone()),
                    _ => None,
                })
            })
            .unwrap_or_default()
    };
    let rows: Vec<Row> = i
        .attribs
        .iter()
        .filter(|a| !a.constant)
        .map(|a| {
            let prompt = if a.prompt.trim().is_empty() { def_prompt(&a.tag) } else { a.prompt.clone() };
            Row { tag: a.tag.clone(), prompt, value: a.text.value.clone(), stored: a.text.value.clone() }
        })
        .collect();
    (!rows.is_empty()).then_some((block, rows))
}

/// Open the editor on block reference `h`. `false` if it has no attributes to edit.
pub fn open(app: &mut CadApp, h: Handle) -> bool {
    let Some((block, rows)) = rows_of(app, h) else { return false };
    app.eattedit = State { handle: Some(h), block, rows, selected: 0, focus: true };
    app.ui.dialog = Some("eattedit".into());
    true
}

/// Write the changed values (one ATTEDIT, one undo step) and take them as the stored ones.
fn apply(app: &mut CadApp) {
    let Some(h) = app.eattedit.handle else { return };
    let mut values = Map::new();
    for r in app.eattedit.rows.iter().filter(|r| r.value != r.stored) {
        values.insert(r.tag.clone(), Value::String(r.value.clone()));
    }
    if values.is_empty() {
        return;
    }
    // Errors are echoed on the command line by `run`.
    if app.run("attedit", json!({ "handle": h.hex(), "values": values })).is_ok() {
        for r in &mut app.eattedit.rows {
            r.stored = r.value.clone();
        }
    }
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let t = Tokens::get();
    // The reference was erased (or undone away): nothing left to edit.
    if app.eattedit.handle.is_none_or(|h| app.session.doc().ok().and_then(|d| d.entity(h)).is_none()) {
        *open = false;
        return;
    }
    let mut action: Option<&str> = None;
    let mut keep = true;
    egui::Window::new(crate::tl!("Enhanced Attribute Editor"))
        .id(egui::Id::new("Enhanced Attribute Editor"))
        .open(&mut keep)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(440.0, 300.0))
        .show(ctx, |ui| {
            let st = &mut app.eattedit;
            let sel = st.selected.min(st.rows.len().saturating_sub(1));
            egui::Grid::new("eattedit_head").num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
                ui.label(RichText::new(crate::tl!("Block:")).color(t.text_dim));
                ui.label(&st.block);
                ui.end_row();
                ui.label(RichText::new(crate::tl!("Tag:")).color(t.text_dim));
                ui.label(st.rows.get(sel).map_or("", |r| r.tag.as_str()));
                ui.end_row();
            });
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                    egui::Grid::new("eattedit_rows").num_columns(3).striped(true).min_col_width(90.0).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                        for head in [crate::tl!("Tag"), crate::tl!("Prompt"), crate::tl!("Value")] {
                            ui.label(RichText::new(head).strong());
                        }
                        ui.end_row();
                        for (k, r) in st.rows.iter().enumerate() {
                            let on = k == sel;
                            if ui.selectable_label(on, &r.tag).clicked() {
                                st.selected = k;
                                st.focus = true;
                            }
                            if ui.selectable_label(on, &r.prompt).clicked() {
                                st.selected = k;
                                st.focus = true;
                            }
                            if ui.selectable_label(on, &r.value).clicked() {
                                st.selected = k;
                                st.focus = true;
                            }
                            ui.end_row();
                        }
                    });
                });
            });
            ui.add_space(6.0);
            let sel = st.selected.min(st.rows.len().saturating_sub(1));
            ui.horizontal(|ui| {
                ui.label(crate::tl!("Value:"));
                if let Some(r) = st.rows.get_mut(sel) {
                    let resp =
                        ui.add(egui::TextEdit::singleline(&mut r.value).id(egui::Id::new(("eattedit_value", sel))).desired_width(f32::INFINITY));
                    if std::mem::take(&mut st.focus) {
                        resp.request_focus();
                    }
                    // Enter moves to the next attribute, as Tab would.
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if sel + 1 < st.rows.len() {
                            st.selected = sel + 1;
                            st.focus = true;
                        } else {
                            action = Some("ok");
                        }
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let changed = st.rows.iter().any(|r| r.value != r.stored);
                if ui.add_enabled(changed, egui::Button::new(crate::tl!("Apply"))).clicked() {
                    action = Some("apply");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(crate::tl!("Cancel")).clicked() {
                        action = Some("cancel");
                    }
                    if ui.button(crate::tl!("OK")).clicked() {
                        action = Some("ok");
                    }
                });
            });
        });
    match action {
        Some("apply") => apply(app),
        Some("ok") => {
            apply(app);
            *open = false;
        }
        Some(_) => *open = false,
        None => {}
    }
    if !keep {
        *open = false;
    }
}
