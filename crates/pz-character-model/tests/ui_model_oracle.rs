//! Parity of `UI3DModel`'s avatar camera with the original
//! `AnimatedModel.UIModelCamera.Begin`: projection, model view and
//! `glViewport` for several viewports, sizes, angles, offsets and the
//! isometric / flipped variants.
//!
//! The oracle harness (outside the repo) runs the original `Begin` and
//! writes `case` / `P` / `MV` / `GL` lines; point `PZ_UI3D_ORACLE` at that
//! file. Without it this test does nothing.

use pz_character_model::ui_model::UiModelCamera;

fn floats(s: &str) -> Vec<f32> {
    s.split_whitespace().map(|v| v.parse().unwrap()).collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn matches_ui_model_camera_oracle() {
    let Ok(path) = std::env::var("PZ_UI3D_ORACLE") else {
        eprintln!("PZ_UI3D_ORACLE not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut n = 0;
    for chunk in lines.chunks(4) {
        let c = floats(chunk[0].strip_prefix("case ").unwrap());
        let p = floats(chunk[1].strip_prefix("P ").unwrap());
        let mv = floats(chunk[2].strip_prefix("MV ").unwrap());
        let gl = chunk[3].strip_prefix("GL ").unwrap();
        let cam = UiModelCamera {
            viewport: [c[0] as i32, c[1] as i32, c[2] as i32, c[3] as i32],
            size_v: c[4],
            anim_player_angle: c[5],
            isometric: c[6] != 0.0,
            flip_y: c[7] != 0.0,
            offset: [c[8], c[9], c[10]],
        };
        let (gp, gmv) = cam.matrices();
        for (what, got, want) in [("P", gp.to_array(), &p), ("MV", gmv.to_array(), &mv)] {
            for i in 0..16 {
                assert!(
                    close(got[i], want[i]),
                    "case {c:?} {what}[{i}]: {} vs {}\n{got:?}\n{want:?}",
                    got[i],
                    want[i]
                );
            }
        }
        let [x, y, w, h] = cam.viewport;
        assert_eq!(gl, format!("glViewport {x} {y} {w} {h}"));
        n += 1;
    }
    assert!(n >= 5, "{n} cases");
}
