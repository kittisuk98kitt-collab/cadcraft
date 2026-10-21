//! Compiled shape fonts (`.shx`): the user's own font files, read at run time.
//!
//! Three kinds of file: `shapes` (one-byte character codes), `unifont` (Unicode codes) and
//! `bigfont` (two-byte codes of an Asian code page; the extended kind keeps its height in the
//! second header byte). A glyph is a little program of vector, arc and pen commands (the shape
//! description language documented for shape definitions): length-and-direction bytes, and
//! codes 0–14 (end, pen down/up, scale down/up, push/pop, subshape, X-Y displacements, octant
//! arcs, fractional arcs, bulge arcs, vertical-only). Fonts are found by file name among the
//! fonts built in (`CRAFT_SHX_DIR` at build time, for a personal build) and, natively, in the
//! folders of `CADCRAFT_SHX_PATH` and a `Fonts` folder next to the program. No font file is
//! part of the source.
//!
//! Input is hostile: every read is checked, subshape depth, glyph program length and drawn
//! points are bounded, and a damaged font is simply not found.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cadcraft_geom::Vec2;

use crate::Shaped;

include!(concat!(env!("OUT_DIR"), "/shx_fonts.rs"));

/// Subshape nesting allowed.
const MAX_DEPTH: usize = 6;
/// Points one glyph may produce.
const MAX_POINTS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Shapes,
    Unifont,
    Bigfont,
}

/// A parsed shape font.
pub struct ShxFont {
    pub name: String,
    kind: Kind,
    /// Font units from the baseline to the top of a capital (the text height).
    above: f64,
    shapes: HashMap<u32, Vec<u8>>,
    /// The code page of a big font's two-byte codes.
    encoding: Option<&'static encoding_rs::Encoding>,
}

impl std::fmt::Debug for ShxFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ShxFont({}, {:?}, {} shapes)", self.name, self.kind, self.shapes.len())
    }
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

/// A glyph record: its name (to the first NUL), then its program.
fn program(rec: &[u8]) -> &[u8] {
    match rec.iter().position(|b| *b == 0) {
        Some(z) => rec.get(z + 1..).unwrap_or(&[]),
        None => &[],
    }
}

impl ShxFont {
    /// Parses a compiled shape font; `None` when it isn't one.
    pub fn parse(name: &str, b: &[u8]) -> Option<ShxFont> {
        let head_end = b.iter().take(64).position(|c| *c == 0x1a)? + 1;
        let head = String::from_utf8_lossy(b.get(..head_end)?).to_ascii_lowercase();
        if !head.starts_with("autocad-86") {
            return None;
        }
        let mut shapes = HashMap::new();
        let mut i = head_end;
        let (kind, above) = if head.contains("unifont") {
            let n = u32_at(b, i)? as usize;
            let dl = usize::from(u16_at(b, i + 4)?);
            let info = b.get(i + 6..i + 6 + dl)?;
            let z = info.iter().position(|c| *c == 0)?;
            let above = f64::from(*info.get(z + 1)?);
            i += 6 + dl;
            for _ in 0..n.min(70_000) {
                let (Some(code), Some(len)) = (u16_at(b, i), u16_at(b, i + 2)) else { break };
                let len = usize::from(len);
                let Some(rec) = b.get(i + 4..i + 4 + len) else { break };
                shapes.insert(u32::from(code), program(rec).to_vec());
                i += 4 + len;
            }
            (Kind::Unifont, above)
        } else if head.contains("bigfont") {
            let n = usize::from(u16_at(b, i + 2)?);
            let nr = usize::from(u16_at(b, i + 4)?);
            i += 6 + 4 * nr.min(1000);
            let mut above = 0.0;
            for k in 0..n.min(70_000) {
                let at = i + 8 * k;
                let (Some(code), Some(len), Some(off)) = (u16_at(b, at), u16_at(b, at + 2), u32_at(b, at + 4)) else { break };
                if len == 0 {
                    continue;
                }
                let off = off as usize;
                let Some(rec) = b.get(off..off.saturating_add(usize::from(len))) else { continue };
                let prog = program(rec);
                if code == 0 {
                    // Font info: above, below, modes, width; the extended kind has 0, height.
                    let a = f64::from(*prog.first().unwrap_or(&0));
                    above = if a > 0.0 { a } else { f64::from(*prog.get(1).unwrap_or(&0)) };
                } else {
                    shapes.insert(u32::from(code), prog.to_vec());
                }
            }
            (Kind::Bigfont, above)
        } else if head.contains("shapes") {
            let n = usize::from(u16_at(b, i + 4)?);
            let mut idx = Vec::with_capacity(n.min(4096));
            for k in 0..n.min(4096) {
                idx.push((u16_at(b, i + 6 + 4 * k)?, usize::from(u16_at(b, i + 8 + 4 * k)?)));
            }
            i += 6 + 4 * idx.len();
            let mut above = 0.0;
            for (code, len) in idx {
                let Some(rec) = b.get(i..i + len) else { break };
                i += len;
                let prog = program(rec);
                if code == 0 {
                    above = f64::from(*prog.first().unwrap_or(&0));
                } else {
                    shapes.insert(u32::from(code), prog.to_vec());
                }
            }
            (Kind::Shapes, above)
        } else {
            return None;
        };
        if shapes.is_empty() {
            return None;
        }
        let above = if above > 0.0 { above } else { 1.0 };
        let encoding = (kind == Kind::Bigfont).then(|| encoding_for(name));
        Some(ShxFont { name: name.to_string(), kind, above, shapes, encoding })
    }

