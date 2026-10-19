//! STEEL: structural steel members from the JIS tables, placed as a weldment tool places them:
//! channels, equal and unequal angles and H sections (JIS G 3192), square and rectangular
//! tubes (JIS G 3466) and flat bars. A member goes in as its section (the profile with its
//! root and toe radii) at a point, or along two points as its side or top view with the hidden
//! edges of its flanges and walls. Each is a block reference whose block keeps its
//! specification, so STEELEDIT can swap a member for another size in place.

use cadcraft_doc::{EntityKind, Handle};
use cadcraft_geom::{Mat3, PolyVertex, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::helpers::{line, lwpoly};
use super::*;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// A section: kind, designation, depth H, width B, web or wall t1, flange t2, root radius r1,
/// toe (or outer corner) radius r2.
#[derive(Clone, Copy, Debug)]
pub struct Section {
    pub kind: &'static str,
    pub h: f64,
    pub b: f64,
    pub t1: f64,
    pub t2: f64,
    pub r1: f64,
    pub r2: f64,
}

const fn c(h: f64, b: f64, t1: f64, t2: f64, r1: f64, r2: f64) -> Section {
    Section { kind: "C", h, b, t1, t2, r1, r2 }
}
const fn l(a: f64, b: f64, t: f64, r1: f64, r2: f64) -> Section {
    Section { kind: "L", h: a, b, t1: t, t2: t, r1, r2 }
}
const fn hb(h: f64, b: f64, t1: f64, t2: f64, r: f64) -> Section {
    Section { kind: "H", h, b, t1, t2, r1: r, r2: 0.0 }
}
const fn tube(h: f64, b: f64, t: f64) -> Section {
    Section { kind: "TUBE", h, b, t1: t, t2: t, r1: t, r2: 2.0 * t }
}

/// JIS G 3192 channels H×B×t1×t2 (r1, r2).
pub const CHANNELS: &[Section] = &[
    c(75.0, 40.0, 5.0, 7.0, 8.0, 4.0),
    c(100.0, 50.0, 5.0, 7.5, 8.0, 4.0),
    c(125.0, 65.0, 6.0, 8.0, 8.0, 4.0),
    c(150.0, 75.0, 6.5, 10.0, 10.0, 5.0),
    c(150.0, 75.0, 9.0, 12.5, 15.0, 7.5),
    c(180.0, 75.0, 7.0, 10.5, 11.0, 5.5),
    c(200.0, 70.0, 7.0, 10.0, 11.0, 5.5),
    c(200.0, 80.0, 7.5, 11.0, 12.0, 6.0),
    c(200.0, 90.0, 8.0, 13.5, 14.0, 7.0),
    c(250.0, 90.0, 9.0, 13.0, 14.0, 7.0),
    c(250.0, 90.0, 11.0, 14.5, 17.0, 8.5),
    c(300.0, 90.0, 9.0, 13.0, 14.0, 7.0),
    c(300.0, 90.0, 10.0, 15.5, 19.0, 9.5),
    c(300.0, 90.0, 12.0, 16.0, 19.0, 9.5),
    c(380.0, 100.0, 10.5, 16.0, 18.0, 9.0),
    c(380.0, 100.0, 13.0, 16.5, 18.0, 9.0),
    c(380.0, 100.0, 13.0, 20.0, 24.0, 12.0),
];

/// JIS G 3192 equal and unequal angles A×B×t (r1, r2).
pub const ANGLES: &[Section] = &[
    l(25.0, 25.0, 3.0, 4.0, 2.0),
    l(30.0, 30.0, 3.0, 4.0, 2.0),
    l(40.0, 40.0, 3.0, 4.5, 2.0),
    l(40.0, 40.0, 5.0, 4.5, 3.0),
    l(45.0, 45.0, 4.0, 6.5, 3.0),
    l(45.0, 45.0, 5.0, 6.5, 3.0),
    l(50.0, 50.0, 4.0, 6.5, 3.0),
    l(50.0, 50.0, 5.0, 6.5, 3.0),
    l(50.0, 50.0, 6.0, 6.5, 4.5),
    l(60.0, 60.0, 4.0, 6.5, 3.0),
    l(60.0, 60.0, 5.0, 6.5, 3.0),
    l(65.0, 65.0, 5.0, 8.5, 3.0),
    l(65.0, 65.0, 6.0, 8.5, 4.0),
    l(65.0, 65.0, 8.0, 8.5, 6.0),
    l(70.0, 70.0, 6.0, 8.5, 4.0),
    l(75.0, 75.0, 6.0, 8.5, 4.0),
    l(75.0, 75.0, 9.0, 8.5, 6.0),
    l(75.0, 75.0, 12.0, 8.5, 6.0),
    l(80.0, 80.0, 6.0, 8.5, 4.0),
    l(90.0, 90.0, 6.0, 10.0, 5.0),
    l(90.0, 90.0, 7.0, 10.0, 5.0),
    l(90.0, 90.0, 10.0, 10.0, 7.0),
    l(90.0, 90.0, 13.0, 10.0, 7.0),
    l(100.0, 100.0, 7.0, 10.0, 5.0),
    l(100.0, 100.0, 10.0, 10.0, 7.0),
    l(100.0, 100.0, 13.0, 10.0, 7.0),
    l(120.0, 120.0, 8.0, 12.0, 5.0),
    l(130.0, 130.0, 9.0, 12.0, 6.0),
    l(130.0, 130.0, 12.0, 12.0, 8.5),
    l(130.0, 130.0, 15.0, 12.0, 8.5),
    l(150.0, 150.0, 12.0, 14.0, 7.0),
    l(150.0, 150.0, 15.0, 14.0, 10.0),
    l(150.0, 150.0, 19.0, 14.0, 10.0),
    l(175.0, 175.0, 12.0, 15.0, 11.0),
    l(175.0, 175.0, 15.0, 15.0, 11.0),
    l(200.0, 200.0, 15.0, 17.0, 12.0),
    l(200.0, 200.0, 20.0, 17.0, 12.0),
    l(200.0, 200.0, 25.0, 17.0, 12.0),
    l(250.0, 250.0, 25.0, 24.0, 12.0),
    l(250.0, 250.0, 35.0, 24.0, 18.0),
    l(90.0, 75.0, 9.0, 8.5, 6.0),
    l(100.0, 75.0, 7.0, 10.0, 5.0),
    l(100.0, 75.0, 10.0, 10.0, 7.0),
    l(125.0, 75.0, 7.0, 10.0, 5.0),
    l(125.0, 75.0, 10.0, 10.0, 7.0),
    l(125.0, 75.0, 13.0, 10.0, 7.0),
    l(125.0, 90.0, 10.0, 10.0, 7.0),
    l(125.0, 90.0, 13.0, 10.0, 7.0),
    l(150.0, 90.0, 9.0, 12.0, 6.0),
    l(150.0, 90.0, 12.0, 12.0, 8.5),
    l(150.0, 100.0, 9.0, 12.0, 6.0),
    l(150.0, 100.0, 12.0, 12.0, 8.5),
    l(150.0, 100.0, 15.0, 12.0, 8.5),
];

/// JIS G 3192 H sections H×B×t1×t2 (r): wide, medium and narrow flange.
pub const H_SECTIONS: &[Section] = &[
    hb(100.0, 100.0, 6.0, 8.0, 8.0),
    hb(125.0, 125.0, 6.5, 9.0, 8.0),
    hb(150.0, 150.0, 7.0, 10.0, 8.0),
    hb(175.0, 175.0, 7.5, 11.0, 13.0),
    hb(200.0, 200.0, 8.0, 12.0, 13.0),
    hb(250.0, 250.0, 9.0, 14.0, 13.0),
    hb(300.0, 300.0, 10.0, 15.0, 13.0),
    hb(350.0, 350.0, 12.0, 19.0, 13.0),
    hb(400.0, 400.0, 13.0, 21.0, 22.0),
    hb(148.0, 100.0, 6.0, 9.0, 8.0),
    hb(194.0, 150.0, 6.0, 9.0, 8.0),
    hb(244.0, 175.0, 7.0, 11.0, 13.0),
    hb(294.0, 200.0, 8.0, 12.0, 13.0),
    hb(340.0, 250.0, 9.0, 14.0, 13.0),
    hb(390.0, 300.0, 10.0, 16.0, 13.0),
    hb(440.0, 300.0, 11.0, 18.0, 13.0),
    hb(488.0, 300.0, 11.0, 18.0, 13.0),
    hb(588.0, 300.0, 12.0, 20.0, 13.0),
    hb(150.0, 75.0, 5.0, 7.0, 8.0),
    hb(175.0, 90.0, 5.0, 8.0, 8.0),
    hb(200.0, 100.0, 5.5, 8.0, 8.0),
    hb(250.0, 125.0, 6.0, 9.0, 8.0),
    hb(300.0, 150.0, 6.5, 9.0, 13.0),
    hb(350.0, 175.0, 7.0, 11.0, 13.0),
    hb(400.0, 200.0, 8.0, 13.0, 13.0),
    hb(450.0, 200.0, 9.0, 14.0, 13.0),
    hb(500.0, 200.0, 10.0, 16.0, 13.0),
    hb(600.0, 200.0, 11.0, 17.0, 13.0),
];

/// JIS G 3466 square and rectangular tubes H×B×t (corners: outside 2t, inside t).
pub const TUBES: &[Section] = &[
    tube(40.0, 40.0, 1.6),
    tube(40.0, 40.0, 2.3),
    tube(50.0, 50.0, 1.6),
    tube(50.0, 50.0, 2.3),
    tube(50.0, 50.0, 3.2),
    tube(60.0, 60.0, 1.6),
    tube(60.0, 60.0, 2.3),
    tube(60.0, 60.0, 3.2),
    tube(75.0, 75.0, 2.3),
    tube(75.0, 75.0, 3.2),
    tube(75.0, 75.0, 4.5),
    tube(100.0, 100.0, 2.3),
    tube(100.0, 100.0, 3.2),
    tube(100.0, 100.0, 4.0),
    tube(100.0, 100.0, 4.5),
    tube(100.0, 100.0, 6.0),
    tube(125.0, 125.0, 3.2),
    tube(125.0, 125.0, 4.5),
    tube(125.0, 125.0, 6.0),
    tube(150.0, 150.0, 4.5),
    tube(150.0, 150.0, 6.0),
    tube(150.0, 150.0, 9.0),
    tube(175.0, 175.0, 4.5),
    tube(175.0, 175.0, 6.0),
    tube(200.0, 200.0, 6.0),
    tube(200.0, 200.0, 8.0),
    tube(200.0, 200.0, 9.0),
    tube(200.0, 200.0, 12.0),
    tube(250.0, 250.0, 6.0),
    tube(250.0, 250.0, 9.0),
    tube(250.0, 250.0, 12.0),
    tube(300.0, 300.0, 9.0),
    tube(300.0, 300.0, 12.0),
    tube(50.0, 25.0, 1.6),
    tube(50.0, 25.0, 2.3),
    tube(60.0, 30.0, 1.6),
    tube(60.0, 30.0, 2.3),
    tube(60.0, 30.0, 3.2),
    tube(75.0, 45.0, 2.3),
    tube(75.0, 45.0, 3.2),
    tube(100.0, 50.0, 2.3),
    tube(100.0, 50.0, 3.2),
    tube(100.0, 50.0, 4.5),
    tube(125.0, 75.0, 2.3),
    tube(125.0, 75.0, 3.2),
    tube(125.0, 75.0, 4.5),
    tube(125.0, 75.0, 6.0),
    tube(150.0, 50.0, 3.2),
    tube(150.0, 50.0, 4.5),
    tube(150.0, 100.0, 3.2),
    tube(150.0, 100.0, 4.5),
    tube(150.0, 100.0, 6.0),
    tube(150.0, 100.0, 9.0),
    tube(200.0, 100.0, 4.5),
    tube(200.0, 100.0, 6.0),
    tube(200.0, 100.0, 9.0),
    tube(250.0, 150.0, 6.0),
    tube(250.0, 150.0, 9.0),
    tube(300.0, 200.0, 6.0),
    tube(300.0, 200.0, 9.0),
    tube(300.0, 200.0, 12.0),
];

fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Section {
    /// The designation as a drawing calls it ("[-150x75x6.5x10", "L-50x50x6", "□-100x100x3.2").
    pub fn name(&self) -> String {
        match self.kind {
            "C" => format!("C{}x{}x{}x{}", num(self.h), num(self.b), num(self.t1), num(self.t2)),
            "L" => format!("L{}x{}x{}", num(self.h), num(self.b), num(self.t1)),
            "H" => format!("H{}x{}x{}x{}", num(self.h), num(self.b), num(self.t1), num(self.t2)),
            "FB" => format!("FB{}x{}", num(self.h), num(self.b)),
            _ => format!("TUBE{}x{}x{}", num(self.h), num(self.b), num(self.t1)),
        }
    }

    /// The outline(s) in section, origin at the bounding box's bottom left: closed polylines
    /// with the radii as bulges.
    fn profile(&self) -> Vec<Vec<PolyVertex>> {
        let (h, b, t1, t2, r1, r2) = (self.h, self.b, self.t1, self.t2, self.r1, self.r2);
        let p = Vec2::new;
        match self.kind {
            // Web on the left, flanges to the right. The flanges' inner faces slope 5° (JIS
            // channels), t2 being the thickness halfway along the flange's inner face.
            "C" => {
                let slope = 5f64.to_radians().tan();
                let half = (b - t1) / 2.0;
                let (root, toe) = (t2 + half * slope, (t2 - half * slope).max(t2 * 0.3));
                vec![rounded(&[
                    (p(0.0, 0.0), 0.0),
                    (p(b, 0.0), 0.0),
                    (p(b, toe), r2),
                    (p(t1, root), r1),
                    (p(t1, h - root), r1),
                    (p(b, h - toe), r2),
                    (p(b, h), 0.0),
                    (p(0.0, h), 0.0),
                ])]
            }
            // Vertical leg A (height), horizontal leg B.
            "L" => vec![rounded(&[(p(0.0, 0.0), 0.0), (p(b, 0.0), 0.0), (p(b, t1), r2), (p(t1, t1), r1), (p(t1, h), r2), (p(0.0, h), 0.0)])],
            "H" => {
                let (wl, wr) = ((b - t1) / 2.0, (b + t1) / 2.0);
                vec![rounded(&[
                    (p(0.0, 0.0), 0.0),
                    (p(b, 0.0), 0.0),
                    (p(b, t2), 0.0),
                    (p(wr, t2), r1),
                    (p(wr, h - t2), r1),
                    (p(b, h - t2), 0.0),
                    (p(b, h), 0.0),
                    (p(0.0, h), 0.0),
                    (p(0.0, h - t2), 0.0),
                    (p(wl, h - t2), r1),
                    (p(wl, t2), r1),
                    (p(0.0, t2), 0.0),
                ])]
            }
            "FB" => vec![rounded(&[(p(0.0, 0.0), 0.0), (p(b, 0.0), 0.0), (p(b, h), 0.0), (p(0.0, h), 0.0)])],
            _ => vec![
                rounded(&[(p(0.0, 0.0), r2), (p(b, 0.0), r2), (p(b, h), r2), (p(0.0, h), r2)]),
                rounded(&[(p(t1, t1), r1), (p(b - t1, t1), r1), (p(b - t1, h - t1), r1), (p(t1, h - t1), r1)]),
            ],
        }
    }

    /// Side view (`top` false: looking at the web or the tube's side, height H) or top view
    /// (height B): the member's height and its hidden edges, as offsets from the bottom edge.
    fn elevation(&self, top: bool) -> (f64, Vec<f64>) {
        let (h, b, t1, t2) = (self.h, self.b, self.t1, self.t2);
        match (self.kind, top) {
            ("C", false) | ("H", false) => (h, vec![t2, h - t2]),
            ("C", true) => (b, vec![t1]),
            ("H", true) => (b, vec![(b - t1) / 2.0, (b + t1) / 2.0]),
            ("L", false) => (h, vec![t1]),
            ("L", true) => (b, vec![t1]),
            ("FB", false) => (h, vec![]),
            ("FB", true) => (b, vec![]),
            (_, false) => (h, vec![t1, h - t1]),
            (_, true) => (b, vec![t1, b - t1]),
        }
    }
}

/// A closed polyline through `pts`, each corner rounded by its radius (0: sharp).
fn rounded(pts: &[(Vec2, f64)]) -> Vec<PolyVertex> {
    let n = pts.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (prev, (cur, r), next) = (pts[(i + n - 1) % n].0, pts[i], pts[(i + 1) % n].0);
        if r <= 0.0 {
            out.push(PolyVertex::new(cur));
            continue;
        }
        let (u, v) = ((cur - prev).normalized(), (next - cur).normalized());
        let turn = u.cross(v).atan2(u.dot(v));
        let t = (r * (turn.abs() / 2.0).tan()).min(cur.dist(prev) / 2.0).min(cur.dist(next) / 2.0);
        out.push(PolyVertex::with_bulge(cur - u * t, (turn / 4.0).tan()));
        out.push(PolyVertex::new(cur + v * t));
    }
    out
}

