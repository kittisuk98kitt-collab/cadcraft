//! AUTODIM on machine parts as a shop drawing dimensions them: turned parts take diameters,
//! slots their ends, sprockets no tooth-by-tooth dimensions, bent plates a row on each side.

use cadcraft_engine::Session;
use cadcraft_engine::doc::{DimKind, EntityKind, Handle};
use serde_json::{Value, json};

fn add(s: &mut Session, id: &str, p: Value) -> String {
    let r = s.execute(id, &p).unwrap();
    r.get("handle")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| r.get("handles").and_then(|h| h.get(0)).and_then(Value::as_str).map(str::to_string))
        .unwrap()
}

fn centre_layer(s: &mut Session) {
    s.execute("linetype", &json!({ "load": "CENTER" })).unwrap();
    s.execute("layer.new", &json!({ "name": "CL", "linetype": "CENTER" })).unwrap();
}

fn centre_line(s: &mut Session, a: [f64; 2], b: [f64; 2]) -> String {
    let h = add(s, "line", json!({ "points": [a, b] }));
    s.execute("chprop", &json!({ "handles": [h], "layer": "CL" })).unwrap();
    h
}

/// (kind, measured length between the definition points, text, dimension line point).
struct D {
    kind: DimKind,
    span: f64,
    text: String,
    at: cadcraft_engine::geom::Vec2,
    p13: cadcraft_engine::geom::Vec2,
    p14: cadcraft_engine::geom::Vec2,
}

fn dims(s: &Session, r: &Value) -> Vec<D> {
    let d = s.doc().unwrap();
    let mut out = Vec::new();
    for h in r["handles"].as_array().unwrap() {
        let Some(e) = h.as_str().and_then(Handle::parse_hex).and_then(|h| d.entity(h)) else { continue };
        if let EntityKind::Dimension(dm) = &e.kind {
            let span = match dm.kind {
                DimKind::Linear { rotation } => {
                    let v = dm.p14.xy() - dm.p13.xy();
                    (v.x * rotation.cos() + v.y * rotation.sin()).abs()
                }
                _ => (dm.p15.xy() - dm.defpt.xy()).len(),
            };
            out.push(D { kind: dm.kind, span, text: dm.text.clone(), at: dm.defpt.xy(), p13: dm.p13.xy(), p14: dm.p14.xy() });
        }
    }
    out
}

fn has_span(ds: &[D], v: f64) -> bool {
    ds.iter().any(|d| matches!(d.kind, DimKind::Linear { .. }) && (d.span - v).abs() < 1e-6)
}

#[test]
fn a_slot_is_positioned_by_its_ends() {
    let mut s = Session::new();
    let mut hs = vec![add(&mut s, "rectang", json!({ "p1": [0, 0], "p2": [300, 100] }))];
    // A 20 wide slot from x 90 to 190 (end centres 100 and 180).
    hs.push(add(
        &mut s,
        "pline",
        json!({ "vertices": [
            {"p": [100, 40]}, {"p": [180, 40], "bulge": 1.0}, {"p": [180, 60]}, {"p": [100, 60], "bulge": 1.0}
        ], "closed": true }),
    ));
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &r);
    assert!(has_span(&ds, 90.0) && has_span(&ds, 100.0) && has_span(&ds, 110.0), "ends 90 / 100 / 110");
    assert!(!has_span(&ds, 80.0), "not the end centres");
}

#[test]
fn a_turned_shaft_takes_diameters_across_its_axis() {
    let mut s = Session::new();
    centre_layer(&mut s);
    // Ø140 × 580 with 3 mm chamfers at both ends (cone edges drawn at x 3 and 577).
    let mut hs = vec![add(
        &mut s,
        "pline",
        json!({ "vertices": [
            {"p": [3, 70]}, {"p": [577, 70]}, {"p": [580, 67]}, {"p": [580, -67]},
            {"p": [577, -70]}, {"p": [3, -70]}, {"p": [0, -67]}, {"p": [0, 67]}
        ], "closed": true }),
    )];
    hs.push(add(&mut s, "line", json!({ "points": [[3, 70], [3, -70]] })));
    hs.push(add(&mut s, "line", json!({ "points": [[577, 70], [577, -70]] })));
    hs.push(centre_line(&mut s, [-15.0, 0.0], [595.0, 0.0]));
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &r);
    assert!(ds.iter().any(|d| d.text.contains("%%c") && (d.span - 140.0).abs() < 1e-6), "Ø140 across the axis");
    assert!(has_span(&ds, 580.0));
    // No half chains 70/70 from the axis, no chamfer steps 3/574/3.
    assert!(!has_span(&ds, 70.0), "half diameters");
    assert!(!has_span(&ds, 3.0) && !has_span(&ds, 574.0), "chamfer steps");
}

