//! `zombie.iso.worldgen.roads` and `utils.triangulation`: dirt roads across
//! the procedural (off-map) area. Road "nexus" points are drawn per meta
//! cell, joined by a Delaunay triangulation, edges longer than the
//! configured filter dropped, and every remaining edge becomes an L-shaped
//! pair of axis-aligned roads.
//!
//! The original keeps points, edges and roads in `HashSet`s of classes
//! hashed by identity (`Vector2D`, and `Road`'s enum field), so their
//! iteration order differs between runs of the game. The set of roads does
//! not depend on it (for points in general position); only the order in
//! which two roads crossing the same square draw from the square's random
//! generator does. This port iterates in a fixed order (points in
//! generation order, roads sorted by their end points).

use std::collections::{BTreeSet, HashMap};

use crate::defs::{RoadConfig, TileGroup};
use crate::params::WorldGenParams;

#[derive(Clone, Copy, Debug, PartialEq)]
struct V2 {
    x: f64,
    y: f64,
}

impl V2 {
    fn sub(self, o: V2) -> V2 {
        V2 {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
    fn add(self, o: V2) -> V2 {
        V2 {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }
    fn mult(self, s: f64) -> V2 {
        V2 {
            x: self.x * s,
            y: self.y * s,
        }
    }
    fn mag(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn mag_sqrt(self) -> f64 {
        self.x * self.x + self.y * self.y
    }
    fn dot(self, o: V2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    fn cross(self, o: V2) -> f64 {
        self.y * o.x - self.x * o.y
    }
}

/// Points are compared by identity in the original; here by index.
type P = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edge {
    a: P,
    b: P,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tri {
    a: P,
    b: P,
    c: P,
}

/// `DelaunayTriangulator` over `pts` (the super-triangle vertices are
/// appended).
struct Delaunay {
    pts: Vec<V2>,
    soup: Vec<Tri>,
}

fn signum(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

impl Delaunay {
    /// `new Edge2D(a, b)`: the end farther from the origin first.
    fn edge(&self, a: P, b: P) -> Edge {
        if self.pts[a].mag_sqrt() > self.pts[b].mag_sqrt() {
            Edge { a, b }
        } else {
            Edge { a: b, b: a }
        }
    }

    fn contains(&self, t: Tri, p: V2) -> bool {
        let (a, b, c) = (self.pts[t.a], self.pts[t.b], self.pts[t.c]);
        let pab = p.sub(a).cross(b.sub(a));
        let pbc = p.sub(b).cross(c.sub(b));
        if signum(pab) != signum(pbc) {
            return false;
        }
        let pca = p.sub(c).cross(a.sub(c));
        signum(pab) == signum(pca)
    }

    fn ccw(&self, t: Tri) -> bool {
        let (a, b, c) = (self.pts[t.a], self.pts[t.b], self.pts[t.c]);
        (a.x - c.x) * (b.y - c.y) - (a.y - c.y) * (b.x - c.x) > 0.0
    }

    fn in_circumcircle(&self, t: Tri, p: V2) -> bool {
        let (a, b, c) = (self.pts[t.a], self.pts[t.b], self.pts[t.c]);
        let a11 = a.x - p.x;
        let a21 = b.x - p.x;
        let a31 = c.x - p.x;
        let a12 = a.y - p.y;
        let a22 = b.y - p.y;
        let a32 = c.y - p.y;
        let a13 = (a.x - p.x) * (a.x - p.x) + (a.y - p.y) * (a.y - p.y);
        let a23 = (b.x - p.x) * (b.x - p.x) + (b.y - p.y) * (b.y - p.y);
        let a33 = (c.x - p.x) * (c.x - p.x) + (c.y - p.y) * (c.y - p.y);
        let det = a11 * a22 * a33 + a12 * a23 * a31 + a13 * a21 * a32
            - a13 * a22 * a31
            - a12 * a21 * a33
            - a11 * a23 * a32;
        if self.ccw(t) {
            det > 0.0
        } else {
            det < 0.0
        }
    }

    fn is_neighbour(t: Tri, e: Edge) -> bool {
        let has = |v| t.a == v || t.b == v || t.c == v;
        has(e.a) && has(e.b)
    }

    fn none_edge_vertex(t: Tri, e: Edge) -> Option<P> {
        [t.a, t.b, t.c].into_iter().find(|&v| v != e.a && v != e.b)
    }

    fn find_neighbour(&self, t: Tri, e: Edge) -> Option<Tri> {
        self.soup
            .iter()
            .copied()
            .find(|&s| Self::is_neighbour(s, e) && s != t)
    }

    fn remove(&mut self, t: Tri) {
        if let Some(i) = self.soup.iter().position(|&s| s == t) {
            self.soup.remove(i);
        }
    }

    fn closest(&self, e: Edge, p: V2) -> V2 {
        let (a, b) = (self.pts[e.a], self.pts[e.b]);
        let ab = b.sub(a);
        let t = (p.sub(a).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0);
        a.add(ab.mult(t))
    }

    /// `TriangleSoup.findNearestEdge` (a stable sort by distance, like
    /// `Arrays.sort` on objects).
    fn nearest_edge(&self, p: V2) -> Edge {
        let mut packs = Vec::new();
        for &t in &self.soup {
            let mut e3 = [
                self.edge(t.a, t.b),
                self.edge(t.b, t.c),
                self.edge(t.c, t.a),
            ]
            .map(|e| (e, self.closest(e, p).sub(p).mag()));
            e3.sort_by(|x, y| x.1.total_cmp(&y.1));
            packs.push(e3[0]);
        }
        packs.sort_by(|x, y| x.1.total_cmp(&y.1));
        packs[0].0
    }

    fn legalize(&mut self, t: Tri, e: Edge, v: P) {
        let Some(n) = self.find_neighbour(t, e) else {
            return;
        };
        if !self.in_circumcircle(n, self.pts[v]) {
            return;
        }
        self.remove(t);
        self.remove(n);
        let Some(nev) = Self::none_edge_vertex(n, e) else {
            return;
        };
        let first = Tri {
            a: nev,
            b: e.a,
            c: v,
        };
        let second = Tri {
            a: nev,
            b: e.b,
            c: v,
        };
        self.soup.push(first);
        self.soup.push(second);
        let e1 = self.edge(nev, e.a);
        let e2 = self.edge(nev, e.b);
        self.legalize(first, e1, v);
        self.legalize(second, e2, v);
    }

    /// `triangulate()` then `getEdges()`.
    fn triangulate(points: &[V2]) -> (Vec<V2>, Vec<Edge>) {
        let n = points.len();
        let mut pts = points.to_vec();
        let mut m = 0.0f64;
        for p in points {
            m = p.x.max(p.y).max(m);
        }
        m *= 16.0;
        pts.push(V2 { x: 0.0, y: 3.0 * m });
        pts.push(V2 { x: 3.0 * m, y: 0.0 });
        pts.push(V2 {
            x: -3.0 * m,
            y: -3.0 * m,
        });
        let mut d = Delaunay {
            pts,
            soup: vec![Tri {
                a: n,
                b: n + 1,
                c: n + 2,
            }],
        };
        for i in 0..n {
            let p = d.pts[i];
            let found = d.soup.iter().copied().find(|&t| d.contains(t, p));
            match found {
                None => {
                    let e = d.nearest_edge(p);
                    let Some(first) = d.soup.iter().copied().find(|&t| Self::is_neighbour(t, e))
                    else {
                        continue;
                    };
                    let Some(second) = d.find_neighbour(first, e) else {
                        continue;
                    };
                    let (Some(f), Some(s)) = (
                        Self::none_edge_vertex(first, e),
                        Self::none_edge_vertex(second, e),
                    ) else {
                        continue;
                    };
                    d.remove(first);
                    d.remove(second);
                    let t1 = Tri { a: e.a, b: f, c: i };
                    let t2 = Tri { a: e.b, b: f, c: i };
                    let t3 = Tri { a: e.a, b: s, c: i };
                    let t4 = Tri { a: e.b, b: s, c: i };
                    d.soup.extend([t1, t2, t3, t4]);
                    let (e1, e2, e3, e4) = (
                        d.edge(e.a, f),
                        d.edge(e.b, f),
                        d.edge(e.a, s),
                        d.edge(e.b, s),
                    );
                    d.legalize(t1, e1, i);
                    d.legalize(t2, e2, i);
                    d.legalize(t3, e3, i);
                    d.legalize(t4, e4, i);
                }
                Some(t) => {
                    d.remove(t);
                    let first = Tri {
                        a: t.a,
                        b: t.b,
                        c: i,
                    };
                    let second = Tri {
                        a: t.b,
                        b: t.c,
                        c: i,
                    };
                    let third = Tri {
                        a: t.c,
                        b: t.a,
                        c: i,
                    };
                    d.soup.extend([first, second, third]);
                    let (e1, e2, e3) = (d.edge(t.a, t.b), d.edge(t.b, t.c), d.edge(t.c, t.a));
                    d.legalize(first, e1, i);
                    d.legalize(second, e2, i);
                    d.legalize(third, e3, i);
                }
            }
        }
        d.soup.retain(|t| t.a < n && t.b < n && t.c < n);
        let mut edges = Vec::new();
        for &t in &d.soup {
            for e in [d.edge(t.a, t.b), d.edge(t.b, t.c), d.edge(t.a, t.c)] {
                let same = |o: &Edge| {
                    d.pts[o.a] == d.pts[e.a] && d.pts[o.b] == d.pts[e.b] && o.a == e.a && o.b == e.b
                };
                if !edges.iter().any(same) {
                    edges.push(e);
                }
            }
        }
        (d.pts, edges)
    }
}

/// `RoadDirection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoadDirection {
    NS,
    WE,
}

/// `org.joml.Vector2i.lengthSquared()`: `x * x + y * y` in `int`, widened.
fn length_squared(p: (i32, i32)) -> i64 {
    p.0.wrapping_mul(p.0).wrapping_add(p.1.wrapping_mul(p.1)) as i64
}

fn chunk_of(p: (i32, i32)) -> (i32, i32) {
    (
        pz_world::java::fastfloor(p.0 as f32 / 8.0),
        pz_world::java::fastfloor(p.1 as f32 / 8.0),
    )
}

/// `Road`: an axis-aligned segment.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Road {
    pub a: (i32, i32),
    pub b: (i32, i32),
    pub ca: (i32, i32),
    pub cb: (i32, i32),
    pub direction: RoadDirection,
    /// Index of the generator whose tiles and probability apply.
    pub generator: usize,
}

impl Road {
    fn new(a: (i32, i32), b: (i32, i32), generator: usize) -> Road {
        let test = length_squared(a) > length_squared(b);
        let direction = if a.0 == b.0 {
            RoadDirection::NS
        } else {
            RoadDirection::WE
        };
        let (a, b) = if test { (a, b) } else { (b, a) };
        Road {
            a,
            b,
            ca: chunk_of(a),
            cb: chunk_of(b),
            direction,
            generator,
        }
    }
}

/// `RoadEdge`: two roads meeting at `(a.x, b.y)`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RoadEdge {
    a: (i32, i32),
    b: (i32, i32),
    roads: [Road; 2],
}

impl RoadEdge {
    fn new(a: (i32, i32), b: (i32, i32), generator: usize) -> Self {
        let test = length_squared(a) > length_squared(b);
        let (a, b) = if test { (a, b) } else { (b, a) };
        let sub = (a.0, b.1);
        RoadEdge {
            a,
            b,
            roads: [Road::new(a, sub, generator), Road::new(sub, b, generator)],
        }
    }
}

/// `RoadGenerator`.
pub struct RoadGenerator {
    pub config: RoadConfig,
    pub index: usize,
    /// Road edges by chunk (`roadEdges`), deduplicated by end points.
    edges: HashMap<(i32, i32), Vec<usize>>,
    road_edges: Vec<RoadEdge>,
    min_cell: (i32, i32),
    max_cell: (i32, i32),
}

impl RoadGenerator {
    /// `new RoadGenerator(seed, config, offset)` over the meta grid's cell
    /// bounds. `fixed_points` are the two nexus points the original adds
    /// after the random ones.
    pub fn new(
        params: &WorldGenParams,
        config: RoadConfig,
        index: usize,
        offset: i64,
        min_cell: (i32, i32),
        max_cell: (i32, i32),
        fixed_points: &[(f64, f64)],
    ) -> Self {
        let mut points = Vec::new();
        for x in min_cell.0..=max_cell.0 {
            for y in min_cell.1..=max_cell.1 {
                let mut r = params.random(x, y, offset);
                if (r.next_float() as f64).partial_cmp(&config.probability)
                    != Some(std::cmp::Ordering::Less)
                {
                    continue;
                }
                let px = x * 256 + (256.0f32 * r.next_float()) as i32;
                let py = y * 256 + (256.0f32 * r.next_float()) as i32;
                points.push(V2 {
                    x: px as f64,
                    y: py as f64,
                });
            }
        }
        for &(x, y) in fixed_points {
            points.push(V2 { x, y });
        }
        let mut road_edges: Vec<RoadEdge> = Vec::new();
        if points.len() >= 3 {
            let (pts, edges) = Delaunay::triangulate(&points);
            let filtered: Vec<Edge> = edges
                .into_iter()
                .filter(|e| pts[e.a].sub(pts[e.b]).mag_sqrt() < config.filter)
                .collect();
            for p in &points {
                let point = (p.x as i32, p.y as i32);
                for e in &filtered {
                    let (a, b) = (pts[e.a], pts[e.b]);
                    let remote = if (a.x as i32, a.y as i32) == point {
                        (b.x as i32, b.y as i32)
                    } else if (b.x as i32, b.y as i32) == point {
                        (a.x as i32, a.y as i32)
                    } else {
                        continue;
                    };
                    road_edges.push(RoadEdge::new(point, remote, index));
                }
            }
        }
        let mut edges: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let mut unique: Vec<RoadEdge> = Vec::new();
        for re in road_edges {
            let i = match unique.iter().position(|u| u.a == re.a && u.b == re.b) {
                Some(i) => i,
                None => {
                    unique.push(re);
                    unique.len() - 1
                }
            };
            let re = &unique[i];
            for c in [chunk_of(re.a), chunk_of(re.b), chunk_of((re.a.0, re.b.1))] {
                let v = edges.entry(c).or_default();
                if !v.contains(&i) {
                    v.push(i);
                }
            }
        }
        RoadGenerator {
            config,
            index,
            edges,
            road_edges: unique,
            min_cell,
            max_cell,
        }
    }

    /// `getRoads(cx, cy)`: the roads crossing chunk `(cx, cy)`, in a fixed
    /// order.
    pub fn roads(&self, cx: i32, cy: i32) -> Vec<Road> {
        let mut out = BTreeSet::new();
        let mut take = |c: (i32, i32), dir: RoadDirection| {
            if let Some(list) = self.edges.get(&c) {
                for &i in list {
                    for r in &self.road_edges[i].roads {
                        if r.direction != dir {
                            continue;
                        }
                        let ok = match dir {
                            RoadDirection::WE => {
                                r.ca.0.min(r.cb.0) <= cx && r.ca.0.max(r.cb.0) >= cx
                            }
                            RoadDirection::NS => {
                                r.ca.1.min(r.cb.1) <= cy && r.ca.1.max(r.cb.1) >= cy
                            }
                        };
                        if ok {
                            out.insert(r.clone());
                        }
                    }
                }
            }
        };
        for x in self.min_cell.0 * 32..=self.max_cell.0 * 32 {
            take((x, cy), RoadDirection::WE);
        }
        for y in self.min_cell.1 * 32..=self.max_cell.1 * 32 {
            take((cx, y), RoadDirection::NS);
        }
        out.into_iter().collect()
    }

    /// `Road.getSingleFeatures()`.
    pub fn tiles(&self) -> &[TileGroup] {
        &self.config.tiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_triangulates_into_two_triangles() {
        let pts = [
            V2 { x: 0.0, y: 0.0 },
            V2 { x: 10.0, y: 1.0 },
            V2 { x: 1.0, y: 11.0 },
            V2 { x: 12.0, y: 12.0 },
        ];
        let (_, edges) = Delaunay::triangulate(&pts);
        assert_eq!(edges.len(), 5);
    }

    #[test]
    fn road_edge_is_an_l() {
        let e = RoadEdge::new((100, 10), (20, 200), 0);
        assert_eq!(e.a, (20, 200));
        assert_eq!(e.roads[0].direction, RoadDirection::NS);
        assert_eq!(e.roads[1].direction, RoadDirection::WE);
        assert_eq!(length_squared((50000, 50000)), 5000000000i64 as i32 as i64);
    }
}
