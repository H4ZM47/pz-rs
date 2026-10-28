//! Port of the parts of Clipper2 1.5.4 `clipper.core.h` (and a few helpers
//! from `clipper.h`) that the engine and offsetter use.
//!
//! Copyright (c) Angus Johnson 2010-2025. Distributed under the Boost
//! Software License, Version 1.0 (see `LICENSE-BOOST` in this crate).
//!
//! # Floating point
//!
//! The reference library was compiled by clang for arm64 with the default
//! `-ffp-contract=on`: inside a single C++ expression, `a * b + c` and
//! `a * b - c * d` are fused into one `fmadd` (the left-hand product is
//! fused, a right-hand product is rounded first). Products produced by a
//! separate (even if inlined) function, such as `Sqr()`, are never fused.
//! Every such site is reproduced here with [`fma`], so results are
//! bit-identical to the arm64 build of the game library.

/// Fused multiply-add, `a * b + c` with a single rounding.
#[inline(always)]
pub(crate) fn fma(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

/// `std::nearbyint` in the default rounding mode (ties to even).
#[inline(always)]
pub(crate) fn nearbyint(v: f64) -> f64 {
    v.round_ties_even()
}

/// `static_cast<int64_t>(double)` as performed by arm64 `fcvtzs`
/// (truncate, saturate, NaN -> 0); identical to Rust's `as`.
#[inline(always)]
pub(crate) fn trunc_i64(v: f64) -> i64 {
    v as i64
}

/// A point with 64-bit integer coordinates (`Point64`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point64 {
    pub x: i64,
    pub y: i64,
}

impl Point64 {
    pub const fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }

    /// `Point64(double, double)`: `static_cast<int64_t>(std::round(v))`.
    #[inline]
    pub fn from_f64(x: f64, y: f64) -> Self {
        Self {
            x: x.round() as i64,
            y: y.round() as i64,
        }
    }

    /// `Point64(const PointD&)`.
    #[inline]
    pub fn from_pointd(p: PointD) -> Self {
        Self::from_f64(p.x, p.y)
    }
}

/// A point with `double` coordinates (`PointD`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointD {
    pub x: f64,
    pub y: f64,
}

impl PointD {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn from_point64(p: Point64) -> Self {
        Self {
            x: p.x as f64,
            y: p.y as f64,
        }
    }
}

pub type Path64 = Vec<Point64>;
pub type Paths64 = Vec<Path64>;
pub type PathD = Vec<PointD>;

/// `Rect64`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect64 {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

impl Rect64 {
    pub const fn new(left: i64, top: i64, right: i64, bottom: i64) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bottom <= self.top || self.right <= self.left
    }

    pub fn contains_rect(&self, rec: &Rect64) -> bool {
        rec.left >= self.left
            && rec.right <= self.right
            && rec.top >= self.top
            && rec.bottom <= self.bottom
    }

    pub fn mid_point(&self) -> Point64 {
        Point64::new(
            self.left.wrapping_add(self.right) / 2,
            self.top.wrapping_add(self.bottom) / 2,
        )
    }

    pub fn as_path(&self) -> Path64 {
        vec![
            Point64::new(self.left, self.top),
            Point64::new(self.right, self.top),
            Point64::new(self.right, self.bottom),
            Point64::new(self.left, self.bottom),
        ]
    }
}

/// `FillRule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    EvenOdd,
    NonZero,
    Positive,
    Negative,
}

pub(crate) const MAX_DBL: f64 = f64::MAX;

#[inline]
pub(crate) fn sqr(v: f64) -> f64 {
    v * v
}

/// `GetBounds(const Path64&)`.
pub fn get_bounds(path: &[Point64]) -> Rect64 {
    let mut xmin = i64::MAX;
    let mut ymin = i64::MAX;
    let mut xmax = i64::MIN;
    let mut ymax = i64::MIN;
    for p in path {
        if p.x < xmin {
            xmin = p.x;
        }
        if p.x > xmax {
            xmax = p.x;
        }
        if p.y < ymin {
            ymin = p.y;
        }
        if p.y > ymax {
            ymax = p.y;
        }
    }
    Rect64::new(xmin, ymin, xmax, ymax)
}

/// `StripDuplicates(Path64&, bool)`.
pub(crate) fn strip_duplicates(path: &mut Path64, is_closed_path: bool) {
    path.dedup();
    if is_closed_path {
        while path.len() > 1 && path.last() == path.first() {
            path.pop();
        }
    }
}