#[test]
fn a_lone_end_view_circle_takes_only_its_diameter() {
    let mut s = Session::new();
    centre_layer(&mut s);
    let hs = vec![
        add(&mut s, "circle", json!({ "center": [0, 0], "radius": 70 })),
        add(&mut s, "circle", json!({ "center": [0, 0], "radius": 67 })),
        centre_line(&mut s, [-85.0, 0.0], [85.0, 0.0]),
        centre_line(&mut s, [0.0, -85.0], [0.0, 85.0]),
    ];
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &r);
    assert_eq!(ds.len(), 1, "one diameter");
    assert!(matches!(ds.first().map(|d| d.kind), Some(DimKind::Diameter)));
}

#[test]
fn a_sprocket_takes_its_tip_diameter_bore_and_key_not_its_teeth() {
    let mut s = Session::new();
    let mut hs = Vec::new();
    let n = 16;
    let (pr, rs) = (195.0_f64, 24.0_f64);
    let tip = 214.0_f64;
    let pt = |r: f64, a: f64| [r * a.cos(), r * a.sin()];
    for i in 0..n {
        let g = std::f64::consts::TAU * i as f64 / n as f64;
        let c = pt(pr, g);
        let inward = g + std::f64::consts::PI;
        let (a0, a1) = (inward - 75f64.to_radians(), inward + 75f64.to_radians());
        hs.push(add(&mut s, "arc", json!({ "center": c, "radius": rs, "start": a0.to_degrees(), "end": a1.to_degrees() })));
        let e = [c[0] + rs * a1.cos(), c[1] + rs * a1.sin()];
        let t = g + std::f64::consts::PI / n as f64;
        let (t1, t2) = (pt(tip, t - 0.035), pt(tip, t + 0.035));
        let g2 = std::f64::consts::TAU * (i + 1) as f64 / n as f64;
        let c2 = pt(pr, g2);
        let b0 = g2 + std::f64::consts::PI - 75f64.to_radians();
        let next = [c2[0] + rs * b0.cos(), c2[1] + rs * b0.sin()];
        hs.push(add(&mut s, "line", json!({ "points": [e, t1] })));
        hs.push(add(&mut s, "line", json!({ "points": [t1, t2] })));
        hs.push(add(&mut s, "line", json!({ "points": [t2, next] })));
    }
    hs.push(add(&mut s, "circle", json!({ "center": [0, 0], "radius": 150 })));
    // Bore Ø165 with a 40 wide keyway on the left, 174.2 from the far side of the bore.
    let (r, kw) = (82.5_f64, 40.0_f64);
    let ex = (r * r - (kw / 2.0) * (kw / 2.0)).sqrt();
    let kx = -(174.2 - r);
    hs.push(add(&mut s, "line", json!({ "points": [[-ex, 20], [kx, 20]] })));
    hs.push(add(&mut s, "line", json!({ "points": [[kx, 20], [kx, -20]] })));
    hs.push(add(&mut s, "line", json!({ "points": [[kx, -20], [-ex, -20]] })));
    let a = (20.0 / r).asin().to_degrees();
    hs.push(add(&mut s, "arc", json!({ "center": [0, 0], "radius": r, "start": 180.0 + a, "end": 180.0 - a })));
    let res = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &res);
    assert!(ds.len() <= 8, "{} dimensions", ds.len());
    let diam: Vec<f64> = ds.iter().filter(|d| matches!(d.kind, DimKind::Diameter)).map(|d| d.span).collect();
    assert!(diam.iter().any(|v| (v - 165.0).abs() < 1e-6), "bore {diam:?}");
    assert!(diam.iter().any(|v| (v - 300.0).abs() < 1e-6), "hub {diam:?}");
    assert!(diam.iter().any(|v| *v > 420.0), "tip {diam:?}");
    assert!(has_span(&ds, 40.0) && has_span(&ds, 174.2), "keyway");
}

