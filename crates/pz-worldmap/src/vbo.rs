//! The part of `zombie.core.opengl.VBORenderer` the map drawer records
//! into: runs of lines, triangles or quads, each with its texture and
//! sampling state, in scene coordinates (the map's projection and
//! model-view apply).

use pz_render::Texture;

/// `GL_LINES`, `GL_TRIANGLES`, `GL_QUADS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    Lines = 1,
    Triangles = 4,
    Quads = 7,
}

/// The two polygon modes used by the native map drawer. Lines retain
/// their ordinary primitive behavior in either mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PolygonMode {
    #[default]
    Fill,
    Line,
}

/// One `addElement` vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vert {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub u: f32,
    pub v: f32,
    pub color: [f32; 4],
}

/// What a run samples.
#[derive(Clone, Debug)]
pub enum RunTexture {
    Texture(Texture),
    /// An image-pyramid tile, by (pyramid key, x, y, z).
    Tile(std::rc::Rc<str>, i32, i32, i32),
}

/// A finished run (`startRun` ... `endRun`).
#[derive(Clone, Debug)]
pub struct Run {
    pub prim: Prim,
    pub polygon_mode: PolygonMode,
    pub line_width: f32,
    pub texture: Option<RunTexture>,
    /// `setMinMagFilters(GL_NEAREST, GL_NEAREST)`.
    pub nearest: bool,
    /// `GL_REPEAT` wrapping (`drawTextureTiled`).
    pub repeat: bool,
    pub verts: Vec<Vert>,
}

/// The recorder.
#[derive(Default, Debug)]
pub struct Vbo {
    pub runs: Vec<Run>,
    cur: Option<Run>,
    polygon_mode: PolygonMode,
}

impl Vbo {
    /// `startRun(format)` with `setMode(prim)`.
    pub fn start_run(&mut self, prim: Prim) {
        self.cur = Some(Run {
            prim,
            polygon_mode: self.polygon_mode,
            line_width: 1.0,
            texture: None,
            nearest: false,
            repeat: false,
            verts: Vec::new(),
        });
    }
    pub fn set_polygon_mode(&mut self, mode: PolygonMode) {
        self.polygon_mode = mode;
    }
    fn run(&mut self) -> &mut Run {
        self.cur.as_mut().expect("forgot to call startRun()")
    }
    pub fn set_line_width(&mut self, w: f32) {
        self.run().line_width = w;
    }
    pub fn set_texture(&mut self, t: RunTexture) {
        self.run().texture = Some(t);
    }
    pub fn set_nearest(&mut self) {
        self.run().nearest = true;
    }
    pub fn set_repeat(&mut self) {
        self.run().repeat = true;
    }
    /// `endRun()`.
    pub fn end_run(&mut self) {
        if let Some(r) = self.cur.take() {
            self.runs.push(r);
        }
    }
    pub fn prim(&self) -> Option<Prim> {
        self.cur.as_ref().map(|r| r.prim)
    }

    /// `addElement(x, y, z, r, g, b, a)`.
    pub fn element(&mut self, x: f32, y: f32, z: f32, c: [f32; 4]) {
        self.run().verts.push(Vert {
            x,
            y,
            z,
            u: 0.0,
            v: 0.0,
            color: c,
        });
    }
    /// `addElement(x, y, z, u, v, r, g, b, a)`.
    pub fn element_uv(&mut self, x: f32, y: f32, z: f32, u: f32, v: f32, c: [f32; 4]) {
        self.run().verts.push(Vert {
            x,
            y,
            z,
            u,
            v,
            color: c,
        });
    }
    /// `addLine(x0, y0, z0, x1, y1, z1, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4]) {
        self.element(x0, y0, 0.0, c);
        self.element(x1, y1, 0.0, c);
    }
    /// `addRectOutline(x0, y0, x1, y1, z, r, g, b, a)`.
    pub fn rect_outline(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4]) {
        self.line(x0, y0, x1, y0, c);
        self.line(x1, y0, x1, y1, c);
        self.line(x1, y1, x0, y1, c);
        self.line(x0, y1, x0, y0, c);
    }
    /// `addTriangle(x0, y0, z0, x1, y1, z1, x2, y2, z2, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn triangle(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32, c: [f32; 4]) {
        self.element(x0, y0, 0.0, c);
        self.element(x1, y1, 0.0, c);
        self.element(x2, y2, 0.0, c);
    }
    /// `addQuad(x0, y0, x1, y1, z, r, g, b, a)`, by the run's mode.
    pub fn quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4]) {
        match self.prim() {
            Some(Prim::Lines) => self.rect_outline(x0, y0, x1, y1, c),
            Some(Prim::Triangles) => {
                self.triangle(x0, y0, x1, y0, x0, y1, c);
                self.triangle(x1, y0, x1, y1, x0, y1, c);
            }
            _ => {
                self.element(x0, y0, 0.0, c);
                self.element(x1, y0, 0.0, c);
                self.element(x1, y1, 0.0, c);
                self.element(x0, y1, 0.0, c);
            }
        }
    }
    /// `addQuad(x0, y0, u0, v0, x1, y1, u1, v1, z, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn quad_uv(
        &mut self,
        x0: f32,
        y0: f32,
        u0: f32,
        v0: f32,
        x1: f32,
        y1: f32,
        u1: f32,
        v1: f32,
        c: [f32; 4],
    ) {
        self.quad4_uv(
            [
                [x0, y0, u0, v0],
                [x1, y0, u1, v0],
                [x1, y1, u1, v1],
                [x0, y1, u0, v1],
            ],
            c,
        );
    }
    /// `addQuad` with four free corners and texture coordinates.
    pub fn quad4_uv(&mut self, p: [[f32; 4]; 4], c: [f32; 4]) {
        match self.prim() {
            Some(Prim::Lines) => {
                for i in 0..4 {
                    let (a, b) = (p[i], p[(i + 1) % 4]);
                    self.line(a[0], a[1], b[0], b[1], c);
                }
            }
            Some(Prim::Triangles) => {
                for i in [0, 1, 3, 1, 2, 3] {
                    self.element_uv(p[i][0], p[i][1], 0.0, p[i][2], p[i][3], c);
                }
            }
            _ => {
                for q in p {
                    self.element_uv(q[0], q[1], 0.0, q[2], q[3], c);
                }
            }
        }
    }
}