pub fn table(kind: &str) -> &'static [Section] {
    match kind {
        "C" => CHANNELS,
        "L" => ANGLES,
        "H" => H_SECTIONS,
        "TUBE" => TUBES,
        _ => &[],
    }
}

fn kind_of(k: &str) -> Option<&'static str> {
    match k.trim().to_ascii_uppercase().as_str() {
        "C" | "CHANNEL" | "[" => Some("C"),
        "L" | "ANGLE" => Some("L"),
        "H" | "HBEAM" | "H-BEAM" => Some("H"),
        "TUBE" | "SQUARE" | "RECT" | "RHS" | "SHS" | "□" => Some("TUBE"),
        "FB" | "FLAT" | "FLATBAR" => Some("FB"),
        _ => None,
    }
}

/// Find `size` ("150x75x6.5x10", "50x50x6", "100x100x3.2", "FB" "50x6") of `kind`.
pub fn find(kind: &str, size: &str) -> Option<Section> {
    let k = kind_of(kind)?;
    let nums: Vec<f64> = size
        .trim()
        .trim_start_matches(|c: char| c.is_alphabetic() || c == '[' || c == '□' || c == '-')
        .split(['x', 'X', '×', '*'])
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect();
    if k == "FB" {
        let (w, t) = (*nums.first()?, *nums.get(1)?);
        return (w > 0.0 && t > 0.0 && w < 5000.0 && t < 1000.0).then_some(Section { kind: "FB", h: t, b: w, t1: t, t2: t, r1: 0.0, r2: 0.0 });
    }
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    table(k)
        .iter()
        .find(|s| {
            let want: Vec<f64> = match k {
                "C" | "H" => vec![s.h, s.b, s.t1, s.t2],
                _ => vec![s.h, s.b, s.t1],
            };
            // The thickness may be left out when the size has only one.
            nums.len() <= want.len() && nums.len() >= 2 && nums.iter().zip(&want).all(|(a, b)| close(*a, *b))
        })
        .copied()
}

