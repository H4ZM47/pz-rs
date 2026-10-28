//! `zombie.worldMap.Rasterize`: the cells (or tiles) a triangle covers.

#[derive(Clone, Copy, Default)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dx: f32,
    dy: f32,
}

impl Edge {
    fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let (x0, y0, x1, y1) = if y0 > y1 {
            (x1, y1, x0, y0)
        } else {
            (x0, y0, x1, y1)
        };
        Edge {
            x0,
            y0,
            x1,
            y1,
            dx: x1 - x0,
            dy: y1 - y0,
        }
    }
}

fn scan_span(mut e0: Edge, mut e1: Edge, min: i32, max: i32, s: &mut dyn FnMut(i32, i32)) {
    let y0 = (min as f64).max((e1.y0 as f64).floor()) as i32;
    let y1 = (max as f64).min((e1.y1 as f64).ceil()) as i32;
    if e0.x0 == e1.x0 && e0.y0 == e1.y0 {
        if e0.x0 + e1.dy / e0.dy * e0.dx < e1.x1 {
            std::mem::swap(&mut e0, &mut e1);
        }
    } else if e0.x1 - e1.dy / e0.dy * e0.dx < e1.x0 {
        std::mem::swap(&mut e0, &mut e1);
    }
    let m0 = (e0.dx / e0.dy) as f64;
    let m1 = (e1.dx / e1.dy) as f64;
    let d0 = if e0.dx > 0.0 { 1.0 } else { 0.0 };
    let d1 = if e1.dx < 0.0 { 1.0 } else { 0.0 };
    for y in y0..y1 {
        let x0 = m0 * 0f64.max((e0.dy as f64).min(y as f64 + d0 - e0.y0 as f64)) + e0.x0 as f64;
        let x1 = m1 * 0f64.max((e1.dy as f64).min(y as f64 + d1 - e1.y0 as f64)) + e1.x0 as f64;
        for x in (x1.floor() as i32)..(x0.ceil() as i32) {
            s(x, y);
        }
    }
}

/// `scanTriangle(xa, ya, xb, yb, xc, yc, min, max, callback)`.
#[allow(clippy::too_many_arguments)]
pub fn scan_triangle(
    xa: f32,
    ya: f32,
    xb: f32,
    yb: f32,
    xc: f32,
    yc: f32,
    min: i32,
    max: i32,
    s: &mut dyn FnMut(i32, i32),
) {
    let mut ab = Edge::new(xa, ya, xb, yb);
    let mut bc = Edge::new(xb, yb, xc, yc);
    let mut ca = Edge::new(xc, yc, xa, ya);
    if ab.dy > ca.dy {
        std::mem::swap(&mut ab, &mut ca);
    }
    if bc.dy > ca.dy {
        std::mem::swap(&mut bc, &mut ca);
    }
    if ab.dy > 0.0 {
        scan_span(ca, ab, min, max, s);
    }
    if bc.dy > 0.0 {
        scan_span(ca, bc, min, max, s);
    }
}