#[test]
fn a_bent_plate_takes_its_lower_edge_from_below() {
    let mut s = Session::new();
    let hs = vec![add(
        &mut s,
        "pline",
        json!({ "vertices": [
            {"p": [0, 456]}, {"p": [190, 456]}, {"p": [395, 192]}, {"p": [613, 192]}, {"p": [843, 75]}, {"p": [990, 75]},
            {"p": [990, 0]}, {"p": [825, 0]}, {"p": [595, 117]}, {"p": [358.3, 117]}, {"p": [153.3, 381]}, {"p": [0, 381]}
        ], "closed": true }),
    )];
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &r);
    let horiz = |d: &&D| matches!(d.kind, DimKind::Linear { rotation } if rotation.abs() < 1e-9);
    // Above: the upper edge's corners 190 / 205 / 218 / 230 / 147.
    let above: Vec<&D> = ds.iter().filter(horiz).filter(|d| d.at.y > 456.0).collect();
    for v in [190.0, 205.0, 218.0, 230.0, 147.0] {
        assert!(above.iter().any(|d| (d.span - v).abs() < 1e-6), "above: {v}");
    }
    assert!(!above.iter().any(|d| (d.span - 153.3).abs() < 1e-6), "the lower edge's corner is not above");
    // Below: the lower edge's corners, measured from the lower edge.
    let below: Vec<&D> = ds.iter().filter(horiz).filter(|d| d.at.y < 0.0).collect();
    assert!(below.iter().any(|d| (d.span - 153.3).abs() < 1e-6), "below: 153.3");
    assert!(below.iter().all(|d| d.p13.y <= 381.0 + 1e-6 && d.p14.y <= 381.0 + 1e-6));
}

#[test]
fn holes_of_two_sizes_in_one_row_take_one_chain() {
    let mut s = Session::new();
    let mut hs = vec![add(&mut s, "rectang", json!({ "p1": [0, 0], "p2": [990, 150] }))];
    hs.push(add(&mut s, "circle", json!({ "center": [65, 75], "radius": 25 })));
    hs.push(add(&mut s, "circle", json!({ "center": [495, 75], "radius": 30 })));
    hs.push(add(&mut s, "circle", json!({ "center": [925, 75], "radius": 25 })));
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &r);
    assert!(has_span(&ds, 430.0), "65 / 430 / 430 / 65");
    assert!(!has_span(&ds, 860.0) && !has_span(&ds, 495.0), "not one row per size");
}

#[test]
fn a_shaft_diameter_keeps_clear_of_its_keyway() {
    let mut s = Session::new();
    centre_layer(&mut s);
    let mut hs = vec![add(&mut s, "rectang", json!({ "p1": [0, -70], "p2": [580, 70] }))];
    // A 36 wide keyway from x 151 to 429 on the axis.
    hs.push(add(&mut s, "line", json!({ "points": [[169, 18], [411, 18]] })));
    hs.push(add(&mut s, "line", json!({ "points": [[169, -18], [411, -18]] })));
    hs.push(add(&mut s, "arc", json!({ "center": [411, 0], "radius": 18, "start": -90, "end": 90 })));
    hs.push(add(&mut s, "arc", json!({ "center": [169, 0], "radius": 18, "start": 90, "end": 270 })));
    hs.push(centre_line(&mut s, [-15.0, 0.0], [595.0, 0.0]));
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    let ds = dims(&s, &r);
    let d = ds.iter().find(|d| d.text.contains("%%c")).expect("a diameter");
    assert!((d.span - 140.0).abs() < 1e-6);
    assert!(d.at.x < 151.0 - 1.0 || d.at.x > 429.0 + 1.0, "Ø line at x {} crosses the keyway", d.at.x);
    // The keyway is positioned by its ends: 151 / 278 / 151.
    assert!(has_span(&ds, 151.0) && has_span(&ds, 278.0));
}

