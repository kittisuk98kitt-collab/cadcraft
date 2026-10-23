//! A crossing box selects what it touches, and only that: dimensions and blocks by the lines
//! they draw, not by their bounding rectangle.

use cadcraft_engine::Session;
use cadcraft_engine::doc::{Handle, Space};
use cadcraft_engine::geom::{Bounds2, Vec2};
use serde_json::{Value, json};

fn add(s: &mut Session, id: &str, p: Value) -> Handle {
    let r = s.execute(id, &p).unwrap();
    let h = r.get("handle").and_then(Value::as_str).or_else(|| r.get("handles").and_then(|h| h.get(0)).and_then(Value::as_str)).unwrap();
    Handle::parse_hex(h).unwrap()
}

#[test]
fn a_crossing_box_selects_only_the_dimensions_it_touches() {
    let mut s = Session::new();
    s.execute("dimstyle", &json!({ "name": "S", "scale": 1, "textHeight": 2.5 })).unwrap();
    // A vertical chain 25 / 25 and the overall 50 further out, as AUTODIM lays them.
    let d1 = add(&mut s, "dimlinear", json!({ "p1": [0, 0], "p2": [0, 25], "at": [15, 12], "rotation": 90 }));
    let d2 = add(&mut s, "dimlinear", json!({ "p1": [0, 25], "p2": [0, 50], "at": [15, 37], "rotation": 90 }));
    let d3 = add(&mut s, "dimlinear", json!({ "p1": [0, 0], "p2": [0, 50], "at": [30, 25], "rotation": 90 }));
    // A small crossing box over the 50's dimension line only, low down beside the chain.
    let bx = Bounds2::new(Vec2::new(28.0, 2.0), Vec2::new(38.0, 9.0));
    let d = s.doc().unwrap();
    let got = cadcraft_engine::select::select_window(d, &Space::Model, bx, true);
    assert!(got.contains(&d3), "the 50 it crosses");
    assert!(!got.contains(&d1) && !got.contains(&d2), "not the chain beside it: {got:?}");
    // A box in empty space inside a dimension's extents selects nothing.
    let inside = Bounds2::new(Vec2::new(20.0, 30.0), Vec2::new(24.0, 33.0));
    assert!(cadcraft_engine::select::select_window(d, &Space::Model, inside, true).is_empty());
}

#[test]
fn a_crossing_box_selects_a_block_by_its_lines() {
    let mut s = Session::new();
    let a = add(&mut s, "line", json!({ "points": [[0, 0], [100, 0]] }));
    let b = add(&mut s, "line", json!({ "points": [[0, 0], [0, 100]] }));
    s.execute("block", &json!({ "name": "L", "base": [0, 0], "handles": [a.hex(), b.hex()] })).unwrap();
    let ins = add(&mut s, "insert", json!({ "name": "L", "at": [0, 0] }));
    let d = s.doc().unwrap();
    // In the empty corner of the L, inside its bounding box.
    let empty = Bounds2::new(Vec2::new(50.0, 50.0), Vec2::new(60.0, 60.0));
    assert!(!cadcraft_engine::select::select_window(d, &Space::Model, empty, true).contains(&ins));
    let touching = Bounds2::new(Vec2::new(50.0, -5.0), Vec2::new(60.0, 5.0));
    assert!(cadcraft_engine::select::select_window(d, &Space::Model, touching, true).contains(&ins));
}

#[test]
fn a_crossing_box_on_one_autodim_dimension_selects_just_it() {
    let mut s = Session::new();
    s.execute("dimstyle", &json!({ "name": "S", "scale": 1, "textHeight": 2.5 })).unwrap();
    let mut hs = vec![add(&mut s, "rectang", json!({ "p1": [0, 0], "p2": [40, 50] })).hex()];
    hs.push(add(&mut s, "circle", json!({ "center": [-10, 25], "radius": 7 })).hex());
    hs.push(add(&mut s, "circle", json!({ "center": [20, 25], "radius": 7 })).hex());
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let added: Vec<Handle> = r["handles"].as_array().unwrap().iter().filter_map(|h| h.as_str().and_then(Handle::parse_hex)).collect();
    let d = s.doc().unwrap();
    // The overall vertical dimension (the outermost one on the right).
    let mut verts: Vec<(f64, Handle)> = added
        .iter()
        .filter_map(|h| match &d.entity(*h)?.kind {
            cadcraft_engine::doc::EntityKind::Dimension(dm)
                if matches!(dm.kind, cadcraft_engine::doc::DimKind::Linear { rotation } if (rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9) =>
            {
                Some((dm.defpt.x, *h))
            }
            _ => None,
        })
        .collect();
    verts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (x, outer) = *verts.last().unwrap();
    let bx = Bounds2::new(Vec2::new(x - 0.5, 2.0), Vec2::new(x + 8.0, 9.0));
    let got = cadcraft_engine::select::select_window(d, &Space::Model, bx, true);
    assert_eq!(got, vec![outer], "verticals {verts:?}");
}
