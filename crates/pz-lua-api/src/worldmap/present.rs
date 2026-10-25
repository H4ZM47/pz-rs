//! Puts a recorded map frame ([`MapDraw`]) into the UI draw list: each
//! vertex is projected with the map's camera into its viewport (the
//! `glViewport` the drawer sets) and the runs become sprites (triangles
//! as degenerate quads) and lines.

use pz_render::{color_to_abgr, DrawCmd, Rgba, Sprite};
use pz_ui::{with_ui, UiDrawCmd};
use pz_worldmap::draw::MapDraw;
use pz_worldmap::vbo::{Prim, RunTexture, Vert};

/// The UI position of a scene vertex.
fn project(d: &MapDraw, v: &Vert) -> [f32; 2] {
    let p = d
        .model_view_projection
        .project(v.x, v.y, v.z, [0, 0, d.width, d.height]);
    [d.x as f32 + p[0], d.y as f32 + d.height as f32 - p[1]]
}

fn col(c: [f32; 4]) -> u32 {
    color_to_abgr(c[0], c[1], c[2], c[3])
}

/// The sprites and lines for `d`, in run order.
pub fn lower(d: &MapDraw) -> Vec<UiDrawCmd> {
    let mut out = Vec::new();
    for run in &d.vbo.runs {
        let texture = match &run.texture {
            Some(RunTexture::Texture(t)) => Some(t.clone()),
            // TODO(worldmap pyramids): pyramid tiles.
            Some(RunTexture::Tile(..)) => continue,
            None => None,
        };
        if texture.as_ref().is_some_and(|t| !t.ready) {
            continue;
        }
        let corners = |vs: &[&Vert]| -> Sprite {
            let pts: Vec<[f32; 2]> = vs.iter().map(|v| project(d, v)).collect();
            Sprite {
                texture: texture.clone(),
                x: [pts[0][0], pts[1][0], pts[2][0], pts[3][0]],
                y: [pts[0][1], pts[1][1], pts[2][1], pts[3][1]],
                u: [vs[0].u, vs[1].u, vs[2].u, vs[3].u],
                v: [vs[0].v, vs[1].v, vs[2].v, vs[3].v],
                color: [
                    col(vs[0].color),
                    col(vs[1].color),
                    col(vs[2].color),
                    col(vs[3].color),
                ],
                flipped: false,
            }
        };
        match run.prim {
            Prim::Triangles => {
                for t in run.verts.as_chunks::<3>().0 {
                    out.push(UiDrawCmd::Render(DrawCmd::Sprite(corners(&[
                        &t[0], &t[1], &t[2], &t[2],
                    ]))));
                }
            }
            Prim::Quads => {
                for q in run.verts.as_chunks::<4>().0 {
                    out.push(UiDrawCmd::Render(DrawCmd::Sprite(corners(&[
                        &q[0], &q[1], &q[2], &q[3],
                    ]))));
                }
            }
            Prim::Lines => {
                for l in run.verts.as_chunks::<2>().0 {
                    let (a, b) = (project(d, &l[0]), project(d, &l[1]));
                    let c = l[0].color;
                    out.push(UiDrawCmd::Line {
                        texture: None,
                        x1: a[0],
                        y1: a[1],
                        x2: b[0],
                        y2: b[1],
                        thickness: run.line_width / 2.0,
                        color: Rgba(c[0], c[1], c[2], c[3]),
                    });
                }
            }
        }
    }
    out
}

/// Appends `d` to the UI draw list.
pub fn submit(d: &MapDraw) {
    let cmds = lower(d);
    with_ui(|m| m.sprite.cmds.extend(cmds));
}
