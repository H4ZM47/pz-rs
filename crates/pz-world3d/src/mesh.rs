//! Turns render squares into textured model-space quads.
//!
//! The isometric view is an orthographic (affine) projection, so a tile
//! sprite is exactly the projection of the square's geometry. Texturing a
//! planar face with the UVs found by projecting its corners into the
//! sprite's pixel frame (see [`uv_of`]) reproduces the sprite, including
//! its transparent pixels.
//!
//! Floors and walls keep their full geometry with UVs clamped into the
//! (trimmed) texture; cards are clipped to the texture rectangle.

use pz_iso_render::{RenderObject, RenderSquare, SpriteInfo, TileType};
use pz_render::iso::{x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::Texture;

use crate::to_model;

/// One textured quad in model space.
#[derive(Clone, Debug)]
pub struct Face {
    pub texture: Texture,
    pub pos: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    /// ABGR, as `vert_lights`.
    pub color: [u32; 4],
}

/// `performRenderFrame`'s scale for the sprite's instance (1.0).
fn frame_scale(tex: &Texture) -> f32 {
    if TILE_SCALE == 2 && tex.width_orig() == 64 && tex.height_orig() == 128 {
        2.0
    } else {
        1.0
    }
}

/// The object's effective `offsetY` (`renderFloorTile` adds
/// `renderYOffset` in screen units).
fn off_y(obj: &RenderObject) -> f32 {
    obj.offset_y + obj.render_y_offset * TILE_SCALE as f32
}

/// The position of `d` (relative to the square's origin corner, `dz` in
/// levels) as a fraction of the drawn trimmed texture rectangle, `0..1` on
/// both axes inside the texture, before flipping.
fn frac_of(tex: &Texture, sprite: &SpriteInfo, obj: &RenderObject, d: [f32; 3]) -> [f32; 2] {
    let s = frame_scale(tex);
    // Pixel in the sprite's frame: the frame origin is the square's corner
    // minus the object offset plus the sprite offset.
    let fx = x_to_screen(d[0], d[1]) + obj.offset_x - sprite.soff_x as f32;
    let fy = y_to_screen(d[0], d[1], d[2], 0) + off_y(obj) - sprite.soff_y as f32;
    let (w, h) = (tex.width as f32 * s, tex.height as f32 * s);
    if w <= 0.0 || h <= 0.0 {
        return [0.0, 0.0];
    }
    [(fx - tex.offset_x * s) / w, (fy - tex.offset_y * s) / h]
}

fn uv_from_frac(tex: &Texture, obj: &RenderObject, f: [f32; 2]) -> [f32; 2] {
    let flip = tex.flip != obj.flip;
    let a = if flip { 1.0 - f[0] } else { f[0] };
    [
        tex.x_start + a * (tex.x_end - tex.x_start),
        tex.y_start + f[1] * (tex.y_end - tex.y_start),
    ]
}

/// The texture coordinates, in the texture's own UV space (`x_start`..
/// `y_end`), at which the sprite drawn for a square shows the world point
/// `d`: relative to the square's origin corner, `x`/`y` in squares and `z`
/// in levels. Honours the sprite's and object's offsets, `renderYOffset`,
/// the 64x128 instance scale and `flip`.
pub fn uv_of(tex: &Texture, sprite: &SpriteInfo, obj: &RenderObject, d: [f32; 3]) -> [f32; 2] {
    uv_from_frac(tex, obj, frac_of(tex, sprite, obj, d))
}

fn channel(c: u32, i: u32) -> f32 {
    ((c >> (8 * i)) & 0xFF) as f32
}

/// Bilinear blend of the four corner colours at `(s, t)`.
fn blend(c: [u32; 4], s: f32, t: f32) -> u32 {
    let w = [(1.0 - s) * (1.0 - t), s * (1.0 - t), s * t, (1.0 - s) * t];
    let mut out = 0u32;
    for i in 0..4 {
        let v: f32 = (0..4).map(|k| channel(c[k], i) * w[k]).sum();
        out |= (v.round().clamp(0.0, 255.0) as u32) << (8 * i);
    }
    out
}

#[derive(Clone, Copy)]
struct Vtx {
    s: f32,
    t: f32,
    f: [f32; 2],
}

fn lerp_v(a: Vtx, b: Vtx, k: f32) -> Vtx {
    let l = |x: f32, y: f32| x + (y - x) * k;
    Vtx {
        s: l(a.s, b.s),
        t: l(a.t, b.t),
        f: [l(a.f[0], b.f[0]), l(a.f[1], b.f[1])],
    }
}

/// Keeps the part of the polygon where `val(v) >= 0`.
fn clip(poly: &[Vtx], val: impl Fn(&Vtx) -> f32) -> Vec<Vtx> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (va, vb) = (val(&a), val(&b));
        if va >= 0.0 {
            out.push(a);
        }
        if (va >= 0.0) != (vb >= 0.0) {
            out.push(lerp_v(a, b, va / (va - vb)));
        }
    }
    out
}

