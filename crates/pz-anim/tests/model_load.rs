//! A game mesh and animation loaded through pz-model (`resolve_model_file`,
//! `load_mesh`, `load_animation`) and played: the skinned body stays a
//! sane size every step.
//!
//! Reads the installed game at runtime: point `PZ_GAME_DIR` at the folder
//! holding `media/`. Without it (or its files) this test does nothing.

use std::path::PathBuf;
use std::rc::Rc;

use pz_anim::AnimationPlayer;
use pz_model::assets::{resolve_model_file, AssetKind, MediaIndex};
use pz_model::loader::{load_animation, load_mesh, MeshParams};

#[test]
fn loaded_model_animates() {
    let Some(game) = std::env::var_os("PZ_GAME_DIR")
        .map(PathBuf::from)
        .filter(|g| g.join("media").is_dir())
    else {
        eprintln!("PZ_GAME_DIR not set or has no media/; skipping");
        return;
    };
    let media = MediaIndex::scan(&game).unwrap();
    let (Some((mesh_path, mesh_ty)), Some((anim_path, anim_ty))) = (
        resolve_model_file(&media, "Skinned/MaleBody", AssetKind::Mesh),
        resolve_model_file(&media, "Bob/Bob_Walk", AssetKind::Animation),
    ) else {
        eprintln!(
            "MaleBody / Bob_Walk not found under {}; skipping",
            game.display()
        );
        return;
    };
    let mesh = load_mesh(
        &std::fs::read(mesh_path).unwrap(),
        mesh_ty.import_format().unwrap(),
        &MeshParams::default(),
    )
    .unwrap();
    let mut sd = mesh.skeleton.clone().unwrap();
    let clips = load_animation(
        &std::fs::read(anim_path).unwrap(),
        anim_ty.import_format().unwrap(),
        Some(&sd),
    )
    .unwrap();
    let name = clips.keys().next().unwrap().clone();
    sd.animation_clips.extend(clips);
    let mut p = AnimationPlayer::new(Rc::new(sd));
    let t = p.play(&name, true, false, -1.0).unwrap();
    p.track_mut(t).unwrap().set_blend_weight(1.0);
    let skm = mesh.skinned_mesh.unwrap();
    let s = skm.stride();
    for step in 0..4 {
        p.update(0.25, None);
        let pal = p.palette();
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in skm.vertices.chunks_exact(s) {
            let mut o = [0.0f32; 3];
            for k in 0..4 {
                let w = v[6 + k];
                if w == 0.0 {
                    continue;
                }
                let mat = pal[v[10 + k] as usize];
                for (r, o) in o.iter_mut().enumerate() {
                    *o += w * (mat[0][r] * v[0] + mat[1][r] * v[1] + mat[2][r] * v[2] + mat[3][r]);
                }
            }
            for r in 0..3 {
                mn[r] = mn[r].min(o[r]);
                mx[r] = mx[r].max(o[r]);
            }
        }
        eprintln!("step {step}: skinned bounds {mn:?} {mx:?}");
        assert!(mx
            .iter()
            .zip(mn.iter())
            .all(|(a, b)| (a - b).is_finite() && (a - b) < 3.0));
    }
}