/// STEEL's settings (the Steel dialog).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SteelSettings {
    /// C, L, H, TUBE or FB.
    pub kind: String,
    pub size: String,
    /// section, side or top.
    pub view: String,
    /// Section: the profile's point placed at the pick: BL, BC, BR, ML, MC, MR, TL, TC, TR.
    /// Side/top: which edge runs along the picked line: B (bottom), M (centre), T (top).
    pub anchor: String,
    pub angle: f64,
    pub mirror: bool,
    pub hatch: bool,
}

impl Default for SteelSettings {
    fn default() -> Self {
        SteelSettings {
            kind: "C".into(),
            size: "100x50x5x7.5".into(),
            view: "section".into(),
            anchor: "MC".into(),
            angle: 0.0,
            mirror: false,
            hatch: false,
        }
    }
}

/// What a member's block keeps (its description).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    kind: String,
    size: String,
    view: String,
    anchor: String,
    mirror: bool,
    hatch: bool,
    length: f64,
}

const TAG: &str = "CADCRAFT_STEEL ";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("steel", "Structural Member", run_steel)
            .menu(&["Draw", "Structural Member..."])
            .alias(&["weldment", "member", "stl"])
            .params("{points: [[x,y]] (section: one point per member; side/top: start and end of each member in pairs), kind?: C|L|H|TUBE|FB, size?: \"150x75x6.5x10\", view?: section|side|top, anchor?, angle? (degrees, section), mirror?, hatch?}")
            .interactive(|_| Ok(Box::new(SteelM::default()))),
        CommandSpec::new("steeledit", "Edit Structural Member", run_steeledit)
            .menu(&["Modify", "Object", "Structural Member..."])
            .params("{handle (a member), kind?, size?, mirror?, hatch?, anchor?}")
            .interactive(|_| Ok(Box::new(SteelEditM))),
        CommandSpec::new("steel.settings", "Structural Member Settings", run_settings)
            .params("{kind?, size?, view?, anchor?, angle?, mirror?, hatch?} → the settings and the sizes of the kind")
            .noundo(),
        CommandSpec::new("steel.member", "Structural Member Info", run_member_info).params("{handle} → the member").noundo(),
    ]
}

