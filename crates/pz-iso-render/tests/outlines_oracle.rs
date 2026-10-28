//! Parity of `ModelOutlines` against the original: the highlight colour's
//! ABGR round trip, `beginRenderOutline`'s mask sharing and flushing,
//! `endFrame`'s passes (buffers, viewports, programs, uniforms, quads) at
//! several zooms, and the player drawn into the mask behind outlines.
//!
//! The oracle harness (outside the repo) runs the original
//! `ModelOutlines`/`ModelSlotRenderData.performRenderCharacterOutline` on a
//! 1280x720 screen with GL shimmed and writes `outlines.txt`. Point
//! `PZ_OUTLINES_ORACLE` at that file; without it this test does nothing.
//!
//! The port's draw commands are expanded into the GL calls the original
//! makes for them: `OutlineThicken` is `endFrame`'s two passes into
//! `fboB` and `fboC` (each a viewport quad whose texture coordinates
//! address the texels under it, which is how the GPU backend samples),
//! `OutlineBlit` the masked blit, `OutlineBegin`/`OutlineEnd` `fboA`'s
//! `startDrawing`/`endDrawing` with the solid shaders' `u_color`.

use pz_iso_render::outlines::{abgr_to_color, ModelOutlines};
use pz_render::DrawCmd;

const W: u32 = 1280;
const H: u32 = 720;

fn f(v: f32) -> String {
    format!("{v:?}")
}

/// The original's GL calls for the port's outline commands (the subset
/// the oracle's trace is filtered to).
struct Expand {
    lines: Vec<String>,
    /// The colour of the last thickened outline (the player mask's
    /// `u_color`).
    last: [f32; 4],
    after_thicken: bool,
}

impl Expand {
    fn quad(&mut self, tex: u32) {
        let (hw, hh) = (W.next_power_of_two() as f32, H.next_power_of_two() as f32);
        self.lines.push(format!("vbo.texture {tex}"));
        self.lines.push(format!(
            "vbo.quad 0.0,0.0,0.0,{};{},{},{},0.0 z=0.0 color=1.0,1.0,1.0,1.0 ortho={},{},-1.0,1.0",
            f(H as f32 / hh),
            f(W as f32),
            f(H as f32),
            f(W as f32 / hw),
            f(2.0 / W as f32),
            f(-2.0 / H as f32)
        ));
    }

    fn cmds(&mut self, out: &[DrawCmd], solid_color: Option<[f32; 3]>) {
        for (i, c) in out.iter().enumerate() {
            match c {
                DrawCmd::OutlineBegin { clear, .. } => {
                    let col = match out.get(i + 1) {
                        Some(DrawCmd::OutlineModel { color, .. }) => *color,
                        _ if self.after_thicken => [self.last[0], self.last[1], self.last[2]],
                        _ => solid_color.expect("solid colour"),
                    };
                    if self.after_thicken {
                        self.lines.push(format!("glViewport 0 0 {W} {H}"));
                    }
                    for p in ["aim_outline_solid", "aim_outline_solid_static"] {
                        self.lines.push(format!(
                            "{p} u_color={},{},{},1.0",
                            f(col[0]),
                            f(col[1]),
                            f(col[2])
                        ));
                    }
                    self.lines.push(format!("fboA.startDrawing {clear} true"));
                }
                DrawCmd::OutlineEnd => self.lines.push("fboA.endDrawing".into()),
                DrawCmd::OutlineThicken {
                    color,
                    radius,
                    resolution,
                } => {
                    self.last = *color;
                    for (fbo, prog, tex) in
                        [("fboB", "aim_outline_h", 1), ("fboC", "aim_outline_v", 2)]
                    {
                        self.lines.push(format!("{fbo}.startDrawing true true"));
                        self.lines.push(format!("glViewport 0 0 {W} {H}"));
                        self.lines.push(format!(
                            "{prog} u_resolution={},{}",
                            f(resolution[0]),
                            f(resolution[1])
                        ));
                        self.lines.push(format!("{prog} u_radius={}", f(*radius)));
                        self.lines.push(format!(
                            "{prog} u_color={},{},{},{}",
                            f(color[0]),
                            f(color[1]),
                            f(color[2]),
                            f(color[3])
                        ));
                        self.quad(tex);
                        self.lines.push(format!("{fbo}.endDrawing"));
                    }
                    self.after_thicken = true;
                }
                DrawCmd::OutlineBlit => {
                    self.lines.push(format!("glViewport 0 0 {W} {H}"));
                    self.lines.push("aim_outline_blit sampler texture=0".into());
                    self.lines.push("aim_outline_blit sampler mask=1".into());
                    self.lines.push("glBindTexture unit1 1".into());
                    self.quad(3);
                    self.after_thicken = false;
                }
                _ => {}
            }
        }
    }
}