#[test]
fn a_layout_takes_its_sections_and_overall_length_only() {
    let mut s = Session::new();
    let mut hs = Vec::new();
    // A line of machines: bed sections 6440 + 6425, a 610 guard, 7490 + 5965, a 2500 drive,
    // with 40 posts and 10 stands along it.
    for (a, b) in [(0.0, 6440.0), (6440.0, 12865.0), (13475.0, 20965.0), (20965.0, 26930.0)] {
        hs.push(add(&mut s, "rectang", json!({ "p1": [a, -270], "p2": [b, 270] })));
    }
    hs.push(add(&mut s, "rectang", json!({ "p1": [12865, -400], "p2": [13475, 400] })));
    hs.push(add(&mut s, "rectang", json!({ "p1": [26930, -890], "p2": [29430, 700] })));
    for k in 0..20 {
        let x = 400.0 + 640.0 * k as f64;
        hs.push(add(&mut s, "rectang", json!({ "p1": [x, 270], "p2": [x + 150.0, 420] })));
        hs.push(add(&mut s, "rectang", json!({ "p1": [x, -420], "p2": [x + 150.0, -270] })));
    }
    for k in 0..10 {
        let x = 14170.0 + 1255.0 * k as f64;
        hs.push(add(&mut s, "rectang", json!({ "p1": [x - 30.0, -610], "p2": [x + 30.0, 1570] })));
    }
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false, "callouts": false })).unwrap();
    let ds = dims(&s, &r);
    let mut spans: Vec<f64> = ds.iter().map(|d| (d.span * 10.0).round() / 10.0).collect();
    spans.sort_by(f64::total_cmp);
    assert_eq!(spans, vec![2500.0, 5965.0, 6425.0, 6440.0, 8100.0, 29430.0], "{spans:?}");
}