fn merge<T: Serialize + for<'de> Deserialize<'de>>(cur: &T, p: &Value) -> Result<T> {
    let mut v = serde_json::to_value(cur).map_err(|e| bad("steel", e.to_string()))?;
    if let (Some(c), Some(o)) = (v.as_object_mut(), p.as_object()) {
        for (k, val) in o {
            if c.contains_key(k) {
                c.insert(k.clone(), val.clone());
            }
        }
    }
    serde_json::from_value(v).map_err(|e| bad("steel", e.to_string()))
}

fn normalize(st: &mut SteelSettings, size_given: bool) -> Result<Section> {
    let k = kind_of(&st.kind).ok_or_else(|| bad("steel", "`kind` is C, L, H, TUBE or FB"))?;
    st.kind = k.into();
    st.view = st.view.trim().to_ascii_lowercase();
    if !matches!(st.view.as_str(), "section" | "side" | "top") {
        return Err(bad("steel", "`view` is section, side or top"));
    }
    st.anchor = st.anchor.trim().to_ascii_uppercase();
    let sec = match find(k, &st.size) {
        Some(s) => s,
        None if size_given => return Err(bad("steel", format!("no {k} size \"{}\"", st.size))),
        None => match table(k).first().copied() {
            Some(s) => s,
            None => find("FB", "50x6").ok_or_else(|| bad("steel", "no size"))?,
        },
    };
    st.size = sec.name().trim_start_matches(|c: char| c.is_alphabetic()).to_string();
    if !st.angle.is_finite() {
        st.angle = 0.0;
    }
    Ok(sec)
}

