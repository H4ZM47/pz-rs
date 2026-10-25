//! 3D model draws (`ModelSlotRenderData`, `Model.DrawChar`/`DrawSolid`):
//! meshes as the model loaders produce them and one draw's uniforms, for
//! the GPU backend to run with ports of `basicEffect`/`basicEffect_static`.
//! The character, animation and item code fills these.

use std::rc::Rc;

use crate::Texture;

/// `VertexBufferObject` layout of a mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshKind {
    /// `ImportedStaticMesh`: position, normal, uv (8 floats per vertex).
    Static,
    /// `ImportedSkinnedMesh`: position, normal, 4 bone weights, 4 bone
    /// indices (as floats), uv (16 floats per vertex).
    Skinned,
}

impl MeshKind {
    /// Floats per vertex.
    pub fn stride(self) -> usize {
        match self {
            MeshKind::Static => 8,
            MeshKind::Skinned => 16,
        }
    }
}

/// A mesh ready to upload: interleaved vertices and triangle indices, in
/// the loaders' winding (reversed at import) and texture orientation.
#[derive(Debug)]
pub struct MeshData {
    /// Identifies the mesh for GPU caching; unique per mesh.
    pub id: u64,
    pub kind: MeshKind,
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

/// `uniform mat4 MatrixPalette[60]`.
pub const MAX_BONES: usize = 60;

/// One `Shader.setLight` light, already in the model's space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModelLight {
    pub direction: [f32; 3],
    pub colour: [f32; 3],
}

/// The fixed-function state of a model draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelPass {
    /// Characters and vehicles: blend (SRC_ALPHA, ONE_MINUS_SRC_ALPHA),
    /// depth LESS with writes, front faces culled (or none).
    Character { cull: bool },
    /// `AnimatedModel.DoRender` (the dead-body atlas and UI renders): as
    /// [`ModelPass::Character`], but `StartCharacter` blends with
    /// `glBlendFuncSeparate(SRC_ALPHA, ONE_MINUS_SRC_ALPHA, ONE,
    /// ONE_MINUS_SRC_ALPHA)` (`UIManager.useUiFbo`, on by default), so the
    /// target's alpha accumulates coverage and layers drawn over an opaque
    /// body leave it opaque.
    AnimatedModel { cull: bool },
    /// Items and placed models: premultiplied blend
    /// (ONE, ONE_MINUS_SRC_ALPHA, ONE_MINUS_DST_ALPHA, ONE), depth LEQUAL
    /// with writes.
    Item { cull: bool },
}

/// One mesh draw with its uniforms.
#[derive(Clone, Debug)]
pub struct ModelDraw {
    pub mesh: Rc<MeshData>,
    pub texture: Option<Texture>,
    /// `ModelViewProjection`, column-major, GL clip space.
    pub mvp: [[f32; 4]; 4],
    /// `transform` for static meshes (column-major).
    pub transform: [[f32; 4]; 4],
    /// `MatrixPalette` for skinned meshes (column-major, `global * invBind`).
    pub palette: Rc<Vec<[[f32; 4]; 4]>>,
    /// `targetDepth`: shifts the projected depth so the model's origin
    /// lands on its square's depth.
    pub target_depth: f32,
    pub final_scale: f32,
    pub uv_scale: [f32; 2],
    pub lights: [ModelLight; 5],
    pub ambient: [f32; 3],
    pub tint: [f32; 3],
    /// `HueChange` (0 = none).
    pub hue: f32,
    pub alpha: f32,
    pub pass: ModelPass,
}