/// How far (frame pixels at `TILE_SCALE` 2, scaled with `TILE_SCALE` and the
/// instance scale) floor and wall UVs are pulled toward the face centre.
/// Sprite edges are anti-aliased / partly transparent, so sampling exactly on
/// the quad edge lets the clear colour bleed through as thin seams; the
/// inset makes edges sample interior opaque texels. Geometry is unchanged.
const EDGE_INSET: f32 = 3.0;

/// Moves each vertex's frame fraction toward the quad's centre by the inset
/// (capped at the distance to the centre).
fn inset_fracs(tex: &Texture, verts: &mut [Vtx]) {
    let s = frame_scale(tex);
    let (w, h) = (tex.width as f32 * s, tex.height as f32 * s);
    let d = EDGE_INSET * TILE_SCALE as f32 / 2.0 * s;
    let n = verts.len() as f32;
    let cx = verts.iter().map(|v| v.f[0]).sum::<f32>() / n;
    let cy = verts.iter().map(|v| v.f[1]).sum::<f32>() / n;
    for v in verts.iter_mut() {
        let (dx, dy) = ((cx - v.f[0]) * w, (cy - v.f[1]) * h);
        let len = dx.hypot(dy);
        if len > 1e-6 {
            let k = d.min(len) / len;
            v.f[0] += dx * k / w;
            v.f[1] += dy * k / h;
        }
    }
}

/// Where a face sits: its square and the model-space origin.
struct Ctx<'a> {
    tex: &'a Texture,
    sprite: &'a SpriteInfo,
    obj: &'a RenderObject,
    sq_pos: [f32; 3],
    origin: [f32; 3],
}