fn sizes_json(kind: &str) -> Value {
    json!(table(kind).iter().map(|s| s.name().trim_start_matches(|c: char| c.is_alphabetic()).to_string()).collect::<Vec<_>>())
}

fn run_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let mut next: SteelSettings = merge(&s.steel, p)?;
    normalize(&mut next, p.get("size").is_some())?;
    s.steel = next;
    let mut v = serde_json::to_value(&s.steel).map_err(|e| bad("steel", e.to_string()))?;
    v["sizes"] = sizes_json(&s.steel.kind);
    Ok(v)
}

/// The anchor's offset into the bounding box (fractions of width and height).
fn anchor_frac(a: &str) -> (f64, f64) {
    let a = a.as_bytes();
    let row = |c: u8| match c {
        b'B' => 0.0,
        b'T' => 1.0,
        _ => 0.5,
    };
    let col = |c: u8| match c {
        b'L' => 0.0,
        b'R' => 1.0,
        _ => 0.5,
    };
    match a {
        [r, c, ..] => (col(*c), row(*r)),
        [r] => (0.5, row(*r)),
        [] => (0.5, 0.5),
    }
}

/// The member's geometry in its block (origin at the anchor): kind and whether hidden.
fn geometry(sec: &Section, m: &Member) -> Vec<(EntityKind, bool)> {
    let mut out = Vec::new();
    if m.view == "section" {
        let (fx, fy) = anchor_frac(&m.anchor);
        let o = Vec2::new(sec.b * fx, sec.h * fy);
        for ring in sec.profile() {
            let mut k = lwpoly(ring, true);
            // Mirrored: flipped across the profile's middle (x → B − x) before the anchor shift.
            let t = if m.mirror {
                Mat3::translate(-o).then_before(Mat3::translate(Vec2::new(sec.b, 0.0))).then_before(Mat3::scale(-1.0, 1.0))
            } else {
                Mat3::translate(-o)
            };
            k.transform(&t);
            out.push((k, false));
        }
        return out;
    }
    let (ht, hidden) = sec.elevation(m.view == "top");
    let (_, fy) = anchor_frac(&m.anchor);
    let y0 = -ht * fy;
    let len = m.length;
    let p = |x: f64, y: f64| Vec2::new(x, y0 + y);
    out.push((
        lwpoly(vec![PolyVertex::new(p(0.0, 0.0)), PolyVertex::new(p(len, 0.0)), PolyVertex::new(p(len, ht)), PolyVertex::new(p(0.0, ht))], true),
        false,
    ));
    for y in hidden {
        let y = if m.mirror { ht - y } else { y };
        out.push((line(p(0.0, y), p(len, y)), true));
    }
    out
}

