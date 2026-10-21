//! AUTODIM dimensions a plate the way a shop drawing does; DIMCHECK finds what is missing.

use cadcraft_engine::Session;
use cadcraft_engine::doc::EntityKind;
use serde_json::{Value, json};

fn add(s: &mut Session, id: &str, p: Value) -> String {
    let r = s.execute(id, &p).unwrap();
    r.get("handle")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| r.get("handles").and_then(|h| h.get(0)).and_then(Value::as_str).map(str::to_string))
        .unwrap()
}

/// A 350 × 200 plate: 4 holes Ø14 at the corners, a row of six Ø8 holes 50 apart, two M10
/// tapped holes (8.5 tap drill inside the 10 circle), a slot 12 × 36, a fillet R10 and a C5
/// chamfer.
fn plate(s: &mut Session) -> Vec<String> {
    let mut hs = Vec::new();
    hs.push(add(
        s,
        "pline",
        json!({ "vertices": [
            {"p": [0, 0]}, {"p": [345, 0]}, {"p": [350, 5]}, {"p": [350, 190], "bulge": 0.41421356237}, {"p": [340, 200]}, {"p": [0, 200]}
        ], "closed": true }),
    ));
    for (x, y) in [(25.0, 30.0), (325.0, 30.0), (25.0, 170.0), (325.0, 170.0)] {
        hs.push(add(s, "circle", json!({ "center": [x, y], "radius": 7 })));
    }
    for k in 0..6 {
        hs.push(add(s, "circle", json!({ "center": [50.0 + 50.0 * k as f64, 100.0], "radius": 4 })));
    }
    for x in [120.0, 230.0] {
        hs.push(add(s, "circle", json!({ "center": [x, 40], "radius": 4.25 })));
        hs.push(add(s, "circle", json!({ "center": [x, 40], "radius": 5 })));
    }
    // Slot: centres (160, 160) and (184, 160), width 12.
    hs.push(add(
        s,
        "pline",
        json!({ "vertices": [
            {"p": [160, 154]}, {"p": [184, 154], "bulge": 1.0}, {"p": [184, 166]}, {"p": [160, 166], "bulge": 1.0}
        ], "closed": true }),
    ));
    hs
}

fn texts(s: &Session, handles: &[Value]) -> Vec<String> {
    let d = s.doc().unwrap();
    let mut out = Vec::new();
    for h in handles {
        let Some(h) = h.as_str().and_then(cadcraft_engine::doc::Handle::parse_hex) else { continue };
        if let Some(e) = d.entity(h) {
            match &e.kind {
                EntityKind::MText(m) => out.push(m.contents.clone()),
                EntityKind::Text(t) => out.push(t.value.clone()),
                EntityKind::Dimension(dm) if !dm.text.is_empty() => out.push(dm.text.clone()),
                _ => {}
            }
        }
    }
    out
}

#[test]
fn autodim_dimensions_a_plate_in_shop_drawing_style() {
    let mut s = Session::new();
    let hs = plate(&mut s);
    let r = s.execute("autodim", &json!({ "handles": hs, "thickness": 12 })).unwrap();
    let handles = r["handles"].as_array().unwrap().clone();
    let t = texts(&s, &handles);
    for want in ["4-14DRILL", "2-M10 THRU", "12x36 SLOT", "R10", "C5", "t=12", "P50x5=<>"] {
        assert!(t.iter().any(|x| x.contains(want)), "missing {want} in {t:?}");
    }
    // The six Ø8 holes: one callout with the count.
    assert!(t.iter().any(|x| x.contains("6-8DRILL")), "{t:?}");
    // Every position is tied down.
    assert_eq!(r["check"]["issues"].as_array().unwrap().len(), 0, "{}", r["check"]);
    // Overall sizes outermost: the highest horizontal dimension spans the plate.
    let d = s.doc().unwrap();
    let mut top: Option<(f64, f64)> = None;
    for h in &handles {
        let Some(e) = h.as_str().and_then(cadcraft_engine::doc::Handle::parse_hex).and_then(|h| d.entity(h)) else { continue };
        if let EntityKind::Dimension(dm) = &e.kind
            && matches!(dm.kind, cadcraft_engine::doc::DimKind::Linear { rotation } if rotation.abs() < 1e-9)
        {
            let span = (dm.p14.x - dm.p13.x).abs();
            if top.is_none_or(|(y, _)| dm.defpt.y > y) {
                top = Some((dm.defpt.y, span));
            }
        }
    }
    assert!((top.unwrap().1 - 350.0).abs() < 1e-6, "{top:?}");
}

