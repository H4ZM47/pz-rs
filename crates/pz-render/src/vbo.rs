//! `VBORenderer` runs: vertices with float colours drawn as lines or quad
//! strips under a model-view-projection matrix with the
//! `vboRenderer_PositionColor` / `vboRenderer_PositionNormalColor`
//! shaders (the colour passed through, `userDepth` added to the window
//! depth), and the primitives that build them: `Model.debugDrawAxis`,
//! `VBORenderer.addCylinder_Fill` and `VBORenderer.addTorus`, with the
//! original's vertex generation (float trigonometry through `Math.sin`/
//! `Math.cos`, the torus in doubles).
//!
//! Each run draws with `GL_DEPTH_TEST` as the run says (`setDepthTest`,
//! off by default and forced off by `setDepthTestForAllRuns(FALSE)`) and
//! the blend state current at the time; lines are
//! `min(lineWidth, 1)` wide.

/// `Run.mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VboMode {
    /// `GL_LINES` (1).
    Lines,
    /// `GL_QUAD_STRIP` (8): quad `i` is vertices `2i, 2i+1, 2i+3, 2i+2`.
    QuadStrip,
}

/// One vertex: position and float colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VboVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

/// One `VBORenderer` run as `flush` draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct VboRun {
    pub mode: VboMode,
    pub vertices: Vec<VboVertex>,
    /// `ModelViewProjection` (projection times model-view), column-major.
    pub mvp: [[f32; 4]; 4],
    /// `GL_DEPTH_TEST` for the run.
    pub depth_test: bool,
    /// The shader's `userDepth`.
    pub user_depth: f32,
}

impl VboRun {
    fn new(mode: VboMode, mvp: [[f32; 4]; 4]) -> Self {
        VboRun {
            mode,
            vertices: Vec::new(),
            mvp,
            depth_test: false,
            user_depth: 0.0,
        }
    }

    /// The vertex positions in clip space (`ModelViewProjection * pos`,
    /// then the shader's `userDepth` adjustment of z).
    pub fn clip_positions(&self) -> Vec<[f32; 4]> {
        let m = &self.mvp;
        self.vertices
            .iter()
            .map(|v| {
                let [x, y, z] = v.pos;
                let mut o = [0.0f32; 4];
                for (r, o) in o.iter_mut().enumerate() {
                    *o = m[0][r] * x + m[1][r] * y + m[2][r] * z + m[3][r];
                }
                let clip = (o[2] + 1.0) / 2.0 + self.user_depth;
                o[2] = clip * 2.0 - 1.0;
                o
            })
            .collect()
    }

    /// The run's triangles as vertex index triples: each `GL_QUAD_STRIP`
    /// quad as the polygon `2i, 2i+1, 2i+3, 2i+2`, fanned from its first
    /// vertex. Empty for lines.
    pub fn triangles(&self) -> Vec<[usize; 3]> {
        if self.mode != VboMode::QuadStrip {
            return Vec::new();
        }
        let quads = self.vertices.len().saturating_sub(2) / 2;
        (0..quads)
            .flat_map(|i| {
                let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 3, 2 * i + 2);
                [[a, b, c], [a, c, d]]
            })
            .collect()
    }
}

fn sin(f: f32) -> f32 {
    (f as f64).sin() as f32
}

fn cos(f: f32) -> f32 {
    (f as f64).cos() as f32
}

/// `Model.debugDrawAxis(0, 0, 0, flipX, flipY, flipZ, length, thickness)`:
/// red, green and blue lines from the origin, no depth test.
pub fn debug_draw_axis(mvp: [[f32; 4]; 4], length: f32, flip: [bool; 3]) -> VboRun {
    let mut run = VboRun::new(VboMode::Lines, mvp);
    let s = |f: bool| if f { -1.0 } else { 1.0 };
    let o = [0.0; 3];
    let ends = [
        ([length * s(flip[0]), 0.0, 0.0], [1.0, 0.0, 0.0, 1.0]),
        ([0.0, length * s(flip[1]), 0.0], [0.0, 1.0, 0.0, 1.0]),
        ([0.0, 0.0, length * s(flip[2])], [0.0, 0.0, 1.0, 1.0]),
    ];
    for (end, color) in ends {
        run.vertices.push(VboVertex { pos: o, color });
        run.vertices.push(VboVertex { pos: end, color });
    }
    run
}