/// `ProductsAreEqual` (128-bit path).
#[inline]
pub(crate) fn products_are_equal(a: i64, b: i64, c: i64, d: i64) -> bool {
    (a as i128) * (b as i128) == (c as i128) * (d as i128)
}

/// `CrossProductSign` (128-bit path).
#[inline]
pub(crate) fn cross_product_sign(pt1: Point64, pt2: Point64, pt3: Point64) -> i32 {
    let a = pt2.x.wrapping_sub(pt1.x);
    let b = pt3.y.wrapping_sub(pt2.y);
    let c = pt2.y.wrapping_sub(pt1.y);
    let d = pt3.x.wrapping_sub(pt2.x);
    let ab = (a as i128) * (b as i128);
    let cd = (c as i128) * (d as i128);
    if ab > cd {
        1
    } else if ab < cd {
        -1
    } else {
        0
    }
}

/// `IsCollinear`.
#[inline]
pub(crate) fn is_collinear(pt1: Point64, shared: Point64, pt2: Point64) -> bool {
    let a = shared.x.wrapping_sub(pt1.x);
    let b = pt2.y.wrapping_sub(shared.y);
    let c = shared.y.wrapping_sub(pt1.y);
    let d = pt2.x.wrapping_sub(shared.x);
    products_are_equal(a, b, c, d)
}

/// `CrossProduct(pt1, pt2, pt3)` for `Point64`.
#[inline]
pub(crate) fn cross_product3(pt1: Point64, pt2: Point64, pt3: Point64) -> f64 {
    let a = pt2.x.wrapping_sub(pt1.x) as f64;
    let b = pt3.y.wrapping_sub(pt2.y) as f64;
    let c = pt2.y.wrapping_sub(pt1.y) as f64;
    let d = pt3.x.wrapping_sub(pt2.x) as f64;
    fma(a, b, -(c * d))
}

/// `DotProduct(pt1, pt2, pt3)` for `Point64`.
#[inline]
pub(crate) fn dot_product3(pt1: Point64, pt2: Point64, pt3: Point64) -> f64 {
    let a = pt2.x.wrapping_sub(pt1.x) as f64;
    let b = pt3.x.wrapping_sub(pt2.x) as f64;
    let c = pt2.y.wrapping_sub(pt1.y) as f64;
    let d = pt3.y.wrapping_sub(pt2.y) as f64;
    fma(a, b, c * d)
}

/// `CrossProduct(vec1, vec2)` for `PointD`.
#[inline]
pub(crate) fn cross_product_vec(v1: PointD, v2: PointD) -> f64 {
    fma(v1.y, v2.x, -(v2.y * v1.x))
}

/// `DotProduct(vec1, vec2)` for `PointD`.
#[inline]
pub(crate) fn dot_product_vec(v1: PointD, v2: PointD) -> f64 {
    fma(v1.x, v2.x, v1.y * v2.y)
}

/// `PerpendicDistFromLineSqrd` for `Point64`.
#[inline]
pub(crate) fn perpendic_dist_from_line_sqrd(pt: Point64, line1: Point64, line2: Point64) -> f64 {
    let a = pt.x.wrapping_sub(line1.x) as f64;
    let b = pt.y.wrapping_sub(line1.y) as f64;
    let c = line2.x.wrapping_sub(line1.x) as f64;
    let d = line2.y.wrapping_sub(line1.y) as f64;
    if c == 0.0 && d == 0.0 {
        return 0.0;
    }
    sqr(fma(a, d, -(c * b))) / fma(c, c, d * d)
}

/// `Area(const Path64&)`.
pub(crate) fn area_path(path: &[Point64]) -> f64 {
    let cnt = path.len();
    if cnt < 3 {
        return 0.0;
    }
    let mut a = 0.0f64;
    // it2 = cend - 1, stop = it2 (+1 when cnt is even)
    let mut it2 = cnt - 1;
    let stop = if cnt & 1 == 0 { it2 + 1 } else { it2 };
    let mut it1 = 0usize;
    while it1 != stop {
        let p2 = path[it2];
        let p1 = path[it1];
        a = fma(
            p2.y.wrapping_add(p1.y) as f64,
            p2.x.wrapping_sub(p1.x) as f64,
            a,
        );
        it2 = it1 + 1;
        let p2 = path[it2];
        a = fma(
            p1.y.wrapping_add(p2.y) as f64,
            p1.x.wrapping_sub(p2.x) as f64,
            a,
        );
        it1 += 2;
    }
    if cnt & 1 == 1 {
        let p2 = path[it2];
        let p1 = path[it1];
        a = fma(
            p2.y.wrapping_add(p1.y) as f64,
            p2.x.wrapping_sub(p1.x) as f64,
            a,
        );
    }
    a * 0.5
}