/// Emits the quad `p` (world corners, in `(s,t)` order `(0,0) (1,0) (1,1)
/// (0,1)`), textured from the sprite. With `fill` (floors and walls) the
/// sampled region is pulled in from the quad edges (see [`EDGE_INSET`]) and
/// UVs are clamped into the texture; otherwise the face is clipped to the
/// texture rectangle.
fn push_face(out: &mut Vec<Face>, c: &Ctx<'_>, p: [[f32; 3]; 4], color: [u32; 4], fill: bool) {
    let (tex, sprite, obj) = (c.tex, c.sprite, c.obj);
    if tex.width == 0 || tex.height == 0 || !tex.ready {
        return;
    }
    if p.iter().flatten().any(|v| !v.is_finite()) {
        return;
    }
    let st = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let mut verts: Vec<Vtx> = (0..4)
        .map(|i| {
            let d = [
                p[i][0] - c.sq_pos[0],
                p[i][1] - c.sq_pos[1],
                p[i][2] - c.sq_pos[2],
            ];
            Vtx {
                s: st[i].0,
                t: st[i].1,
                f: frac_of(tex, sprite, obj, d),
            }
        })
        .collect();
    if fill {
        inset_fracs(tex, &mut verts);
    }
    if fill {
        // Floors and walls keep their full geometry; UVs are clamped into
        // the texture rectangle, half a texel in, so a trimmed texture never
        // leaves gaps and filtering never reads neighbouring atlas texels.
        let (hx, hy) = (0.5 / tex.width as f32, 0.5 / tex.height as f32);
        let pos = [0, 1, 2, 3].map(|i| to_model(p[i], c.origin));
        let uv = [0, 1, 2, 3].map(|i| {
            let f = [
                verts[i].f[0].clamp(hx, 1.0 - hx),
                verts[i].f[1].clamp(hy, 1.0 - hy),
            ];
            uv_from_frac(tex, obj, f)
        });
        out.push(Face {
            texture: tex.clone(),
            pos,
            uv,
            color,
        });
        return;
    }
    let eps = 1e-3;
    if verts
        .iter()
        .all(|v| v.f.iter().all(|k| *k >= -eps && *k <= 1.0 + eps))
    {
        let pos = [0, 1, 2, 3].map(|i| to_model(p[i], c.origin));
        let uv = [0, 1, 2, 3].map(|i| uv_from_frac(tex, obj, verts[i].f));
        out.push(Face {
            texture: tex.clone(),
            pos,
            uv,
            color,
        });
        return;
    }
    let mut poly = verts;
    poly = clip(&poly, |v| v.f[0]);
    poly = clip(&poly, |v| 1.0 - v.f[0]);
    poly = clip(&poly, |v| v.f[1]);
    poly = clip(&poly, |v| 1.0 - v.f[1]);
    if poly.len() < 3 {
        return;
    }
    let make = |v: &Vtx| {
        let w = [
            (1.0 - v.s) * (1.0 - v.t),
            v.s * (1.0 - v.t),
            v.s * v.t,
            (1.0 - v.s) * v.t,
        ];
        let mut wp = [0.0f32; 3];
        for k in 0..4 {
            for (ax, acc) in wp.iter_mut().enumerate() {
                *acc += p[k][ax] * w[k];
            }
        }
        let f = [v.f[0].clamp(0.0, 1.0), v.f[1].clamp(0.0, 1.0)];
        (
            to_model(wp, c.origin),
            uv_from_frac(tex, obj, f),
            blend(color, v.s, v.t),
        )
    };
    let m: Vec<_> = poly.iter().map(make).collect();
    for i in 1..m.len() - 1 {
        let tri = [m[0], m[i], m[i + 1], m[i + 1]];
        out.push(Face {
            texture: tex.clone(),
            pos: tri.map(|t| t.0),
            uv: tri.map(|t| t.1),
            color: tri.map(|t| t.2),
        });
    }
}

enum Kind {
    Floor,
    WallN,
    WallW,
    WallNW,
    Card,
}

/// Whether the iso renderer draws the object with `render_floor`: the Floor
/// and TranslucentFloor layers of `cell::object_layer` (cell.rs:62-83), i.e.
/// `solidfloor` or `render_layer == 1` (not sloped, not fire) and
/// `transparent_floor`; the floor-like sprite flags are kept too.
fn is_floor(sprite: &SpriteInfo, obj: &RenderObject) -> bool {
    let f = &sprite.flags;
    if f.floor_overlay || f.diamond_floor || f.water || f.transparent_floor {
        return true;
    }
    (f.solidfloor || sprite.render_layer == 1)
        && !sprite.sloped
        && obj.kind != pz_iso_render::ObjectKind::Fire
}

fn classify(sprite: &SpriteInfo, obj: &RenderObject) -> Kind {
    match classify_closed(sprite, obj) {
        // An open hinged door keeps its closed door's flags and tile type
        // (`set_open_door_properties`), but its leaf is drawn swung onto
        // the square's other edge. Garage doors open upward and stay.
        k if matches!(obj.kind, pz_iso_render::ObjectKind::Door { open: true })
            && !sprite.flags.garage_door =>
        {
            match k {
                Kind::WallN => Kind::WallW,
                Kind::WallW => Kind::WallN,
                k => k,
            }
        }
        k => k,
    }
}

