//! A tiny DirectX text file built in the test, run through the same path
//! `FileTask_LoadMesh.loadX` and `FileTask_LoadAnimation.loadX` take.

use pz_model::import::{self, Format, Steps};
use pz_model::loader::{load_animation, load_mesh, MeshParams};

const IDENT: &str = "1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0;;";

fn x_file() -> String {
    format!(
        r#"xof 0303txt 0032
template Frame {{
 <3d82ab46-62da-11cf-ab39-0020af71e433>
 [...]
}}
AnimTicksPerSecond {{
 4800;
}}
Frame Dummy01 {{
 FrameTransformMatrix {{
  {IDENT}
 }}
 Frame Bip01 {{
  FrameTransformMatrix {{
   1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.5,0.0,1.0;;
  }}
 }}
}}
Frame Body {{
 FrameTransformMatrix {{
  {IDENT}
 }}
 Mesh Body {{
  4;
  0.0;0.0;0.0;,
  1.0;0.0;0.0;,
  1.0;1.0;0.0;,
  0.0;1.0;0.0;;
  2;
  3;0,1,2;,
  3;0,2,3;;
  MeshNormals {{
   1;
   0.0;0.0;1.0;;
   2;
   3;0,0,0;,
   3;0,0,0;;
  }}
  MeshTextureCoords {{
   4;
   0.0;0.25;,
   1.0;0.25;,
   1.0;0.75;,
   0.0;0.75;;
  }}
  MeshMaterialList {{
   1;
   2;
   0,
   0;;
   Material Skin {{
    1.0;1.0;1.0;1.0;;
    0.0;
    0.0;0.0;0.0;;
    0.0;0.0;0.0;;
    TextureFilename {{
     "body.png";
    }}
   }}
  }}
  SkinWeights {{
   "Dummy01";
   0;
   ;
   ;
   {IDENT}
  }}
  SkinWeights {{
   "Bip01";
   4;
   0,
   1,
   2,
   3;
   1.0,
   1.0,
   0.5,
   0.5;
   1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,-0.5,0.0,1.0;;
  }}
 }}
}}
AnimationSet Armature|Walk {{
 Animation {{
  {{ Bip01 }}
  AnimationKey {{
   2;
   2;
   0;3;0.0,0.5,0.0;;,
   4800;3;0.0,1.0,0.0;;;
  }}
  AnimationKey {{
   0;
   1;
   0;4;1.0,0.0,0.0,0.0;;;
  }}
 }}
}}
"#
    )
}

#[test]
fn raw_scene_matches_xfile_importer() {
    let data = x_file();
    let scene = import::import(data.as_bytes(), Format::X, Steps::mesh_default()).unwrap();
    // Dummy01 and Body become siblings under a dummy root.
    assert_eq!(scene.nodes[scene.root].name, "$dummy_root");
    assert_eq!(scene.meshes.len(), 1);
    let m = &scene.meshes[0];
    assert_eq!(m.name, "Body");
    // 6 face corners joined back to 4 vertices.
    assert_eq!(m.vertices.len(), 4);
    // The bone without weights is dropped by the importer.
    assert_eq!(m.bones.len(), 1);
    assert_eq!(m.bones[0].name, "Bip01");
}

#[test]
fn mesh_vertices_winding_and_skeleton() {
    let data = x_file();
    let p = load_mesh(data.as_bytes(), Format::X, &MeshParams::default()).unwrap();
    let skm = p.skinned_mesh.expect("skinned mesh");
    assert_eq!(skm.num_uvs, 1);
    assert_eq!(skm.stride(), 16);
    assert_eq!(skm.vertices.len(), 4 * 16);
    // The importer flips the winding and the game reverses it again, so
    // triangles come out in file order.
    assert_eq!(skm.elements, vec![0, 1, 2, 0, 2, 3]);
    let v = |i: usize| &skm.vertices[i * 16..(i + 1) * 16];
    assert_eq!(&v(2)[0..3], &[1.0, 1.0, 0.0]);
    assert_eq!(&v(2)[3..6], &[0.0, 0.0, 1.0]);
    // Weights normalised, bone index 1 (Bip01).
    assert_eq!(&v(2)[6..10], &[1.0, 0.0, 0.0, 0.0]);
    assert_eq!(&v(2)[10..14], &[1.0, 0.0, 0.0, 0.0]);
    // V is flipped twice (importer, then ImportedSkinnedMesh).
    assert_eq!(&v(2)[14..16], &[1.0, 0.75]);

    let sk = p.skeleton.expect("skeleton");
    assert_eq!(sk.bone_indices["Dummy01"], 0);
    assert_eq!(sk.bone_indices["Bip01"], 1);
    assert_eq!(sk.skeleton_hierarchy, vec![-1, 0]);
    // bind = inverse(inverse(offset) * inverse(parent offset)); Dummy01
    // has no bone in the mesh, so its offset is identity.
    assert_eq!(sk.bind_pose[1][3], [0.0, -0.5, 0.0, 1.0]);
    assert_eq!(sk.inverse_bind_pose[1][3], [0.0, 0.5, 0.0, 1.0]);
    assert_eq!(sk.bind_pose[0], pz_model::import::aimath::AiMat4::IDENTITY.m);
}

#[test]
fn animation_keys_and_clip_name() {
    let data = x_file();
    let mesh = load_mesh(data.as_bytes(), Format::X, &MeshParams::default()).unwrap();
    let clips = load_animation(data.as_bytes(), Format::X, mesh.skeleton.as_ref()).unwrap();
    // "Armature|Walk" keeps the part after '|'.
    let clip = &clips["Walk"];
    assert_eq!(clip.duration, 1.0);
    assert!(clip.keep_last_frame);
    assert_eq!(clip.keyframes.len(), 2);
    let k0 = &clip.keyframes[0];
    assert_eq!((k0.bone, k0.bone_name.as_str(), k0.time), (1, "Bip01", 0.0));
    assert_eq!(k0.position, [0.0, 0.5, 0.0]);
    // X rotation keys are inverted on import (w negated).
    assert_eq!((k0.rotation.x, k0.rotation.y, k0.rotation.z, k0.rotation.w), (0.0, 0.0, 0.0, -1.0));
    let k1 = &clip.keyframes[1];
    assert_eq!(k1.time, 1.0);
    assert_eq!(k1.position, [0.0, 1.0, 0.0]);
    // Only one rotation key: held.
    assert_eq!(k1.rotation.w, -1.0);
}