/// `VBORenderer.addCylinder_Fill(baseRadius, topRadius, height, slices,
/// stacks, r, g, b, a)` (`GLU_FILL`, `GLU_OUTSIDE`): one quad-strip run
/// per stack.
#[allow(clippy::too_many_arguments)]
pub fn cylinder_fill(
    mvp: [[f32; 4]; 4],
    base_radius: f32,
    top_radius: f32,
    height: f32,
    slices: i32,
    stacks: i32,
    color: [f32; 4],
) -> Vec<VboRun> {
    let da = std::f32::consts::PI * 2.0 / slices as f32;
    let dr = (top_radius - base_radius) / stacks as f32;
    let dz = height / stacks as f32;
    let mut z = 0.0f32;
    let mut r = base_radius;
    let mut runs = Vec::new();
    for _ in 0..stacks {
        let mut run = VboRun::new(VboMode::QuadStrip, mvp);
        for i in 0..=slices {
            let (x, y) = if i == slices {
                (sin(0.0), cos(0.0))
            } else {
                (sin(i as f32 * da), cos(i as f32 * da))
            };
            run.vertices.push(VboVertex {
                pos: [x * r, y * r, z],
                color,
            });
            run.vertices.push(VboVertex {
                pos: [x * (r + dr), y * (r + dr), z + dz],
                color,
            });
        }
        runs.push(run);
        r += dr;
        z += dz;
    }
    runs
}