    /// The glyph code drawing `c`, if the font has one.
    fn code_of(&self, c: char) -> Option<u32> {
        let has = |k: u32| self.shapes.contains_key(&k).then_some(k);
        match self.kind {
            Kind::Unifont | Kind::Shapes => {
                let u = u32::from(c);
                let alt: &[u32] = match c {
                    '°' => &[0xB0, 127],
                    '±' => &[0xB1, 128],
                    '⌀' | 'Ø' | 'ø' => &[0x2205, 0x2300, 129, 0xD8],
                    _ => &[],
                };
                has(u).or_else(|| alt.iter().find_map(|k| has(*k)))
            }
            Kind::Bigfont => {
                let enc = self.encoding?;
                let mut buf = [0u8; 4];
                let (bytes, _, bad) = enc.encode(c.encode_utf8(&mut buf));
                if bad {
                    return None;
                }
                let code = match bytes.as_ref() {
                    [a] => u32::from(*a),
                    [a, b] => (u32::from(*a) << 8) | u32::from(*b),
                    _ => return None,
                };
                has(code)
            }
        }
    }

    /// Whether the font draws `c`.
    pub fn has(&self, c: char) -> bool {
        self.code_of(c).is_some()
    }
}

/// The code page of a big font, from its file name.
fn encoding_for(name: &str) -> &'static encoding_rs::Encoding {
    let n = name.to_ascii_lowercase();
    if n.contains("gb") || n.contains("hz") {
        encoding_rs::GBK
    } else if n.contains("chineset") || n.contains("big5") {
        encoding_rs::BIG5
    } else if n.starts_with("wh") || n.contains("kor") {
        encoding_rs::EUC_KR
    } else {
        encoding_rs::SHIFT_JIS
    }
}

/// The 16 vector directions of a length-and-direction byte.
const DIRS: [(f64, f64); 16] = [
    (1.0, 0.0),
    (1.0, 0.5),
    (1.0, 1.0),
    (0.5, 1.0),
    (0.0, 1.0),
    (-0.5, 1.0),
    (-1.0, 1.0),
    (-1.0, 0.5),
    (-1.0, 0.0),
    (-1.0, -0.5),
    (-1.0, -1.0),
    (-0.5, -1.0),
    (0.0, -1.0),
    (0.5, -1.0),
    (1.0, -1.0),
    (1.0, -0.5),
];

fn s8(v: u8) -> f64 {
    f64::from(v as i8)
}

/// Runs glyph programs, collecting polylines in font units.
struct Pen<'a> {
    font: &'a ShxFont,
    at: Vec2,
    down: bool,
    cur: Vec<Vec2>,
    out: Vec<Vec<Vec2>>,
    points: usize,
}