/// The block for a member, made once per size, view and length.
fn ensure_block(s: &mut Session, sec: &Section, m: &Member) -> Result<String> {
    let mut name = format!("STEEL_{}_{}_{}{}", sec.name(), m.view.to_ascii_uppercase(), m.anchor, if m.mirror { "_M" } else { "" });
    if m.view != "section" {
        name.push_str(&format!("_L{}", num(m.length)));
    }
    if m.hatch && m.view == "section" {
        name.push_str("_H");
    }
    let name = name.replace('.', "_");
    if s.doc()?.block(&name).is_some() {
        return Ok(name);
    }
    {
        let d = s.doc_mut()?;
        if d.linetype("HIDDEN").is_none()
            && let Some(lt) = cadcraft_doc::library::standard_linetypes_for(d).into_iter().find(|l| l.name == "HIDDEN")
        {
            d.linetypes.push(lt);
        }
    }
    let mut hs: Vec<Handle> = Vec::new();
    let mut rings = Vec::new();
    for (k, hidden) in geometry(sec, m) {
        let h = s.add_entity(k)?;
        if hidden {
            s.doc_mut()?.modify_entity(h, |e| e.common.linetype = "HIDDEN".into()).map_err(|e| bad("steel", e.to_string()))?;
        } else {
            rings.push(h);
        }
        hs.push(h);
    }
    if m.hatch
        && m.view == "section"
        && let Some(hatch) = find_command("hatch")
    {
        let before = s.doc()?.space(&s.space()).map(|st| st.handles()).unwrap_or_default();
        (hatch.run)(
            s,
            &json!({ "handles": rings.iter().map(|h| h.hex()).collect::<Vec<_>>(), "pattern": "ANSI31", "scale": (sec.h.min(sec.b) / 40.0).max(0.25), "associative": false }),
        )?;
        let after = s.doc()?.space(&s.space()).map(|st| st.handles()).unwrap_or_default();
        hs.extend(after.into_iter().filter(|h| !before.contains(h)));
    }
    let desc = format!("{TAG}{}", serde_json::to_string(m).map_err(|e| bad("steel", e.to_string()))?);
    let block = find_command("block").ok_or_else(|| bad("steel", "BLOCK is not available"))?;
    (block.run)(
        s,
        &json!({ "name": name, "base": [0, 0], "handles": hs.iter().map(|h| h.hex()).collect::<Vec<_>>(), "keep": "delete", "description": desc }),
    )?;
    Ok(name)
}

