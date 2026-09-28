//! Cloud layout (spike S4.A decision): server-side and deterministic —
//! the same roster always yields the same positions, so every browser and
//! every reconnect agree and nothing jumps on re-render. Agents gather
//! around their machine's cluster center (phase 5 adds machines, not
//! code), spread on a golden-angle spiral, then a few repulsion passes
//! keep points from overlapping.

/// SVG viewBox size.
pub const WIDTH: f64 = 1000.0;
pub const HEIGHT: f64 = 640.0;
const MARGIN: f64 = 40.0;
/// Largest point radius (metrics::radius) + halo + a gap, doubled.
const MIN_DIST: f64 = 2.0 * (18.0 + 7.0) + 6.0;
const SPIRAL_STEP: f64 = 30.0;
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
const PASSES: usize = 60;
/// Horizontal stretch of each cluster (the canvas is ~1.56:1).
const ASPECT: f64 = 1.25;
/// Per-pass pull toward the cluster center, applied only outside the
/// cluster's own footprint so a full canvas is not squeezed.
const PULL: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pos {
    pub x: f64,
    pub y: f64,
}

/// `clusters[i]` = number of agents on the i-th machine, in display
/// order. Returns each cluster's center, plus positions per cluster in the
/// same agent order.
pub fn layout(clusters: &[usize]) -> (Vec<Pos>, Vec<Vec<Pos>>) {
    let k = clusters.len();
    let centers: Vec<Pos> = (0..k)
        .map(|i| {
            if k == 1 {
                return Pos {
                    x: WIDTH / 2.0,
                    y: HEIGHT / 2.0,
                };
            }
            // start on the left: two machines sit side by side on the wide canvas
            let a = std::f64::consts::PI + std::f64::consts::TAU * i as f64 / k as f64;
            Pos {
                x: WIDTH / 2.0 + 0.3 * WIDTH * a.cos(),
                y: HEIGHT / 2.0 + 0.28 * HEIGHT * a.sin(),
            }
        })
        .collect();

    // flat list: (cluster, position)
    let mut pts: Vec<(usize, Pos)> = Vec::new();
    for (c, &n) in clusters.iter().enumerate() {
        for i in 0..n {
            let r = SPIRAL_STEP * (i as f64 + 0.5).sqrt();
            let a = i as f64 * GOLDEN_ANGLE;
            // wider than tall, like the canvas
            pts.push((
                c,
                Pos {
                    x: centers[c].x + ASPECT * r * a.cos(),
                    y: centers[c].y + r * a.sin() / ASPECT,
                },
            ));
        }
    }

    for _ in 0..PASSES {
        for i in 0..pts.len() {
            for j in (i + 1)..pts.len() {
                let (dx, dy) = (pts[j].1.x - pts[i].1.x, pts[j].1.y - pts[i].1.y);
                let d = (dx * dx + dy * dy).sqrt();
                if d >= MIN_DIST {
                    continue;
                }
                // coincident points separate along a fixed direction
                let (ux, uy) = if d > 1e-9 {
                    (dx / d, dy / d)
                } else {
                    (1.0, 0.0)
                };
                let push = (MIN_DIST - d) / 2.0;
                pts[i].1.x -= ux * push;
                pts[i].1.y -= uy * push;
                pts[j].1.x += ux * push;
                pts[j].1.y += uy * push;
            }
        }
        for (c, p) in &mut pts {
            // pull strays home, then stay on the canvas
            let (dx, dy) = (centers[*c].x - p.x, centers[*c].y - p.y);
            let reach = SPIRAL_STEP * (clusters[*c] as f64).sqrt() * ASPECT;
            if (dx * dx + dy * dy).sqrt() > reach {
                p.x += dx * PULL;
                p.y += dy * PULL;
            }
            p.x = p.x.clamp(MARGIN, WIDTH - MARGIN);
            p.y = p.y.clamp(MARGIN, HEIGHT - MARGIN);
        }
    }

    let mut out: Vec<Vec<Pos>> = clusters.iter().map(|&n| Vec::with_capacity(n)).collect();
    for (c, p) in pts {
        out[c].push(p);
    }
    (centers, out)
}

/// The viewBox `(x, y, w, h)` that frames `pts` with padding, at the
/// canvas aspect ratio, never zoomed in past half the canvas — a small
/// fleet fills the screen instead of huddling in the middle.
pub fn frame(pts: impl IntoIterator<Item = Pos>) -> (f64, f64, f64, f64) {
    const PAD: f64 = 70.0;
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if x0 > x1 {
        return (0.0, 0.0, WIDTH, HEIGHT);
    }
    let aspect = WIDTH / HEIGHT;
    let mut w = (x1 - x0 + 2.0 * PAD).max(WIDTH / 2.0);
    let mut h = (y1 - y0 + 2.0 * PAD).max(HEIGHT / 2.0);
    if w / h > aspect {
        h = w / aspect;
    } else {
        w = h * aspect;
    }
    ((x0 + x1 - w) / 2.0, (y0 + y1 - h) / 2.0, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(out: &[Vec<Pos>]) -> Vec<Pos> {
        out.iter().flatten().copied().collect()
    }

    #[test]
    fn frame_fits_the_points_at_canvas_aspect() {
        let pts = [Pos { x: 400.0, y: 300.0 }, Pos { x: 700.0, y: 340.0 }];
        let (x, y, w, h) = frame(pts);
        assert!((w / h - WIDTH / HEIGHT).abs() < 1e-9);
        for p in pts {
            assert!(p.x - x >= 70.0 && x + w - p.x >= 70.0);
            assert!(p.y - y >= 70.0 && y + h - p.y >= 70.0);
        }
        assert!(w >= WIDTH / 2.0, "never zooms past 2x");
        assert_eq!(frame([]), (0.0, 0.0, WIDTH, HEIGHT));
    }

    #[test]
    fn same_roster_same_positions() {
        assert_eq!(layout(&[12, 5]), layout(&[12, 5]));
    }

    #[test]
    fn hundred_points_stay_on_canvas_without_overlap() {
        let (_, out) = layout(&[100]);
        let pts = all(&out);
        assert_eq!(pts.len(), 100);
        let mut close = 0;
        for (i, a) in pts.iter().enumerate() {
            assert!((MARGIN..=WIDTH - MARGIN).contains(&a.x));
            assert!((MARGIN..=HEIGHT - MARGIN).contains(&a.y));
            for b in &pts[i + 1..] {
                if ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt() < MIN_DIST * 0.8 {
                    close += 1;
                }
            }
        }
        assert_eq!(close, 0, "points overlap");
    }

    #[test]
    fn agents_sit_nearest_their_own_machine() {
        let (centers, out) = layout(&[6, 6, 6]);
        for (c, cluster) in out.iter().enumerate() {
            for p in cluster {
                let d = |q: &Pos| (p.x - q.x).powi(2) + (p.y - q.y).powi(2);
                let nearest = (0..centers.len())
                    .min_by(|a, b| d(&centers[*a]).total_cmp(&d(&centers[*b])))
                    .unwrap();
                assert_eq!(nearest, c);
            }
        }
    }
}