impl Pen<'_> {
    fn flush(&mut self) {
        if self.cur.len() > 1 {
            self.out.push(std::mem::take(&mut self.cur));
        }
        self.cur.clear();
        self.cur.push(self.at);
    }
    fn to(&mut self, p: Vec2) {
        if !p.is_finite() || self.points >= MAX_POINTS {
            return;
        }
        if self.down {
            if self.cur.is_empty() {
                self.cur.push(self.at);
            }
            self.cur.push(p);
            self.points += 1;
        } else {
            self.cur.clear();
            self.cur.push(p);
        }
        self.at = p;
    }
    /// An arc round `c` of radius `r` from angle `a0` through `sweep` (radians).
    fn arc(&mut self, c: Vec2, r: f64, a0: f64, sweep: f64) {
        let n = ((sweep.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize).clamp(1, 64);
        for k in 1..=n {
            let a = a0 + sweep * k as f64 / n as f64;
            self.to(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    fn bulge(&mut self, d: Vec2, b: f64) {
        let end = self.at + d;
        if b.abs() < 1e-9 {
            self.to(end);
            return;
        }
        let chord = d.len();
        if chord < 1e-12 {
            return;
        }
        let sweep = 4.0 * b.atan();
        // The centre: off the chord's middle along its left normal (bulge = tan(sweep / 4)).
        let left = Vec2::new(-d.y, d.x) * (1.0 / chord);
        let c = self.at + d * 0.5 + left * (chord / 2.0 * (1.0 - b * b) / (2.0 * b));
        let r = c.dist(self.at);
        let a0 = (self.at.y - c.y).atan2(self.at.x - c.x);
        self.arc(c, r, a0, sweep);
        self.at = end;
    }
    /// Runs glyph `code`; returns false when the font lacks it.
    fn run(&mut self, code: u32, depth: usize, scale0: f64) -> bool {
        let Some(spec) = self.font.shapes.get(&code) else { return false };
        let spec = spec.clone();
        let mut sc = scale0;
        let mut stack: Vec<Vec2> = Vec::new();
        self.down = true;
        self.cur.clear();
        self.cur.push(self.at);
        let mut i = 0usize;
        let mut skip_next = false;
        let byte = |i: usize| spec.get(i).copied().unwrap_or(0);
        while i < spec.len() {
            let c = byte(i);
            i += 1;
            let skip = std::mem::take(&mut skip_next);
            if c > 15 {
                if !skip {
                    let (dx, dy) = DIRS.get(usize::from(c & 15)).copied().unwrap_or((0.0, 0.0));
                    let l = f64::from(c >> 4) * sc;
                    self.to(self.at + Vec2::new(dx * l, dy * l));
                }
                continue;
            }
            match c {
                0 => break,
                1 => {
                    if !skip {
                        self.down = true;
                        self.cur.clear();
                        self.cur.push(self.at);
                    }
                }
                2 => {
                    if !skip {
                        self.flush();
                        self.down = false;
                    }
                }
                3 => {
                    let v = byte(i);
                    i += 1;
                    if !skip && v > 0 {
                        sc /= f64::from(v);
                    }
                }
                4 => {
                    let v = byte(i);
                    i += 1;
                    if !skip && v > 0 {
                        sc *= f64::from(v);
                    }
                }
                5 => {
                    if !skip && stack.len() < 64 {
                        stack.push(self.at);
                    }
                }
                6 => {
                    if !skip && let Some(p) = stack.pop() {
                        self.flush();
                        self.at = p;
                        self.cur.clear();
                        self.cur.push(p);
                    }
                }
                7 => {
                    let mut sub_scale = sc;
                    let sub = match self.font.kind {
                        Kind::Shapes => {
                            i += 1;
                            u32::from(byte(i - 1))
                        }
                        Kind::Unifont => {
                            i += 2;
                            (u32::from(byte(i - 2)) << 8) | u32::from(byte(i - 1))
                        }
                        Kind::Bigfont => {
                            let first = byte(i);
                            if first == 0 {
                                // Extended: 0, code (2 bytes), origin x, y, width, height.
                                let code = (u32::from(byte(i + 1)) << 8) | u32::from(byte(i + 2));
                                let (ox, oy, h) = (f64::from(byte(i + 3)), f64::from(byte(i + 4)), f64::from(byte(i + 6)));
                                i += 7;
                                if !skip {
                                    self.flush();
                                    self.at += Vec2::new(ox * sc, oy * sc);
                                    self.cur.clear();
                                    self.cur.push(self.at);
                                }
                                if h > 0.0 {
                                    sub_scale = sc * h / self.font.above;
                                }
                                code
                            } else {
                                i += 2;
                                (u32::from(first) << 8) | u32::from(byte(i - 1))
                            }
                        }
                    };
                    if !skip && depth < MAX_DEPTH {
                        self.flush();
                        let saved_down = self.down;
                        self.run(sub, depth + 1, sub_scale);
                        self.flush();
                        self.down = saved_down;
                    }
                }
                8 => {
                    let (dx, dy) = (s8(byte(i)), s8(byte(i + 1)));
                    i += 2;
                    if !skip {
                        self.to(self.at + Vec2::new(dx * sc, dy * sc));
                    }
                }
                9 => {
                    while i + 1 < spec.len() {
                        let (dx, dy) = (s8(byte(i)), s8(byte(i + 1)));
                        i += 2;
                        if dx == 0.0 && dy == 0.0 {
                            break;
                        }
                        if !skip {
                            self.to(self.at + Vec2::new(dx * sc, dy * sc));
                        }
                    }
                }
                10 => {
                    let (r, o) = (f64::from(byte(i)) * sc, byte(i + 1) as i8);
                    i += 2;
                    if !skip {
                        self.octant_arc(r, o, 0.0, 0.0);
                    }
                }
                11 => {
                    let (so, eo) = (f64::from(byte(i)), f64::from(byte(i + 1)));
                    let r = f64::from(u16::from(byte(i + 2)) << 8 | u16::from(byte(i + 3))) * sc;
                    let o = byte(i + 4) as i8;
                    i += 5;
                    if !skip {
                        self.octant_arc(r, o, so, eo);
                    }
                }
                12 => {
                    let (dx, dy, b) = (s8(byte(i)), s8(byte(i + 1)), s8(byte(i + 2)));
                    i += 3;
                    if !skip {
                        self.bulge(Vec2::new(dx * sc, dy * sc), b / 127.0);
                    }
                }
                13 => {
                    while i + 1 < spec.len() {
                        let (dx, dy) = (s8(byte(i)), s8(byte(i + 1)));
                        i += 2;
                        if dx == 0.0 && dy == 0.0 {
                            break;
                        }
                        let b = s8(byte(i));
                        i += 1;
                        if !skip {
                            self.bulge(Vec2::new(dx * sc, dy * sc), b / 127.0);
                        }
                    }
                }
                14 => skip_next = true,
                _ => {}
            }
        }
        self.flush();
        true
    }
    /// Octant arc (code 10) and fractional arc (code 11, offsets in 1/256 of an octant): the
    /// current point is on the arc at its start.
    fn octant_arc(&mut self, r: f64, o: i8, start_off: f64, end_off: f64) {
        if r <= 0.0 {
            return;
        }
        let cw = o < 0;
        let v = o.unsigned_abs();
        let s = f64::from((v >> 4) & 7);
        let n = match v & 7 {
            0 => 8.0,
            k => f64::from(k),
        };
        let oct = std::f64::consts::FRAC_PI_4;
        let dir = if cw { -1.0 } else { 1.0 };
        let a0 = (s + dir * start_off / 256.0) * oct;
        let a1 = if end_off > 0.0 { (s + dir * (n - 1.0) + dir * end_off / 256.0) * oct } else { (s + dir * n) * oct };
        let c = self.at - Vec2::new(a0.cos(), a0.sin()) * r;
        let mut sweep = a1 - a0;
        if cw && sweep > 0.0 {
            sweep -= std::f64::consts::TAU;
        }
        if !cw && sweep < 0.0 {
            sweep += std::f64::consts::TAU;
        }
        self.arc(c, r, a0, sweep);
    }
}

/// A text style's shape fonts: the main font and its big font.
#[derive(Clone, Debug, Default)]
pub struct ShxPair {
    pub main: Option<Arc<ShxFont>>,
    pub big: Option<Arc<ShxFont>>,
}

/// Draws one glyph of `font` at `x` (height `h`, width factor `wf`, shear `shear`): its
/// polylines and advance; `None` when the font lacks `c`.
fn glyph(font: &ShxFont, c: char, x: f64, h: f64, wf: f64, shear: f64) -> Option<(Vec<Vec<Vec2>>, f64)> {
    let code = font.code_of(c)?;
    let mut pen = Pen { font, at: Vec2::ZERO, down: true, cur: Vec::new(), out: Vec::new(), points: 0 };
    pen.run(code, 0, 1.0);
    let k = h / font.above;
    let map = |p: Vec2| Vec2::new(x + p.x * k * wf + p.y * k * shear, p.y * k);
    let strokes = pen.out.into_iter().map(|s| s.into_iter().map(map).collect()).collect();
    Some((strokes, pen.at.x * k * wf))
}

/// Shapes one line in shape fonts; characters neither has come from the stroke font (and its
/// fallbacks).
pub fn shape(fonts: &ShxPair, s: &str, height: f64, width_factor: f64, oblique: f64) -> Shaped {
    let h = if height.is_finite() && height > 0.0 { height } else { 1.0 };
    let wf = if width_factor.is_finite() && width_factor.abs() > 1e-6 { width_factor } else { 1.0 };
    let shear = oblique.tan().clamp(-10.0, 10.0);
    let mut out = Shaped::default();
    let mut x = 0.0;
    let mut spans: Vec<(f64, f64, bool, bool)> = Vec::new();
    for (c, under, over) in crate::decode_controls(s) {
        if crate::bidi::is_format_control(c) {
            continue;
        }
        let drawn = fonts
            .main
            .as_deref()
            .and_then(|f| glyph(f, c, x, h, wf, shear))
            .or_else(|| fonts.big.as_deref().and_then(|f| glyph(f, c, x, h, wf, shear)));
        let adv = match drawn {
            Some((strokes, adv)) => {
                out.strokes.extend(strokes);
                adv
            }
            None => {
                // Not in the shape fonts: the built-in font draws it.
                let mut one = crate::shape_stroke(&c.to_string(), h, wf, oblique);
                let adv =
                    if c.is_whitespace() { crate::char_advance(c) * h * wf } else { one.width + crate::stroke::GAP / crate::stroke::CAP * h * wf };
                one.map(|p| Vec2::new(p.x + x, p.y));
                out.extend(one);
                adv
            }
        };
        spans.push((x, x + adv, under, over));
        x += adv;
    }
    out.strokes.extend(crate::decorations(&spans, h));
    out.width = x.max(0.0);
    out
}

/// Width of one line in shape fonts.
pub fn width(fonts: &ShxPair, s: &str, height: f64, width_factor: f64) -> f64 {
    shape(fonts, s, height, width_factor, 0.0).width
}

type Cache = Mutex<HashMap<String, Option<Arc<ShxFont>>>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The lookup key of a font name: lower-case file stem (`C:\Fonts\TXT.SHX` → `txt`).
fn key(name: &str) -> String {
    let base = name.trim().rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    base.strip_suffix(".shx").unwrap_or(&base).to_string()
}

/// Folders searched for `.shx` files at run time.
#[cfg(not(target_arch = "wasm32"))]
fn search_dirs() -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = std::env::var_os("CADCRAFT_SHX_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        v.push(dir.join("Fonts"));
        v.push(dir.join("fonts"));
    }
    v
}

fn read_font(k: &str) -> Option<ShxFont> {
    if let Some((n, bytes)) = SHX_FONTS.iter().find(|(n, _)| key(n) == k) {
        return ShxFont::parse(n, bytes);
    }
    #[cfg(not(target_arch = "wasm32"))]
    for dir in search_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.to_ascii_lowercase().ends_with(".shx") && key(&n) == k {
                // A font file is small; refuse anything unreasonably large.
                if e.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > 16 << 20 {
                    continue;
                }
                if let Ok(b) = std::fs::read(e.path()) {
                    return ShxFont::parse(&n, &b);
                }
            }
        }
    }
    None
}