#[test]
fn a_leader_follows_a_dimscale_set_for_the_drawing() {
    let mut s = Session::new();
    let st = s.doc().unwrap().header.str("DIMSTYLE", "Standard");
    let base = s.doc().unwrap().dim_style(&st).map(|d| d.text_height).unwrap();
    s.cmdline("DIMSCALE 7").unwrap();
    let r = s.execute("leader", &json!({ "points": [[0, 0], [10, 10]], "text": "C5" })).unwrap();
    let d = s.doc().unwrap();
    let hs: Vec<String> = r["handles"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let h = hs
        .iter()
        .filter_map(|h| Handle::parse_hex(h).and_then(|h| d.entity(h)))
        .find_map(|e| match &e.kind {
            EntityKind::MText(m) => Some(m.height),
            EntityKind::Text(t) => Some(t.height),
            _ => None,
        })
        .expect("leader text");
    assert!((h - base * 7.0).abs() < 1e-6, "text height {h}, style {base}");
}

fn style_of(s: &Session, r: &Value) -> Vec<String> {
    let d = s.doc().unwrap();
    r["handles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h.as_str().and_then(Handle::parse_hex).and_then(|h| d.entity(h)))
        .filter_map(|e| match &e.kind {
            EntityKind::Dimension(dm) => Some(dm.style.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn autodim_uses_the_current_style_and_spaces_rows_by_its_scale() {
    let mut s = Session::new();
    s.execute("dimstyle", &json!({ "name": "S5", "scale": 5, "textHeight": 2.5, "current": false })).unwrap();
    s.execute("dimstyle", &json!({ "name": "S2", "scale": 2, "textHeight": 2.5, "current": false })).unwrap();
    // The sheet already has dimensions in S5 (1:5)...
    s.execute("dimstyle.current", &json!({ "name": "S5" })).unwrap();
    for k in 0..3 {
        s.execute("dimlinear", &json!({ "p1": [1000 + k * 10, 0], "p2": [1005 + k * 10, 0], "at": [1000, 20] })).unwrap();
    }
    // ...and the user makes S2 current for a 1:2 detail.
    s.execute("dimstyle.current", &json!({ "name": "S2" })).unwrap();
    let hs = vec![add(&mut s, "rectang", json!({ "p1": [0, 0], "p2": [100, 50] }))];
    let r = s.execute("autodim", &json!({ "handles": hs, "check": false })).unwrap();
    assert!(style_of(&s, &r).iter().all(|n| n == "S2"), "{:?}", style_of(&s, &r));
    assert_eq!(s.doc().unwrap().header.str("DIMSTYLE", ""), "S2");
    // First row 3 text heights (2.5 × 2) above the part: 15, not the 1:5 37.5.
    let ds = dims(&s, &r);
    let top =
        ds.iter().filter(|d| matches!(d.kind, DimKind::Linear { rotation } if rotation.abs() < 1e-9)).map(|d| d.at.y).fold(f64::INFINITY, f64::min);
    assert!((top - 65.0).abs() < 1e-6, "first row at {top}");
    // An asked-for style is made current with its own DIMSCALE (no stale override).
    let hs2 = vec![add(&mut s, "rectang", json!({ "p1": [500, 0], "p2": [600, 50] }))];
    let r2 = s.execute("autodim", &json!({ "handles": hs2, "check": false, "style": "S5" })).unwrap();
    let d = s.doc().unwrap();
    for h in r2["handles"].as_array().unwrap() {
        if let Some(EntityKind::Dimension(dm)) = h.as_str().and_then(Handle::parse_hex).and_then(|h| d.entity(h)).map(|e| e.kind.clone()) {
            assert_eq!(dm.style, "S5");
            assert!(dm.overrides.get("scale").is_none(), "stale DIMSCALE override {:?}", dm.overrides);
        }
    }
}

fn note_height(s: &Session) -> f64 {
    let d = s.doc().unwrap();
    d.model
        .iter()
        .find_map(|e| match &e.kind {
            EntityKind::MText(m) => Some(m.height),
            EntityKind::Text(t) => Some(t.height),
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_leader_note_follows_its_dimension_style() {
    let mut s = Session::new();
    s.execute("dimstyle", &json!({ "name": "S2", "scale": 2, "textHeight": 2.5 })).unwrap();
    let r = s.execute("leader", &json!({ "points": [[0, 0], [20, 20], [30, 20]], "text": "C5" })).unwrap();
    let leader = r["handles"].as_array().unwrap().iter().filter_map(|h| h.as_str()).find(|h| {
        let d = s.doc().unwrap();
        Handle::parse_hex(h).and_then(|h| d.entity(h)).is_some_and(|e| matches!(e.kind, EntityKind::Leader(_)))
    });
    let leader = leader.unwrap().to_string();
    assert!((note_height(&s) - 5.0).abs() < 1e-9);
    // Properties: another style resizes the note.
    s.execute("dimstyle", &json!({ "name": "S5", "scale": 5, "textHeight": 2.5, "current": false })).unwrap();
    s.execute("properties.set", &json!({ "handles": [leader.clone()], "dimStyle": "S5" })).unwrap();
    assert!((note_height(&s) - 12.5).abs() < 1e-9, "{}", note_height(&s));
    // Editing the style's scale resizes it again.
    s.execute("dimstyle", &json!({ "name": "S5", "scale": 10, "current": false })).unwrap();
    assert!((note_height(&s) - 25.0).abs() < 1e-9, "{}", note_height(&s));
    // DIMSTYLE Apply with S2 current.
    s.execute("dimstyle.current", &json!({ "name": "S2" })).unwrap();
    s.execute("dimstyle.update", &json!({ "handles": [leader] })).unwrap();
    assert!((note_height(&s) - 5.0).abs() < 1e-9, "{}", note_height(&s));
}

#[test]
fn autodim_window_corners_are_not_snapped() {
    let mut s = Session::new();
    add(&mut s, "rectang", json!({ "p1": [0, 0], "p2": [100, 50] }));
    s.set_selection(vec![]);
    s.cmdline("AUTODIM").unwrap();
    assert!(s.current_prompt().unwrap().nosnap);
    s.input(cadcraft_engine::Input::Point(cadcraft_engine::geom::Vec2::new(-5.0, -5.0))).unwrap();
    let p = s.current_prompt().unwrap();
    assert!(p.nosnap && p.corner);
}