fn classify_closed(sprite: &SpriteInfo, obj: &RenderObject) -> Kind {
    let f = &sprite.flags;
    let t = sprite.tile_type;
    if is_floor(sprite, obj) {
        Kind::Floor
    } else if f.wall_nw {
        Kind::WallNW
    } else if f.wall_n
        || f.door_wall_n
        || f.window_n
        || f.door_n
        || matches!(t, TileType::DoorFrN | TileType::DoorN)
    {
        Kind::WallN
    } else if f.wall_w
        || f.door_wall_w
        || f.window_w
        || f.door_w
        || matches!(t, TileType::DoorFrW | TileType::DoorW)
    {
        Kind::WallW
    } else {
        Kind::Card
    }
}

/// The faces for one square, in model space around `origin` (world
/// coordinates).
pub fn square_faces(sq: &RenderSquare, origin: [f32; 3], out: &mut Vec<Face>) {
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    let v = &sq.vert_lights;
    let li = sq.light_info;
    let light = pz_iso_render::color::to_abgr(li.r, li.g, li.b, li.a);
    for obj in &sq.objects {
        let Some(sprite) = obj.sprite.as_deref() else {
            continue;
        };
        let Some(tex) = sprite.texture.as_ref() else {
            continue;
        };
        if sprite.flags.invisible {
            continue;
        }
        let c = Ctx {
            tex,
            sprite,
            obj,
            sq_pos: [x, y, z],
            origin,
        };
        let lit = |col: [u32; 4]| {
            if sprite.flags.unlit {
                [0xFFFF_FFFF; 4]
            } else {
                col
            }
        };
        let north = |out: &mut Vec<Face>| {
            let p = [
                [x, y, z],
                [x + 1.0, y, z],
                [x + 1.0, y, z + 1.0],
                [x, y, z + 1.0],
            ];
            push_face(out, &c, p, lit([v[0], v[1], v[5], v[4]]), true);
        };
        let west = |out: &mut Vec<Face>| {
            let p = [
                [x, y, z],
                [x, y + 1.0, z],
                [x, y + 1.0, z + 1.0],
                [x, y, z + 1.0],
            ];
            push_face(out, &c, p, lit([v[0], v[3], v[7], v[4]]), true);
        };
        match classify(sprite, obj) {
            Kind::Floor => {
                let p = [
                    [x, y, z],
                    [x + 1.0, y, z],
                    [x + 1.0, y + 1.0, z],
                    [x, y + 1.0, z],
                ];
                push_face(out, &c, p, lit([v[0], v[1], v[2], v[3]]), true);
            }
            Kind::WallN => north(out),
            Kind::WallW => west(out),
            Kind::WallNW => {
                north(out);
                west(out);
            }
            Kind::Card => {
                let s = frame_scale(tex);
                let ts = TILE_SCALE as f32;
                let (ox, oy) = (obj.offset_x, off_y(obj));
                // Trimmed rectangle in frame pixels.
                let fx0 = tex.offset_x * s;
                let fx1 = fx0 + tex.width as f32 * s;
                let fy0 = tex.offset_y * s;
                let fy1 = fy0 + tex.height as f32 * s;
                // Frame row of the floor diamond's middle (dz = 0).
                let fy_mid = 16.0 * ts + oy - sprite.soff_y as f32;
                let fy1 = fy1.min(fy_mid);
                if fy1 <= fy0 {
                    continue;
                }
                let t_of = |fx: f32| ((fx - ox + sprite.soff_x as f32) / (32.0 * ts) + 1.0) * 0.5;
                let dz_of = |fy: f32| (fy_mid - fy) / (96.0 * ts);
                let (t0, t1) = (t_of(fx0), t_of(fx1));
                let (dz0, dz1) = (dz_of(fy1), dz_of(fy0));
                let pt = |t: f32, dz: f32| [x + t, y + 1.0 - t, z + dz];
                let p = [pt(t0, dz0), pt(t1, dz0), pt(t1, dz1), pt(t0, dz1)];
                push_face(out, &c, p, lit([light; 4]), false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    fn tex(w: u32, h: u32, ox: f32, oy: f32, orig: (i32, i32)) -> Texture {
        let mut t = Texture::empty("t");
        t.width = w;
        t.height = h;
        t.offset_x = ox;
        t.offset_y = oy;
        t.width_orig = orig.0;
        t.height_orig = orig.1;
        t.ready = true;
        t
    }

    fn full() -> Texture {
        tex(128, 256, 0.0, 0.0, (0, 0))
    }

    fn sq_with(t: Texture, set: impl Fn(&mut SpriteInfo)) -> (RenderSquare, Rc<SpriteInfo>) {
        let mut s = SpriteInfo {
            texture: Some(t),
            ..Default::default()
        };
        set(&mut s);
        let s = Rc::new(s);
        let mut sq = RenderSquare::new(10, 20, 0);
        sq.objects.push(RenderObject::new(
            pz_iso_render::ObjectKind::Plain,
            Some(s.clone()),
        ));
        (sq, s)
    }

    /// Frame pixel of the UV (full 128x256 texture over 0..1).
    fn px(uv: [f32; 2]) -> [f32; 2] {
        [uv[0] * 128.0, uv[1] * 256.0]
    }

    fn close(a: [f32; 2], b: [f32; 2]) {
        assert!(
            (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3,
            "{a:?} != {b:?}"
        );
    }

    const O: [f32; 3] = [10.0, 20.0, 0.0];

    /// Frame-pixel inset used for floors/walls on the 128x256 test frame.
    const D: f32 = EDGE_INSET * TILE_SCALE as f32 / 2.0;

    /// `p` moved toward the centroid of `quad` by `D` pixels: the expected
    /// UV (in frame pixels) of a floor/wall corner after the edge inset.
    fn inset(p: [f32; 2], quad: [[f32; 2]; 4]) -> [f32; 2] {
        let c = [0, 1].map(|i| quad.iter().map(|q| q[i]).sum::<f32>() / 4.0);
        let (dx, dy) = (c[0] - p[0], c[1] - p[1]);
        let len = dx.hypot(dy);
        [p[0] + dx / len * D, p[1] + dy / len * D]
    }

    fn inset_all(quad: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
        quad.map(|p| inset(p, quad))
    }

    #[test]
    fn uv_of_matches_render_inst_pixels() {
        let (sq, s) = sq_with(full(), |_| {});
        let (t, o) = (s.texture.as_ref().unwrap(), &sq.objects[0]);
        close(px(uv_of(t, &s, o, [0.0, 0.0, 0.0])), [64.0, 192.0]);
        close(px(uv_of(t, &s, o, [1.0, 0.0, 0.0])), [128.0, 224.0]);
        close(px(uv_of(t, &s, o, [1.0, 1.0, 0.0])), [64.0, 256.0]);
        close(px(uv_of(t, &s, o, [0.0, 1.0, 0.0])), [0.0, 224.0]);
        close(px(uv_of(t, &s, o, [0.0, 0.0, 1.0])), [64.0, 0.0]);
    }

    #[test]
    fn floor_uvs_and_lights() {
        let (mut sq, _) = sq_with(full(), |s| s.flags.solidfloor = true);
        sq.vert_lights = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        // UVs are pulled toward the centre by the edge inset (geometry is not).
        let e = inset_all([[64.0, 192.0], [128.0, 224.0], [64.0, 256.0], [0.0, 224.0]]);
        for (uv, e) in f.uv.iter().zip(e) {
            close(px(*uv), e);
        }
        // Strictly inside the diamond, by exactly the inset along the
        // corner-to-centre line; the centre itself is unchanged.
        close(px(f.uv[0]), [64.0, 192.0 + D]);
        close(px(f.uv[1]), [128.0 - D, 224.0]);
        let ctr = [0, 1].map(|a| f.uv.iter().map(|u| px(*u)[a]).sum::<f32>() / 4.0);
        close(ctr, [64.0, 224.0]);
        assert_eq!(f.color, [1, 2, 3, 4]);
        assert_eq!(f.pos[0], [0.0, 0.0, 0.0]);
        assert_eq!(f.pos[1], [-1.0, 0.0, 0.0]);
        assert_eq!(f.pos[2], [-1.0, 0.0, -1.0]);
    }

    #[test]
    fn north_wall() {
        let (mut sq, _) = sq_with(full(), |s| s.flags.wall_n = true);
        sq.vert_lights = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        assert_eq!(f.pos[0], [0.0, 0.0, 0.0]);
        assert_eq!(f.pos[1], [-1.0, 0.0, 0.0]);
        assert_eq!(f.pos[2], [-1.0, crate::LEVEL_HEIGHT, 0.0]);
        assert_eq!(f.pos[3], [0.0, crate::LEVEL_HEIGHT, 0.0]);
        // Edge-inset UVs (positions above are unchanged).
        let e = inset_all([[64.0, 192.0], [128.0, 224.0], [128.0, 32.0], [64.0, 0.0]]);
        for (uv, e) in f.uv.iter().zip(e) {
            close(px(*uv), e);
        }
        assert_eq!(f.color, [1, 2, 6, 5]);
    }

    #[test]
    fn open_door_swings_to_the_other_edge() {
        let (mut sq, _) = sq_with(full(), |s| s.flags.door_n = true);
        sq.objects[0].kind = pz_iso_render::ObjectKind::Door { open: true };
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        // The west plane (model x constant), not the north one.
        assert_eq!(out[0].pos[1], [0.0, 0.0, -1.0]);
        sq.objects[0].kind = pz_iso_render::ObjectKind::Door { open: false };
        out.clear();
        square_faces(&sq, O, &mut out);
        assert_eq!(out[0].pos[1], [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn west_wall() {
        let (mut sq, _) = sq_with(full(), |s| s.flags.window_w = true);
        sq.vert_lights = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        assert_eq!(f.pos[1], [0.0, 0.0, -1.0]);
        assert_eq!(f.pos[3], [0.0, crate::LEVEL_HEIGHT, 0.0]);
        // Edge-inset UVs (positions above are unchanged).
        let e = inset_all([[64.0, 192.0], [0.0, 224.0], [0.0, 32.0], [64.0, 0.0]]);
        for (uv, e) in f.uv.iter().zip(e) {
            close(px(*uv), e);
        }
        assert_eq!(f.color, [1, 4, 8, 5]);
    }

    #[test]
    fn corner_wall_makes_two_faces() {
        let (sq, _) = sq_with(full(), |s| s.flags.wall_nw = true);
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn card_spans_frame_on_diagonal() {
        let (sq, _) = sq_with(full(), |_| {});
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        // Bottom edge from (x, y+1) to (x+1, y) at level z.
        assert_eq!(f.pos[0], [0.0, 0.0, -1.0]);
        assert_eq!(f.pos[1], [-1.0, 0.0, 0.0]);
        // The top is 224px (224/192 levels) up.
        let h = 224.0 / 192.0 * crate::LEVEL_HEIGHT;
        assert!((f.pos[2][1] - h).abs() < 1e-4 && (f.pos[3][1] - h).abs() < 1e-4);
        close(px(f.uv[0]), [0.0, 224.0]);
        close(px(f.uv[1]), [128.0, 224.0]);
        close(px(f.uv[2]), [128.0, 0.0]);
        close(px(f.uv[3]), [0.0, 0.0]);
    }

    #[test]
    fn trimmed_floor_strip() {
        // 128x64 strip at the bottom of the frame (offset_y 192).
        let mut t = tex(128, 64, 0.0, 192.0, (128, 256));
        t.y_start = 0.5;
        t.y_end = 0.75;
        let (sq, s) = sq_with(t, |s| s.flags.solidfloor = true);
        let tx = s.texture.as_ref().unwrap();
        // The north corner is at frame y 192, the top of the strip.
        close(uv_of(tx, &s, &sq.objects[0], [0.0, 0.0, 0.0]), [0.5, 0.5]);
        close(uv_of(tx, &s, &sq.objects[0], [1.0, 1.0, 0.0]), [0.5, 0.75]);
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn trimmed_wall_keeps_geometry_and_clamps_uvs() {
        let mut t = tex(128, 64, 0.0, 192.0, (128, 256));
        t.y_start = 0.5;
        t.y_end = 0.75;
        let (sq, _) = sq_with(t, |s| s.flags.wall_n = true);
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        // Full wall height is kept.
        assert_eq!(f.pos[2], [-1.0, crate::LEVEL_HEIGHT, 0.0]);
        assert_eq!(f.pos[3], [0.0, crate::LEVEL_HEIGHT, 0.0]);
        let (hx, hy) = (0.5 / 128.0, 0.5 / 64.0 * 0.25);
        for uv in f.uv {
            assert!(
                uv[0] >= hx - 1e-6
                    && uv[0] <= 1.0 - hx + 1e-6
                    && uv[1] >= 0.5 + hy - 1e-6
                    && uv[1] <= 0.75 - hy + 1e-6,
                "{uv:?}"
            );
        }
    }

    #[test]
    fn trimmed_floor_keeps_geometry_and_clamps_uvs() {
        let mut t = tex(128, 32, 0.0, 192.0, (128, 256));
        t.y_start = 0.0;
        t.y_end = 0.5;
        let (sq, _) = sq_with(t, |s| s.flags.solidfloor = true);
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        let f = &out[0];
        assert_eq!(f.pos[2], [-1.0, 0.0, -1.0]);
        let hy = 0.5 / 32.0 * 0.5;
        for uv in f.uv {
            assert!(uv[1] >= hy - 1e-6 && uv[1] <= 0.5 - hy + 1e-6, "{uv:?}");
        }
    }

    #[test]
    fn render_layer_one_object_is_horizontal() {
        let (sq, _) = sq_with(full(), |s| s.render_layer = 1);
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        assert!(out[0].pos.iter().all(|p| p[1] == 0.0));
        assert_eq!(out[0].pos[2], [-1.0, 0.0, -1.0]);
        // Sloped objects are not drawn as floors.
        let (sq, _) = sq_with(full(), |s| {
            s.render_layer = 1;
            s.sloped = true;
        });
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert!(out[0].pos.iter().any(|p| p[1] != 0.0));
    }

    #[test]
    fn trimmed_texture_offsets_shift_uvs() {
        // 64x64 trimmed image whose top-left sits at (32, 160) in the frame.
        let t = tex(64, 64, 32.0, 160.0, (128, 256));
        let (sq, s) = sq_with(t, |_| {});
        let tx = s.texture.as_ref().unwrap();
        // Frame (64,192) is texel (32,32) of the 64x64 image.
        close(uv_of(tx, &s, &sq.objects[0], [0.0, 0.0, 0.0]), [0.5, 0.5]);
        let mut out = Vec::new();
        square_faces(&sq, O, &mut out);
        assert_eq!(out.len(), 1);
        for uv in out[0].uv {
            assert!(
                uv[0] >= -1e-4 && uv[0] <= 1.0 + 1e-4 && uv[1] >= -1e-4 && uv[1] <= 1.0 + 1e-4,
                "{uv:?}"
            );
        }
        // Card spans exactly the trimmed rectangle: frame x 32..96.
        close(out[0].uv[0], [0.0, 1.0 - 0.0]);
    }

    #[test]
    fn flip_and_missing_texture() {
        let (mut sq, s) = sq_with(full(), |_| {});
        sq.objects[0].flip = true;
        let tx = s.texture.as_ref().unwrap();
        close(
            uv_of(tx, &s, &sq.objects[0], [1.0, 0.0, 0.0]),
            [0.0, 224.0 / 256.0],
        );
        let mut sq2 = RenderSquare::new(0, 0, 0);
        let s2 = Rc::new(SpriteInfo::default());
        sq2.objects.push(RenderObject::new(
            pz_iso_render::ObjectKind::Plain,
            Some(s2),
        ));
        sq2.objects
            .push(RenderObject::new(pz_iso_render::ObjectKind::Plain, None));
        let mut out = Vec::new();
        square_faces(&sq2, O, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn scale_two_for_64x128() {
        let t = tex(64, 128, 0.0, 0.0, (0, 0));
        let (sq, s) = sq_with(t, |_| {});
        let tx = s.texture.as_ref().unwrap();
        // Drawn at twice size: the north floor corner (frame 64,192) is
        // texel (32, 96) of 64x128 => uv (0.5, 0.75).
        close(uv_of(tx, &s, &sq.objects[0], [0.0, 0.0, 0.0]), [0.5, 0.75]);
    }
}
