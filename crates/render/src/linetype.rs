//! Apply a dash pattern along a polyline.

use cadcraft_doc::Linetype;
use cadcraft_geom::Vec2;

/// Split `pts` into dashes. Dots come back as single-point runs. When the scaled pattern is
/// shorter than `min_pattern` (sub-pixel), the polyline is returned whole.
pub fn apply(pts: &[Vec2], lt: &Linetype, scale: f64, min_pattern: f64) -> Vec<Vec<Vec2>> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let total = lt.pattern_length() * scale;
    if pts.len() < 2 || total <= 1e-12 || total < min_pattern {
        return vec![pts.to_vec()];
    }
    // Guard against huge dash counts (zoomed out a lot): fall back to continuous.
    let len: f64 = pts.windows(2).map(|w| w.first().zip(w.get(1)).map(|(a, b)| a.dist(*b)).unwrap_or(0.0)).sum();
    if len / total > 50_000.0 {
        return vec![pts.to_vec()];
    }
    let pattern: Vec<f64> = lt.pattern.iter().map(|d| d.length * scale).collect();
    let mut out: Vec<Vec<Vec2>> = Vec::new();
    let mut cur: Vec<Vec2> = Vec::new();
    // Where along the pattern the run starts. A single straight segment (a line) is laid out
    // around its middle, as drafted centre lines are: the pattern's shortest dash (CENTER's and
    // PHANTOM's short dash) sits on the midpoint, so a hole's centre lines cross on their short
    // dashes and both ends look alike. Other runs start at the pattern's start.
    let phase = match (pts, centre_of_short_dash(&pattern)) {
        ([a, b], Some(c)) => (c - a.dist(*b) / 2.0).rem_euclid(total),
        _ => 0.0,
    };
    // The element the run starts in, and how much of it is left.
    let mut idx = 0usize;
    let mut skipped = 0.0;
    let mut remaining = 0.0;
    for (k, d) in pattern.iter().enumerate().filter(|_| phase > 0.0) {
        if skipped + d.abs() > phase || k + 1 == pattern.len() {
            idx = k;
            remaining = (skipped + d.abs() - phase).max(0.0);
            break;
        }
        skipped += d.abs();
    }
    let first = pattern.get(idx).copied().unwrap_or(0.0);
    if phase == 0.0 {
        remaining = first.abs();
    }
    let mut on = first >= 0.0;
    if on && first == 0.0 {
        if let Some(p) = pts.first() {
            out.push(vec![*p]);
        }
        idx = (idx + 1) % pattern.len();
        let n = pattern.get(idx).copied().unwrap_or(0.0);
        remaining = n.abs();
        on = n > 0.0;
    }
    if on && let Some(p) = pts.first() {
        cur.push(*p);
    }
    let mut guard = 0usize;
    for w in pts.windows(2) {
        let (Some(&a), Some(&b)) = (w.first(), w.get(1)) else { continue };
        let seg = a.dist(b);
        if seg <= 1e-15 {
            continue;
        }
        let dir = (b - a) / seg;
        let mut pos = 0.0;
        while seg - pos > remaining {
            guard += 1;
            if guard > 2_000_000 {
                return vec![pts.to_vec()];
            }
            pos += remaining;
            let p = a + dir * pos;
            if on {
                cur.push(p);
                out.push(std::mem::take(&mut cur));
            }
            idx = (idx + 1) % pattern.len();
            let n = pattern.get(idx).copied().unwrap_or(0.0);
            if n == 0.0 {
                out.push(vec![p]);
                idx = (idx + 1) % pattern.len();
                let n2 = pattern.get(idx).copied().unwrap_or(0.0);
                remaining = n2.abs();
                on = n2 > 0.0;
            } else {
                remaining = n.abs();
                on = n > 0.0;
            }
            if on {
                cur.push(p);
            }
        }
        remaining -= seg - pos;
        if on {
            cur.push(b);
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// The middle of the pattern's shortest dash, as a distance from the pattern's start, when its
/// dashes differ in length (a centre or phantom line); `None` for evenly dashed patterns.
fn centre_of_short_dash(pattern: &[f64]) -> Option<f64> {
    let dashes = || pattern.iter().copied().filter(|d| *d > 0.0);
    let (lo, hi) = (dashes().fold(f64::INFINITY, f64::min), dashes().fold(0.0, f64::max));
    if !(lo.is_finite() && hi > lo * 1.2) {
        return None;
    }
    let mut pos = 0.0;
    for d in pattern {
        if *d > 0.0 && (*d - lo).abs() <= 1e-12 * hi.max(1.0) {
            return Some(pos + d / 2.0);
        }
        pos += d.abs();
    }
    None
}

/// The dashes of the pattern along a run of length `len`, as (start, end) distances; a dot has
/// start == end. Laid out like [`apply`]. `None` when the run is drawn continuous: the scaled
/// pattern is shorter than `min_pattern` or would repeat too often.
pub fn ranges(len: f64, lt: &Linetype, scale: f64, min_pattern: f64) -> Option<Vec<(f64, f64)>> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let total = lt.pattern_length() * scale;
    if !(len.is_finite() && len > 0.0) || total <= 1e-12 || total < min_pattern || len / total > 50_000.0 {
        return None;
    }
    let mut out = Vec::new();
    let mut pos = 0.0;
    for d in lt.pattern.iter().map(|d| d.length * scale).cycle() {
        if pos >= len || out.len() > 2_000_000 {
            break;
        }
        if d >= 0.0 {
            out.push((pos, (pos + d).min(len)));
        }
        pos += d.abs();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_split_line() {
        let lt = Linetype::simple("DASHED", "", &[0.5, -0.25]);
        let d = apply(&[Vec2::ZERO, Vec2::new(3.0, 0.0)], &lt, 1.0, 0.0);
        // 3.0 / 0.75 = 4 patterns → 4 dashes.
        assert_eq!(d.len(), 4);
        assert!((d[0][1].x - 0.5).abs() < 1e-9);
        assert!((d[1][0].x - 0.75).abs() < 1e-9);
    }

    #[test]
    fn centre_line_is_laid_out_around_its_middle() {
        // CENTER: long 1.25, gap 0.25, short 0.25, gap 0.25.
        let lt = Linetype::simple("CENTER", "", &[1.25, -0.25, 0.25, -0.25]);
        for len in [0.7, 3.0, 7.3, 20.0] {
            let d = apply(&[Vec2::ZERO, Vec2::new(len, 0.0)], &lt, 1.0, 0.0);
            let mid = len / 2.0;
            // The short dash is centred on the midpoint.
            let m = d.iter().find(|r| r[0].x <= mid && r[r.len() - 1].x >= mid).expect("a dash at the middle");
            assert!((m[0].x - (mid - 0.125)).abs() < 1e-9 && (m[m.len() - 1].x - (mid + 0.125)).abs() < 1e-9, "{len}: {m:?}");
            // Both ends alike.
            let (a, z) = (d.first().unwrap(), d.last().unwrap());
            assert!((a[0].x - (len - z[z.len() - 1].x)).abs() < 1e-9);
            assert!(((a[a.len() - 1].x - a[0].x) - (z[z.len() - 1].x - z[0].x)).abs() < 1e-9, "{len}");
        }
        // A polyline still starts on the pattern's start.
        let d = apply(&[Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 3.0)], &lt, 1.0, 0.0);
        assert!((d[0][1].x - 1.25).abs() < 1e-9);
    }

    #[test]
    fn dots_are_single_points() {
        let lt = Linetype::simple("DOT", "", &[0.0, -0.25]);
        let d = apply(&[Vec2::ZERO, Vec2::new(1.0, 0.0)], &lt, 1.0, 0.0);
        assert!(d.iter().all(|r| r.len() == 1));
        assert!(d.len() >= 4);
    }

    #[test]
    fn tiny_pattern_is_continuous() {
        let lt = Linetype::simple("DASHED", "", &[0.5, -0.25]);
        let d = apply(&[Vec2::ZERO, Vec2::new(3.0, 0.0)], &lt, 1.0, 10.0);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn dash_continues_across_vertices() {
        let lt = Linetype::simple("DASHED", "", &[0.5, -0.25]);
        let d = apply(&[Vec2::ZERO, Vec2::new(0.3, 0.0), Vec2::new(0.3, 1.0)], &lt, 1.0, 0.0);
        assert_eq!(d[0].len(), 3); // the first dash turns the corner
    }
}