/// `GetSegmentIntersectPt` for `Point64` (`CLIPPER2_HI_PRECISION` off).
/// `ip` is left untouched when the segments are parallel.
#[inline]
pub(crate) fn get_segment_intersect_pt(
    ln1a: Point64,
    ln1b: Point64,
    ln2a: Point64,
    ln2b: Point64,
    ip: &mut Point64,
) -> bool {
    let dx1 = ln1b.x.wrapping_sub(ln1a.x) as f64;
    let dy1 = ln1b.y.wrapping_sub(ln1a.y) as f64;
    let dx2 = ln2b.x.wrapping_sub(ln2a.x) as f64;
    let dy2 = ln2b.y.wrapping_sub(ln2a.y) as f64;
    let det = fma(dy1, dx2, -(dy2 * dx1));
    if det == 0.0 {
        return false;
    }
    let t = fma(
        ln1a.x.wrapping_sub(ln2a.x) as f64,
        dy2,
        -((ln1a.y.wrapping_sub(ln2a.y) as f64) * dx2),
    ) / det;
    if t <= 0.0 {
        *ip = ln1a;
    } else if t >= 1.0 {
        *ip = ln1b;
    } else {
        ip.x = trunc_i64(fma(t, dx1, ln1a.x as f64));
        ip.y = trunc_i64(fma(t, dy1, ln1a.y as f64));
    }
    true
}

/// `GetSegmentIntersectPt` for `PointD`.
#[inline]
pub(crate) fn get_segment_intersect_pt_d(
    ln1a: PointD,
    ln1b: PointD,
    ln2a: PointD,
    ln2b: PointD,
    ip: &mut PointD,
) -> bool {
    let dx1 = ln1b.x - ln1a.x;
    let dy1 = ln1b.y - ln1a.y;
    let dx2 = ln2b.x - ln2a.x;
    let dy2 = ln2b.y - ln2a.y;
    let det = fma(dy1, dx2, -(dy2 * dx1));
    if det == 0.0 {
        return false;
    }
    let t = fma(ln1a.x - ln2a.x, dy2, -((ln1a.y - ln2a.y) * dx2)) / det;
    if t <= 0.0 {
        *ip = ln1a;
    } else if t >= 1.0 {
        *ip = ln1b;
    } else {
        ip.x = fma(t, dx1, ln1a.x);
        ip.y = fma(t, dy1, ln1a.y);
    }
    true
}

/// `GetSign`.
#[inline]
fn get_sign(val: f64) -> i32 {
    if val == 0.0 {
        0
    } else if val > 0.0 {
        1
    } else {
        -1
    }
}

/// `SegmentsIntersect(..., inclusive = false)`.
#[inline]
pub(crate) fn segments_intersect(
    seg1a: Point64,
    seg1b: Point64,
    seg2a: Point64,
    seg2b: Point64,
) -> bool {
    (get_sign(cross_product3(seg1a, seg2a, seg2b)) * get_sign(cross_product3(seg1b, seg2a, seg2b))
        < 0)
        && (get_sign(cross_product3(seg2a, seg1a, seg1b))
            * get_sign(cross_product3(seg2b, seg1a, seg1b))
            < 0)
}