/// The oracle's trace, filtered to the calls the port models.
fn keep(l: &str) -> bool {
    let calls = [
        "begin ->",
        "fbo",
        "glViewport",
        "vbo.texture",
        "vbo.quad",
        "glBindTexture unit1",
        "dirty",
    ];
    (calls.iter().any(|p| l.starts_with(p)) || l.starts_with("aim_outline"))
        && !l.ends_with(".Start")
        && !l.ends_with(".End")
}

#[test]
fn model_outlines_match_the_original() {
    let Some(path) = std::env::var_os("PZ_OUTLINES_ORACLE") else {
        eprintln!("PZ_OUTLINES_ORACLE not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(path).expect("oracle file");
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for l in text.lines() {
        if let Some(name) = l.strip_prefix("== ") {
            sections.push((name.to_string(), Vec::new()));
        } else if let Some((_, s)) = sections.last_mut() {
            if keep(l) {
                s.push(l.to_string());
            }
        } else if let Some(rest) = l.strip_prefix("color ") {
            // color r g b a -> abgr r' g' b' a'
            let (input, output) = rest.split_once(" -> ").unwrap();
            let v: Vec<f32> = input.split(' ').map(|s| s.parse().unwrap()).collect();
            let abgr = pz_render::color_to_abgr(v[0], v[1], v[2], v[3]);
            let c = abgr_to_color(abgr);
            assert_eq!(
                format!("{abgr} {} {} {} {}", f(c[0]), f(c[1]), f(c[2]), f(c[3])),
                output,
                "{l}"
            );
        }
    }
    let red = [1.0, 0.0, 0.0, 1.0];
    let red2 = [1.0, 0.0, 0.0, 0.647_058_84];
    let cyan = [0.0, 1.0, 1.0, 0.65];
    let mut checked = 0;
    for (name, want) in &sections {
        let mut e = Expand {
            lines: Vec::new(),
            last: [0.0; 4],
            after_thicken: false,
        };
        let mut out = Vec::new();
        if let Some(z) = name.strip_prefix("zoom ") {
            let mut o = ModelOutlines::new(W, H, z.parse().unwrap());
            o.start_frame();
            e.lines.push(format!(
                "begin -> {}",
                o.begin_render_outline(red, false, false, &mut out)
            ));
            o.end_frame(&mut out);
            e.cmds(&out, None);
        } else if name == "sequence" {
            let mut o = ModelOutlines::new(W, H, 1.0);
            o.start_frame();
            for (c, behind, mask) in [
                (red, false, false),
                (red, false, false),
                (red2, false, false),
                (cyan, false, false),
                (cyan, true, false),
                (cyan, true, true),
                (red, true, true),
            ] {
                let mut step = Vec::new();
                let r = o.begin_render_outline(c, behind, mask, &mut step);
                e.cmds(&step, None);
                e.lines.push(format!("begin -> {r}"));
            }
            o.end_frame(&mut out);
            e.cmds(&out, None);
            e.lines.push(format!("dirty {}", o.dirty));
            out.clear();
            o.end_frame(&mut out);
            e.cmds(&out, None);
        } else if let Some(b) = name.strip_prefix("player mask behind ") {
            let behind: bool = b.parse().unwrap();
            let mut o = ModelOutlines::new(W, H, 1.0);
            o.start_frame();
            o.render_character_outline(&[], cyan, behind, false, &mut out);
            o.set_player_render_data(Vec::new());
            o.end_frame(&mut out);
            e.cmds(&out, Some([cyan[0], cyan[1], cyan[2]]));
        } else {
            continue;
        }
        assert_eq!(&e.lines, want, "section {name}");
        checked += 1;
    }
    assert_eq!(checked, 12, "sections checked");
}