#[test]
fn dimcheck_finds_a_position_no_dimension_gives() {
    let mut s = Session::new();
    let hs = plate(&mut s);
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let mut all: Vec<Value> = hs.iter().map(|h| json!(h)).collect();
    all.extend(r["handles"].as_array().unwrap().iter().cloned());
    let clean = s.execute("dimcheck", &json!({ "handles": all, "mark": false, "quiet": true })).unwrap();
    assert_eq!(clean["issues"].as_array().unwrap().len(), 0, "{clean}");
    // Without the dimensions there is plenty missing, and the findings are circled.
    let only: Vec<Value> = hs.iter().map(|h| json!(h)).collect();
    let r = s.execute("dimcheck", &json!({ "handles": only })).unwrap();
    assert!(r["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "missing"), "{r}");
    assert!(r["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "size"), "holes without a size: {r}");
    assert!(!r["marks"].as_array().unwrap().is_empty());
    assert!(s.doc().unwrap().layer("DIMCHECK").is_some());
}

#[test]
fn autodim_at_the_command_line() {
    let mut s = Session::new();
    let hs = plate(&mut s);
    let n0 = s.doc().unwrap().model.len();
    s.set_selection(hs.iter().filter_map(|h| cadcraft_engine::doc::Handle::parse_hex(h)).collect());
    s.cmdline("AUTODIM").unwrap();
    s.cmdline("12").unwrap();
    assert!(s.running.is_none());
    assert!(s.doc().unwrap().model.len() > n0 + 10);
    // Nothing selected: the whole drawing is checked.
    s.set_selection(vec![]);
    s.cmdline("DIMCHECK").unwrap();
    s.input(cadcraft_engine::Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert!(s.log.iter().any(|l| l.contains("DIMCHECK")), "{:?}", s.log);
}

#[test]
fn hostile_autodim_params_never_panic() {
    let mut s = Session::new();
    let hs = plate(&mut s);
    for p in [
        json!({}),
        json!({ "handles": [] }),
        json!({ "handles": hs, "pitchMin": -5, "thickness": "x" }),
        json!({ "handles": hs, "style": "no such style" }),
        json!({ "handles": ["zz"] }),
    ] {
        let r = s.execute("autodim", &p);
        assert!(!matches!(r, Err(cadcraft_engine::EngineError::Internal(..))), "{p}: {r:?}");
    }
    let one = add(&mut s, "line", json!({ "points": [[0, 0], [1e300, 1e-300]] }));
    let r = s.execute("autodim", &json!({ "handles": [one] }));
    assert!(!matches!(r, Err(cadcraft_engine::EngineError::Internal(..))));
    let r = s.execute("dimcheck", &json!({ "handles": [] }));
    assert!(!matches!(r, Err(cadcraft_engine::EngineError::Internal(..))));
}

#[test]
fn weld_beads_take_no_dimensions() {
    let mut s = Session::new();
    // A 100 × 50 plate with a 6 mm fillet weld drawn at its top-left corner: a filled
    // triangle with outline lines round it.
    let mut hs = vec![add(&mut s, "rectang", json!({ "p1": [0, 0], "p2": [100, 50] }))];
    let tri = add(&mut s, "pline", json!({ "vertices": [{"p": [0, 50]}, {"p": [6, 50]}, {"p": [0, 56]}], "closed": true }));
    hs.push(tri.clone());
    hs.push(add(&mut s, "hatch", json!({ "handles": [tri], "pattern": "SOLID" })));
    hs.push(add(&mut s, "line", json!({ "points": [[6, 50], [0, 56]] })));
    let r = s.execute("autodim", &json!({ "handles": hs, "callouts": false })).unwrap();
    let d = s.doc().unwrap();
    let mut spans = Vec::new();
    for h in r["handles"].as_array().unwrap() {
        let Some(e) = h.as_str().and_then(cadcraft_engine::doc::Handle::parse_hex).and_then(|h| d.entity(h)) else { continue };
        if let EntityKind::Dimension(dm) = &e.kind {
            spans.push((dm.p14.xy() - dm.p13.xy()).len());
        }
    }
    spans.sort_by(f64::total_cmp);
    // Just the plate: 100 and 50, nothing for the weld's 6.
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert!(spans.iter().all(|v| (*v - 6.0).abs() > 1e-6), "{spans:?}");
}

#[test]
fn autodim_takes_only_what_lies_inside_the_rectangle() {
    let mut s = Session::new();
    let hs = plate(&mut s);
    // A shaft running through the plate and out past both sides.
    add(&mut s, "line", json!({ "points": [[-100, 120], [450, 120]] }));
    let r = s.execute("autodim", &json!({ "window": [[-5, -5], [355, 205]] })).unwrap();
    assert_eq!(r["check"]["issues"].as_array().unwrap().len(), 0, "{}", r["check"]);
    let d = s.doc().unwrap();
    for h in r["handles"].as_array().unwrap() {
        let Some(e) = h.as_str().and_then(cadcraft_engine::doc::Handle::parse_hex).and_then(|h| d.entity(h)) else { continue };
        if let EntityKind::Dimension(dm) = &e.kind {
            // Nothing measured to the shaft's ends.
            assert!(dm.p13.x > -1.0 && dm.p14.x < 351.0, "{:?} {:?}", dm.p13, dm.p14);
        }
    }
    let _ = hs;
    // At the command line: two corners, then a click moving the nearest corner, then Enter.
    let n0 = s.doc().unwrap().model.len();
    s.set_selection(vec![]);
    s.cmdline("AUTODIM").unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(-5.0, -5.0))).unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(200.0, 205.0))).unwrap();
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(356.0, 210.0))).unwrap();
    s.cmdline("12").unwrap();
    s.input(cadcraft_engine::Input::Enter).unwrap();
    assert!(s.running.is_none());
    assert!(s.doc().unwrap().model.len() > n0 + 10);
}

#[test]
fn autodim_puts_dimensions_on_the_dim_layer() {
    let mut s = Session::new();
    s.execute("layer.new", &json!({ "name": "2-DIM", "color": 2, "current": false })).unwrap();
    let hs = plate(&mut s);
    let before = s.doc().unwrap().header.str("CLAYER", "0");
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let d = s.doc().unwrap();
    let mut dims = 0;
    for h in r["handles"].as_array().unwrap() {
        let e = d.entity(cadcraft_engine::doc::Handle::parse_hex(h.as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(e.common.layer, "2-DIM");
        dims += usize::from(matches!(e.kind, EntityKind::Dimension(_)));
    }
    assert!(dims > 5);
    assert_eq!(d.header.str("CLAYER", "0"), before);
}