/// The shape font named `name` (`txt`, `txt.shx`, a path), parsed once.
pub fn find(name: &str) -> Option<Arc<ShxFont>> {
    let k = key(name);
    if k.is_empty() {
        return None;
    }
    let Ok(mut c) = cache().lock() else { return read_font(&k).map(Arc::new) };
    c.entry(k.clone()).or_insert_with(|| read_font(&k).map(Arc::new)).clone()
}

/// Names (`txt.shx`) of the shape fonts available.
pub fn available() -> Vec<String> {
    let mut v: Vec<String> = SHX_FONTS.iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
    #[cfg(not(target_arch = "wasm32"))]
    for dir in search_dirs() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten().take(2000) {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if n.ends_with(".shx") {
                    v.push(n);
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Whether `name` is a big font (two-byte codes).
pub fn is_big_font(name: &str) -> bool {
    find(name).is_some_and(|f| f.kind == Kind::Bigfont)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny unifont made here (no font file is committed): `A` is a 6-unit-high caret drawn
    /// with direction vectors, `O` a circle (octant arc) and `-` a bulge arc.
    fn tiny() -> Vec<u8> {
        let mut b = b"AutoCAD-86 unifont 1.0\r\n\x1a".to_vec();
        let info = [b"T\0".as_slice(), &[6, 2, 0, 0, 0, 0]].concat();
        b.extend(3u32.to_le_bytes());
        b.extend((info.len() as u16).to_le_bytes());
        b.extend(&info);
        let mut glyph = |code: u16, prog: &[u8]| {
            let rec = [b"\0".as_slice(), prog].concat();
            b.extend(code.to_le_bytes());
            b.extend((rec.len() as u16).to_le_bytes());
            b.extend(rec);
        };
        // A: up-right 3 (dir 3 = (0.5,1) ×6), down-right, pen up, move right 1.
        glyph(u16::from(b'A'), &[0x63, 0x6D, 2, 0x10, 0]);
        // O: pen up to (2,0)... octant arc radius 2 starting at octant 6 (270°), full circle.
        glyph(u16::from(b'O'), &[2, 0x20, 1, 10, 2, 0x60, 2, 0x40, 0]);
        // -: a half circle bulge to (4,0), then a subshape A.
        glyph(u16::from(b'-'), &[12, 4, 0, 127, 7, 0, b'A', 0]);
        b
    }

    #[test]
    fn reads_and_draws_a_unifont() {
        let f = ShxFont::parse("tiny.shx", &tiny()).unwrap();
        assert_eq!(f.kind, Kind::Unifont);
        let pair = ShxPair { main: Some(Arc::new(f)), big: None };
        let sh = shape(&pair, "A", 6.0, 1.0, 0.0);
        // The caret is 6 high (text height = "above") and the advance 7 (6 + 1).
        let b = cadcraft_geom::Bounds2::from_points(sh.strokes.iter().flatten().copied());
        assert!((b.max.y - 6.0).abs() < 1e-9 && b.min.y.abs() < 1e-9, "{b:?}");
        assert!((sh.width - 7.0).abs() < 1e-9, "{}", sh.width);
        // Width factor and height scale it.
        assert!((shape(&pair, "A", 3.0, 0.5, 0.0).width - 1.75).abs() < 1e-9);
        // The circle: radius 2 round (2, 2).
        let o = shape(&pair, "O", 6.0, 1.0, 0.0);
        let ob = cadcraft_geom::Bounds2::from_points(o.strokes.iter().flatten().copied());
        assert!((ob.width() - 4.0).abs() < 1e-6 && (ob.height() - 4.0).abs() < 1e-6, "{ob:?}");
        // Bulge half circle then a subshape.
        let m = shape(&pair, "-", 6.0, 1.0, 0.0);
        assert!(m.strokes.len() >= 2);
        // A character the font lacks still shows (built-in font).
        assert!(!shape(&pair, "Z", 6.0, 1.0, 0.0).strokes.is_empty());
    }

    #[test]
    fn damaged_fonts_are_not_fonts() {
        let t = tiny();
        for cut in [0, 10, 25, 30, 40, t.len() - 3] {
            let _ = ShxFont::parse("x.shx", t.get(..cut).unwrap_or(&[]));
        }
        assert!(ShxFont::parse("x.shx", b"not a font").is_none());
        // Garbage programs never panic or run away.
        let mut b = tiny();
        for v in b.iter_mut().skip(40) {
            *v = v.wrapping_mul(37).wrapping_add(11);
        }
        if let Some(f) = ShxFont::parse("x.shx", &b) {
            let pair = ShxPair { main: Some(Arc::new(f)), big: None };
            let _ = shape(&pair, "AO-", 1.0, 1.0, 0.3);
        }
        assert!(find("no-such-font.shx").is_none());
        assert_eq!(key("C:\\Fonts\\TXT.SHX"), "txt");
    }
}