fn insert(s: &mut Session, name: &str, at: Vec2, angle_deg: f64) -> Result<Value> {
    let ins = find_command("insert").ok_or_else(|| bad("steel", "INSERT is not available"))?;
    let r = (ins.run)(s, &json!({ "name": name, "at": [at.x, at.y], "rotation": angle_deg }))?;
    Ok(r.get("handle").cloned().unwrap_or(Value::Null))
}

fn place(s: &mut Session, st: &SteelSettings, sec: &Section, pts: &[Vec2]) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    if st.view == "section" {
        let m = Member {
            kind: st.kind.clone(),
            size: st.size.clone(),
            view: st.view.clone(),
            anchor: st.anchor.clone(),
            mirror: st.mirror,
            hatch: st.hatch,
            length: 0.0,
        };
        let name = ensure_block(s, sec, &m)?;
        for p in pts.iter().take(10_000) {
            out.push(insert(s, &name, *p, st.angle)?);
        }
    } else {
        for w in pts.chunks(2).take(10_000) {
            let [a, b] = w else { return Err(bad("steel", "side and top views take points in pairs (start, end)")) };
            let len = a.dist(*b);
            if len < 1e-6 {
                return Err(bad("steel", "a member needs a length"));
            }
            let m = Member {
                kind: st.kind.clone(),
                size: st.size.clone(),
                view: st.view.clone(),
                anchor: st.anchor.clone(),
                mirror: st.mirror,
                hatch: false,
                length: len,
            };
            let name = ensure_block(s, sec, &m)?;
            out.push(insert(s, &name, *a, (*b - *a).angle().to_degrees())?);
        }
    }
    Ok(out)
}

fn run_steel(s: &mut Session, p: &Value) -> Result<Value> {
    let mut st: SteelSettings = merge(&s.steel, p)?;
    let sec = normalize(&mut st, p.get("size").is_some())?;
    let pts = points_param(p, "points").filter(|v| !v.is_empty()).ok_or_else(|| bad("steel", "`points` is required"))?;
    let hs = place(s, &st, &sec, &pts)?;
    s.echo(format!("{} member(s): {} {}.", hs.len(), sec.name(), st.view));
    Ok(json!({ "handles": hs, "section": sec.name(), "area": area(&sec) }))
}

/// Cross-section area, mm² (radii left out), for weights: × 7.85e-3 kg/m per mm².
fn area(sec: &Section) -> f64 {
    let (h, b, t1, t2) = (sec.h, sec.b, sec.t1, sec.t2);
    match sec.kind {
        "C" => 2.0 * b * t2 + (h - 2.0 * t2) * t1,
        "L" => (h + b - t1) * t1,
        "H" => 2.0 * b * t2 + (h - 2.0 * t2) * t1,
        "FB" => h * b,
        _ => h * b - (h - 2.0 * t1) * (b - 2.0 * t1),
    }
}

/// The member a block reference is, from its block's description.
fn member_of(s: &Session, h: Handle) -> Option<(Member, f64, Vec2)> {
    let d = s.doc().ok()?;
    let EntityKind::Insert(i) = &d.entity(h)?.kind else { return None };
    let b = d.block(&i.block)?;
    let m: Member = serde_json::from_str(b.description.strip_prefix(TAG)?).ok()?;
    Some((m, i.rotation, i.insert.xy()))
}

fn run_member_info(s: &mut Session, p: &Value) -> Result<Value> {
    let h = p.get("handle").and_then(super::curves::handle_value).ok_or_else(|| bad("steel.member", "`handle` is required"))?;
    let (m, rot, at) = member_of(s, h).ok_or_else(|| bad("steel.member", "not a structural member"))?;
    let sec = find(&m.kind, &m.size);
    let mut v = serde_json::to_value(&m).map_err(|e| bad("steel.member", e.to_string()))?;
    v["rotation"] = json!(rot.to_degrees());
    v["at"] = json!([at.x, at.y]);
    if let Some(sec) = sec {
        v["area"] = json!(area(&sec));
        v["kgPerM"] = json!(area(&sec) * 7.85e-3);
    }
    Ok(v)
}

