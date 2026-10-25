//! Parity of `FBORenderChunkCamera`'s projection and model-view with the
//! original class. The oracle harness (outside the repo) builds the camera
//! for a few chunk textures and model positions and writes `chunkcam.txt`;
//! point `PZ_CHUNKCAM_ORACLE_DIR` at it. Without it this test does nothing.

use pz_iso_render::chunk_camera::ChunkCamera;
use pz_iso_render::ChunkTexture;

#[test]
fn chunk_camera_matches_the_original() {
    let Some(dir) = std::env::var_os("PZ_CHUNKCAM_ORACLE_DIR") else {
        eprintln!("PZ_CHUNKCAM_ORACLE_DIR not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("chunkcam.txt")).unwrap();
    // The harness' inputs: chunk wx, wy, min level, max level, w, h; and
    // the model's x, y, z, angle.
    let cases = [
        (10, 12, 0, 0, 512, 1024),
        (3, -2, 0, 1, 512, 1024),
        (7, 7, 2, 3, 512, 512),
    ];
    let pts = [
        [84.0f32, 100.0, 0.0, 0.0],
        [26.3, -11.2, 0.6, 1.3],
        [60.5, 59.1, 2.25, -2.0],
    ];
    let lines: Vec<&str> = text.lines().collect();
    for (i, (c, p)) in cases.iter().zip(pts).enumerate() {
        let nums = |l: &str| -> Vec<f32> {
            l.split_whitespace()
                .skip(1)
                .map(|s| s.parse().unwrap())
                .collect()
        };
        let head: Vec<&str> = lines[i * 3].split_whitespace().collect();
        let (bottom, top): (i32, i32) = (head[3].parse().unwrap(), head[5].parse().unwrap());
        let mut cam = ChunkCamera::new(ChunkTexture {
            wx: c.0,
            wy: c.1,
            min_level: c.2,
            top_level: top,
        });
        cam.width = c.4;
        cam.height = c.5;
        assert_eq!(cam.bottom, bottom, "case {i}");
        for (got, want) in [
            (cam.projection(), nums(lines[i * 3 + 1])),
            (
                cam.model_view(p[0], p[1], p[2], p[3]),
                nums(lines[i * 3 + 2]),
            ),
        ] {
            let g = got.to_array();
            for k in 0..16 {
                assert!(
                    (g[k] - want[k]).abs() <= 1e-5 * (1.0 + want[k].abs()),
                    "case {i} [{k}]: {g:?} vs {want:?}"
                );
            }
        }
    }
}