/// `VBORenderer.addTorus(r, c, rSeg, cSeg, r1, g1, b1, cameraRay)`: one
/// quad-strip run per ring segment; vertices whose direction from the
/// centre faces away from the camera ray (`dot < 0.1`) are opaque, the
/// others half colour at alpha 0.25. Drawn with `glFrontFace(GL_CW)`
/// (no culling, so no effect on the pixels).
pub fn torus(
    mvp: [[f32; 4]; 4],
    r: f64,
    c: f64,
    r_seg: i32,
    c_seg: i32,
    color: [f32; 3],
    ray_direction: [f32; 3],
) -> Vec<VboRun> {
    let tau = std::f64::consts::PI * 2.0;
    let mut runs = Vec::new();
    for i in 0..r_seg {
        let mut run = VboRun::new(VboMode::QuadStrip, mvp);
        for j in 0..=c_seg {
            for k in 0..=1 {
                let s = ((i + k) % r_seg) as f64 + 0.5;
                let t = (j % (c_seg + 1)) as f64;
                let x = (c + r * (s * tau / r_seg as f64).cos()) * (t * tau / c_seg as f64).cos();
                let y = (c + r * (s * tau / r_seg as f64).cos()) * (t * tau / c_seg as f64).sin();
                let z = r * (s * tau / r_seg as f64).sin();
                // Vector3f.set(double, double, double).normalize().
                let (sx, sy, sz) = (x as f32, y as f32, z as f32);
                let inv = 1.0 / (sx * sx + sy * sy + sz * sz).sqrt();
                let spoke = [sx * inv, sy * inv, sz * inv];
                let dot = spoke[0] * ray_direction[0]
                    + spoke[1] * ray_direction[1]
                    + spoke[2] * ray_direction[2];
                let col = if dot < 0.1 {
                    [color[0], color[1], color[2], 1.0]
                } else {
                    [color[0] / 2.0, color[1] / 2.0, color[2] / 2.0, 0.25]
                };
                run.vertices.push(VboVertex {
                    pos: [2.0 * x as f32, 2.0 * y as f32, 2.0 * z as f32],
                    color: col,
                });
            }
        }
        runs.push(run);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    #[test]
    fn axis_lines() {
        let r = debug_draw_axis(ID, 0.5, [false, false, true]);
        assert_eq!(r.mode, VboMode::Lines);
        let p: Vec<_> = r.vertices.iter().map(|v| v.pos).collect();
        assert_eq!(
            p,
            vec![
                [0.0; 3],
                [0.5, 0.0, 0.0],
                [0.0; 3],
                [0.0, 0.5, 0.0],
                [0.0; 3],
                [0.0, 0.0, -0.5]
            ]
        );
        assert_eq!(r.vertices[2].color, [0.0, 1.0, 0.0, 1.0]);
        assert!(!r.depth_test);
    }

    #[test]
    fn cylinder_strip() {
        // The translate gizmo's arrow tip: a cone of 8 slices, 1 stack.
        let runs = cylinder_fill(ID, 0.05, 0.0, 0.1, 8, 1, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(runs.len(), 1);
        let v = &runs[0].vertices;
        assert_eq!(v.len(), 18);
        // x = sin, y = cos: slice 0 at +y, slice 2 at +x; the strip closes
        // on sin(0), cos(0).
        assert_eq!(v[0].pos, [0.0, 0.05, 0.0]);
        assert_eq!(v[1].pos, [0.0, 0.0, 0.1]);
        let da = std::f32::consts::PI * 2.0 / 8.0;
        assert_eq!(
            v[4].pos,
            [
                ((2.0 * da) as f64).sin() as f32 * 0.05,
                ((2.0 * da) as f64).cos() as f32 * 0.05,
                0.0
            ]
        );
        assert_eq!(v[16].pos, v[0].pos);
        assert_eq!(runs[0].triangles().len(), 16);
        assert_eq!(runs[0].triangles()[0], [0, 1, 3]);
        assert_eq!(runs[0].triangles()[1], [0, 3, 2]);
    }

    #[test]
    fn torus_faces_and_fades() {
        // Looking down -z: the near (+z) half of the tube is faded.
        let runs = torus(ID, 0.01, 0.25, 8, 32, [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]);
        assert_eq!(runs.len(), 8);
        assert!(runs.iter().all(|r| r.vertices.len() == 66));
        let v = &runs[0].vertices;
        // i = 0: s = 0.5 and 1.5 segments round the tube, t = 0.
        let a = 0.5 * std::f64::consts::PI * 2.0 / 8.0;
        let x0 = (0.25 + 0.01 * a.cos()) * 1.0;
        assert_eq!(
            v[0].pos,
            [2.0 * x0 as f32, 0.0, 2.0 * (0.01 * a.sin()) as f32]
        );
        // Spoke (mostly +x, slightly +z) . (0, 0, -1) < 0.1: opaque.
        assert_eq!(v[0].color, [0.0, 0.0, 1.0, 1.0]);
        // A spoke facing the viewer would need dot >= 0.1: none of the
        // outer ring's do, as the ring lies in the view plane.
        assert!(runs
            .iter()
            .flat_map(|r| &r.vertices)
            .all(|v| v.color[3] == 1.0));
        let side = torus(ID, 0.01, 0.25, 8, 32, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        // Looking along +x, the far (+x) half of the ring is faded.
        assert!(side[0]
            .vertices
            .iter()
            .any(|v| v.color == [0.0, 0.0, 0.5, 0.25]));
    }

    #[test]
    fn clip_positions_apply_user_depth() {
        let mut r = debug_draw_axis(ID, 0.5, [false; 3]);
        r.user_depth = 0.25;
        let c = r.clip_positions();
        assert_eq!(c[1], [0.5, 0.0, 0.5, 1.0]);
    }
}