/// `GetClosestPointOnSegment` for `Point64`.
#[inline]
#[allow(clippy::manual_clamp)]
pub(crate) fn get_closest_point_on_segment(
    off_pt: Point64,
    seg1: Point64,
    seg2: Point64,
) -> Point64 {
    if seg1.x == seg2.x && seg1.y == seg2.y {
        return seg1;
    }
    let dx = seg2.x.wrapping_sub(seg1.x) as f64;
    let dy = seg2.y.wrapping_sub(seg1.y) as f64;
    let mut q = fma(
        off_pt.x.wrapping_sub(seg1.x) as f64,
        dx,
        (off_pt.y.wrapping_sub(seg1.y) as f64) * dy,
    ) / (sqr(dx) + sqr(dy));
    if q < 0.0 {
        q = 0.0;
    } else if q > 1.0 {
        q = 1.0;
    }
    Point64::new(
        seg1.x.wrapping_add(trunc_i64(nearbyint(q * dx))),
        seg1.y.wrapping_add(trunc_i64(nearbyint(q * dy))),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum PointInPolygonResult {
    IsOn,
    IsInside,
    IsOutside,
}

/// `PointInPolygon(const Point64&, const Path64&)`.
pub(crate) fn point_in_polygon(pt: Point64, polygon: &[Point64]) -> PointInPolygonResult {
    use PointInPolygonResult::*;
    let len = polygon.len();
    if len < 3 {
        return IsOutside;
    }
    let mut val = 0;
    let cbegin = 0usize;
    let mut first = cbegin;
    let mut cend = len;
    while first != cend && polygon[first].y == pt.y {
        first += 1;
    }
    if first == cend {
        return IsOutside;
    }
    let mut is_above = polygon[first].y < pt.y;
    let starting_above = is_above;
    let mut curr = first + 1;
    let mut prev;
    loop {
        if curr == cend {
            if cend == first || first == cbegin {
                break;
            }
            cend = first;
            curr = cbegin;
        }
        if is_above {
            while curr != cend && polygon[curr].y < pt.y {
                curr += 1;
            }
            if curr == cend {
                continue;
            }
        } else {
            while curr != cend && polygon[curr].y > pt.y {
                curr += 1;
            }
            if curr == cend {
                continue;
            }
        }
        prev = if curr == cbegin { len - 1 } else { curr - 1 };
        let c = polygon[curr];
        let p = polygon[prev];
        if c.y == pt.y {
            if c.x == pt.x || (c.y == p.y && ((pt.x < p.x) != (pt.x < c.x))) {
                return IsOn;
            }
            curr += 1;
            if curr == first {
                break;
            }
            continue;
        }
        if pt.x < c.x && pt.x < p.x {
            // we're only interested in edges crossing on the left
        } else if pt.x > p.x && pt.x > c.x {
            val = 1 - val;
        } else {
            let d = cross_product3(p, c, pt);
            if d == 0.0 {
                return IsOn;
            }
            if (d < 0.0) == is_above {
                val = 1 - val;
            }
        }
        is_above = !is_above;
        curr += 1;
    }
    if is_above != starting_above {
        cend = len;
        if curr == cend {
            curr = cbegin;
        }
        prev = if curr == cbegin { cend - 1 } else { curr - 1 };
        let d = cross_product3(polygon[prev], polygon[curr], pt);
        if d == 0.0 {
            return IsOn;
        }
        if (d < 0.0) == is_above {
            val = 1 - val;
        }
    }
    if val == 0 {
        IsOutside
    } else {
        IsInside
    }
}

/// `Path2ContainsPath1(const Path64&, const Path64&)` from `clipper.h`.
pub(crate) fn path2_contains_path1(path1: &[Point64], path2: &[Point64]) -> bool {
    use PointInPolygonResult::*;
    let mut pip = IsOn;
    for &pt in path1 {
        match point_in_polygon(pt, path2) {
            IsOutside => {
                if pip == IsOutside {
                    return false;
                }
                pip = IsOutside;
            }
            IsInside => {
                if pip == IsInside {
                    return true;
                }
                pip = IsInside;
            }
            IsOn => {}
        }
    }
    if pip != IsInside {
        return false;
    }
    let mp1 = get_bounds(path1).mid_point();
    point_in_polygon(mp1, path2) == IsInside
}

/// `PI` as defined by Clipper2.
pub(crate) const PI: f64 = std::f64::consts::PI;

/// `Ellipse(const Point64& center, double rx, double ry, size_t steps)`.
pub(crate) fn ellipse(
    center: Point64,
    radius_x: f64,
    mut radius_y: f64,
    mut steps: usize,
) -> Path64 {
    if radius_x <= 0.0 {
        return Path64::new();
    }
    if radius_y <= 0.0 {
        radius_y = radius_x;
    }
    if steps <= 2 {
        steps = (PI * ((radius_x + radius_y) / 2.0).sqrt()) as usize;
    }
    let si = (2.0 * PI / steps as f64).sin();
    let co = (2.0 * PI / steps as f64).cos();
    let mut dx = co;
    let mut dy = si;
    let mut result = Path64::with_capacity(steps);
    result.push(Point64::from_f64(
        center.x as f64 + radius_x,
        center.y as f64,
    ));
    for _ in 1..steps {
        result.push(Point64::from_f64(
            fma(radius_x, dx, center.x as f64),
            fma(radius_y, dy, center.y as f64),
        ));
        let x = fma(dx, co, -(dy * si));
        dy = fma(dy, co, dx * si);
        dx = x;
    }
    result
}