fn run_steeledit(s: &mut Session, p: &Value) -> Result<Value> {
    let h = p.get("handle").and_then(super::curves::handle_value).ok_or_else(|| bad("steeledit", "`handle` is required"))?;
    let (m, _, _) = member_of(s, h).ok_or_else(|| bad("steeledit", "not a structural member"))?;
    let mut st = SteelSettings {
        kind: m.kind.clone(),
        size: m.size.clone(),
        view: m.view.clone(),
        anchor: m.anchor.clone(),
        angle: 0.0,
        mirror: m.mirror,
        hatch: m.hatch,
    };
    st = merge(&st, p)?;
    let size_given = p.get("size").is_some();
    let sec = normalize(&mut st, size_given)?;
    let nm = Member {
        kind: st.kind.clone(),
        size: st.size.clone(),
        view: m.view.clone(),
        anchor: st.anchor.clone(),
        mirror: st.mirror,
        hatch: st.hatch,
        length: m.length,
    };
    let name = ensure_block(s, &sec, &nm)?;
    s.doc_mut()?
        .modify_entity(h, |e| {
            if let EntityKind::Insert(i) = &mut e.kind {
                i.block = name.clone();
            }
        })
        .map_err(|e| bad("steeledit", e.to_string()))?;
    Ok(json!({ "handle": h.hex(), "section": sec.name() }))
}

/// STEEL typed: section points, or start and end points of members; Settings (the first
/// prompt's default) opens the Steel dialog.
#[derive(Default)]
struct SteelM {
    start: Option<Vec2>,
    placed: usize,
}

impl Interactive for SteelM {
    fn name(&self) -> &'static str {
        "STEEL"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        let st = &s.steel;
        s.echo(format!("Member: {} {}, {} view.", st.kind, st.size, st.view));
        Ok(Step::Continue)
    }
    fn prompt(&self, s: &Session) -> Prompt {
        let section = s.steel.view == "section";
        match self.start {
            Some(a) => Prompt::new("Specify end point of member", Accept::POINT).base(a),
            None => {
                let p =
                    Prompt::new(if section { "Specify insertion point" } else { "Specify start point of member" }, Accept::POINT).kw(&["Settings"]);
                if self.placed == 0 { p.default("Settings") } else { p }
            }
        }
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Point(p) => {
                let mut st = s.steel.clone();
                let sec = normalize(&mut st, false)?;
                if st.view == "section" {
                    place(s, &st, &sec, &[p])?;
                    self.placed += 1;
                } else if let Some(a) = self.start.take() {
                    place(s, &st, &sec, &[a, p])?;
                    self.placed += 1;
                } else {
                    self.start = Some(p);
                }
                Ok(Step::Continue)
            }
            Input::Enter if self.placed > 0 || self.start.is_some() => Ok(Step::Done),
            Input::Enter | Input::Keyword(_) | Input::Text(_) => {
                s.steel_dialog = true;
                Ok(Step::Continue)
            }
            Input::Cancel => {
                s.steel_dialog = false;
                Ok(Step::Done)
            }
            _ => Ok(Step::Continue),
        }
    }
    fn preview(&self, s: &Session, c: Vec2) -> Vec<EntityKind> {
        let mut st = s.steel.clone();
        let Ok(sec) = normalize(&mut st, false) else { return Vec::new() };
        let (at, rot, len) = match (st.view.as_str(), self.start) {
            ("section", _) => (c, st.angle.to_radians(), 0.0),
            (_, Some(a)) if a.dist(c) > 1e-6 => (a, (c - a).angle(), a.dist(c)),
            _ => return Vec::new(),
        };
        let m = Member {
            kind: st.kind.clone(),
            size: st.size.clone(),
            view: st.view.clone(),
            anchor: st.anchor.clone(),
            mirror: st.mirror,
            hatch: false,
            length: len,
        };
        let t = Mat3::translate(at).then_before(Mat3::rotate(rot));
        geometry(&sec, &m)
            .into_iter()
            .map(|(mut k, _)| {
                k.transform(&t);
                k
            })
            .collect()
    }
}

/// STEELEDIT typed: pick a member; the UI opens the Steel dialog on it.
struct SteelEditM;

impl Interactive for SteelEditM {
    fn name(&self) -> &'static str {
        "STEELEDIT"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if let Some(h) = s.selection().into_iter().find(|h| member_of(s, *h).is_some()) {
            s.steel_edit = Some(h);
            return Ok(Step::Done);
        }
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        Prompt::new("Select a structural member", Accept::SELECT)
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Pick(hs) => match hs.into_iter().find(|h| member_of(s, *h).is_some()) {
                Some(h) => {
                    s.steel_edit = Some(h);
                    Ok(Step::Done)
                }
                None => {
                    s.echo("Not a structural member.");
                    Ok(Step::Continue)
                }
            },
            Input::Enter | Input::Cancel => Ok(Step::Done),
            _ => Ok(Step::Continue),
        }
    }
}
