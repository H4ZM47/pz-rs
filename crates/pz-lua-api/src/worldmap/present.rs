//! Puts a recorded map frame ([`MapDraw`]) into the UI draw list: each
//! vertex is projected with the map's camera into its viewport (the
//! `glViewport` the drawer sets) and the runs become sprites (triangles
//! as degenerate quads) and lines.

use pz_render::{color_to_abgr, DrawCmd, Rgba, Sprite};
use pz_ui::{with_ui, UiDrawCmd};
use pz_worldmap::draw::MapDraw;
use pz_worldmap::vbo::{PolygonMode, Prim, RunTexture, Vert};

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

fn render_sprite(sprite: Sprite, nearest: bool, repeat: bool) -> UiDrawCmd {
    let command = if sprite.texture.is_some() && (nearest || repeat) {
        DrawCmd::SampledSprite {
            sprite,
            nearest,
            repeat,
        }
    } else {
        DrawCmd::Sprite(sprite)
    };
    UiDrawCmd::Render(command)
}

/// Emulate the map's GL_LINE polygon edge with a screen-space strip. Its
/// original endpoint UVs and colors remain on both sides of the strip.
fn polygon_edge(
    d: &MapDraw,
    a: &Vert,
    b: &Vert,
    width: f32,
    texture: Option<pz_render::Texture>,
) -> Option<Sprite> {
    let (a_pos, b_pos) = (project(d, a), project(d, b));
    let (dx, dy) = (b_pos[0] - a_pos[0], b_pos[1] - a_pos[1]);
    let length = dx.hypot(dy);
    if length == 0.0 || !length.is_finite() {
        return None;
    }
    let half = width / 2.0;
    let (nx, ny) = (-dy / length * half, dx / length * half);
    Some(Sprite {
        texture,
        x: [a_pos[0] + nx, b_pos[0] + nx, b_pos[0] - nx, a_pos[0] - nx],
        y: [a_pos[1] + ny, b_pos[1] + ny, b_pos[1] - ny, a_pos[1] - ny],
        u: [a.u, b.u, b.u, a.u],
        v: [a.v, b.v, b.v, a.v],
        color: [col(a.color), col(b.color), col(b.color), col(a.color)],
        flipped: false,
    })
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
                    if run.polygon_mode == PolygonMode::Line {
                        for i in 0..3 {
                            if let Some(edge) = polygon_edge(
                                d,
                                &t[i],
                                &t[(i + 1) % 3],
                                run.line_width,
                                texture.clone(),
                            ) {
                                out.push(render_sprite(edge, run.nearest, run.repeat));
                            }
                        }
                        continue;
                    }
                    out.push(render_sprite(
                        corners(&[&t[0], &t[1], &t[2], &t[2]]),
                        run.nearest,
                        run.repeat,
                    ));
                }
            }
            Prim::Quads => {
                for q in run.verts.as_chunks::<4>().0 {
                    if run.polygon_mode == PolygonMode::Line {
                        for i in 0..4 {
                            if let Some(edge) = polygon_edge(
                                d,
                                &q[i],
                                &q[(i + 1) % 4],
                                run.line_width,
                                texture.clone(),
                            ) {
                                out.push(render_sprite(edge, run.nearest, run.repeat));
                            }
                        }
                        continue;
                    }
                    out.push(render_sprite(
                        corners(&[&q[0], &q[1], &q[2], &q[3]]),
                        run.nearest,
                        run.repeat,
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use pz_joml::Matrix4f;
    use pz_worldmap::vbo::Vbo;

    #[test]
    fn texture_sampler_overrides_are_per_run_for_fills_and_polygon_edges() {
        let mut vbo = Vbo::default();
        let mut texture = pz_render::Texture::empty("sampled");
        texture.ready = true;
        for (mode, nearest, repeat, textured) in [
            (PolygonMode::Fill, true, false, true),
            (PolygonMode::Line, false, true, true),
            (PolygonMode::Fill, false, false, true),
            (PolygonMode::Fill, true, false, false),
        ] {
            vbo.set_polygon_mode(mode);
            vbo.start_run(Prim::Quads);
            if textured {
                vbo.set_texture(RunTexture::Texture(texture.clone()));
            }
            if nearest {
                vbo.set_nearest();
            }
            if repeat {
                vbo.set_repeat();
            }
            vbo.quad(0.0, 0.0, 1.0, 1.0, [1.0; 4]);
            vbo.end_run();
        }
        let draw = MapDraw {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            projection: Matrix4f::identity(),
            model_view: Matrix4f::identity(),
            model_view_projection: Matrix4f::identity(),
            wireframe: false,
            vbo,
            aborted: None,
        };
        let commands = lower(&draw);
        assert_eq!(commands.len(), 7);
        assert!(matches!(
            commands[0],
            UiDrawCmd::Render(DrawCmd::SampledSprite {
                nearest: true,
                repeat: false,
                ..
            })
        ));
        for edge in &commands[1..5] {
            assert!(matches!(
                edge,
                UiDrawCmd::Render(DrawCmd::SampledSprite {
                    nearest: false,
                    repeat: true,
                    ..
                })
            ));
        }
        assert!(
            matches!(&commands[5],UiDrawCmd::Render(DrawCmd::Sprite(sprite)) if sprite.texture.is_some())
        );
        assert!(
            matches!(&commands[6],UiDrawCmd::Render(DrawCmd::Sprite(sprite)) if sprite.texture.is_none())
        );
    }

    #[test]
    fn polygon_mode_is_per_run_and_wire_edges_preserve_texture_uvs_and_endpoint_colors() {
        let mut vbo = Vbo::default();
        vbo.set_polygon_mode(PolygonMode::Line);
        vbo.start_run(Prim::Triangles);
        let mut texture = pz_render::Texture::empty("wire");
        texture.ready = true;
        vbo.set_texture(RunTexture::Texture(texture));
        vbo.element_uv(0.0, 0.0, 0.0, 0.2, 0.3, [1.0, 0.0, 0.0, 1.0]);
        vbo.element_uv(1.0, 0.0, 0.0, 0.6, 0.4, [0.0, 1.0, 0.0, 1.0]);
        vbo.element_uv(0.0, 1.0, 0.0, 0.8, 0.9, [0.0, 0.0, 1.0, 1.0]);
        vbo.end_run();
        vbo.start_run(Prim::Quads);
        vbo.quad(0.0, 0.0, 1.0, 1.0, [1.0; 4]);
        vbo.end_run();
        vbo.start_run(Prim::Lines);
        vbo.line(0.0, 0.0, 1.0, 1.0, [1.0; 4]);
        vbo.end_run();
        vbo.set_polygon_mode(PolygonMode::Fill);
        vbo.start_run(Prim::Quads);
        vbo.quad(0.0, 0.0, 1.0, 1.0, [1.0; 4]);
        vbo.end_run();
        let draw = MapDraw {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            projection: Matrix4f::identity(),
            model_view: Matrix4f::identity(),
            model_view_projection: Matrix4f::identity(),
            wireframe: true,
            vbo,
            aborted: None,
        };
        let commands = lower(&draw);
        assert_eq!(
            commands.len(),
            9,
            "three triangle edges, four quad edges, one original line, one filled quad"
        );
        let UiDrawCmd::Render(DrawCmd::Sprite(edge)) = &commands[0] else {
            panic!("textured edge")
        };
        assert_eq!(edge.u, [0.2, 0.6, 0.6, 0.2]);
        assert_eq!(edge.v, [0.3, 0.4, 0.4, 0.3]);
        assert_eq!(
            edge.color,
            [
                col([1.0, 0.0, 0.0, 1.0]),
                col([0.0, 1.0, 0.0, 1.0]),
                col([0.0, 1.0, 0.0, 1.0]),
                col([1.0, 0.0, 0.0, 1.0])
            ]
        );
        assert!(edge.texture.is_some());
        assert_eq!((edge.y[0] - edge.y[3]).abs(), 1.0);
        assert!(matches!(commands[7], UiDrawCmd::Line { .. }));
        let UiDrawCmd::Render(DrawCmd::Sprite(fill)) = &commands[8] else {
            panic!("late filled quad")
        };
        assert_eq!(fill.x, [50.0, 100.0, 100.0, 50.0]);
        assert_eq!(fill.y, [50.0, 50.0, 0.0, 0.0]);
    }
}
