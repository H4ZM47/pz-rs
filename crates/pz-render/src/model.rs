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
    /// A vehicle's static mesh: position, normal, the first and second UV
    /// sets (10 floats per vertex; the second set is zero when the mesh
    /// has one, as GL's disabled attribute array reads).
    StaticUv2,
    /// A vehicle's skinned mesh: the skinned layout plus the second UV set
    /// (18 floats per vertex).
    SkinnedUv2,
}

impl MeshKind {
    /// Floats per vertex.
    pub fn stride(self) -> usize {
        match self {
            MeshKind::Static => 8,
            MeshKind::Skinned => 16,
            MeshKind::StaticUv2 => 10,
            MeshKind::SkinnedUv2 => 18,
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
    /// [`ModelPass::AnimatedModel`] with `setCullFace(GL_BACK)`, as
    /// `FBORenderCorpses` draws mannequins into chunk textures.
    AnimatedModelCullBack,
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

/// The vehicle shader a vehicle mesh draws with (the model script's
/// `shader`, after `ModelManager.modifyShaderName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VehicleShader {
    /// `vehicle`: one UV set for every texture; the older damage blend.
    Vehicle,
    /// `vehicle_multiuv`: rust, damage and blood on the second UV set;
    /// the paint colour fills the skin's transparent texels.
    MultiUv,
    /// `vehicle_norandom_multiuv`: as `MultiUv` without the paint fill;
    /// the damage tint takes the skin's own hue.
    NoRandomMultiUv,
    /// `vehiclewheel`: the texture lit and tinted, alpha from `Alpha`.
    Wheel,
}

impl VehicleShader {
    /// The script's shader name (with or without `_noreflect`); `None`
    /// for non-vehicle shaders. The second value says whether the
    /// `_noreflect` program is used.
    pub fn from_name(name: &str) -> Option<(VehicleShader, bool)> {
        let (base, noreflect) = match name.strip_suffix("_noreflect") {
            Some(b) => (b, true),
            None => (name, false),
        };
        let s = match base {
            "vehicle" => VehicleShader::Vehicle,
            "vehicle_multiuv" => VehicleShader::MultiUv,
            "vehicle_norandom_multiuv" => VehicleShader::NoRandomMultiUv,
            "vehiclewheel" => VehicleShader::Wheel,
            _ => return None,
        };
        Some((s, noreflect))
    }

    /// `Shader.isVehicleShader`: has the `TextureRust` uniform.
    pub fn is_vehicle_shader(self) -> bool {
        self != VehicleShader::Wheel
    }
}

/// The textures a vehicle shader samples (`VehicleModelInstance`), in
/// texture-unit order: `Texture0` (or `Texture` for the wheel shader),
/// `TextureRust`, `TextureMask`, `TextureLights`, `TextureDamage1Overlay`,
/// `TextureDamage1Shell`, `TextureDamage2Overlay`, `TextureDamage2Shell`,
/// `TextureReflectionA`, `TextureReflectionB`.
pub type VehicleTextures = [Option<Texture>; 10];

/// The uniforms `Model.DrawVehicle` sets from a `VehicleModelInstance`.
/// The enable matrices are the instance's `float[16]` arrays as uploaded
/// (`glUniformMatrix4fv(.., transpose = true, ..)`: element `[c][r]` of
/// the GLSL matrix is `array[r * 4 + c]`).
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleUniforms {
    pub texture_uninstall: [[f32; 16]; 2],
    pub texture_lights_enables: [[f32; 16]; 2],
    pub texture_damage1_enables: [[f32; 16]; 2],
    pub texture_damage2_enables: [[f32; 16]; 2],
    pub matrix_blood1_enables: [[f32; 16]; 2],
    pub matrix_blood2_enables: [[f32; 16]; 2],
    pub texture_rust_a: f32,
    /// `TexturePainColor`: the paint hue, saturation and value, and the
    /// slot's alpha.
    pub paint_color: [f32; 4],
    /// `ReflectionParam`: the sky box's texture shift, `refWindows`,
    /// `refBody`.
    pub reflection: [f32; 3],
}

impl Default for VehicleUniforms {
    fn default() -> Self {
        VehicleUniforms {
            texture_uninstall: [[0.0; 16]; 2],
            texture_lights_enables: [[0.0; 16]; 2],
            texture_damage1_enables: [[0.0; 16]; 2],
            texture_damage2_enables: [[0.0; 16]; 2],
            matrix_blood1_enables: [[0.0; 16]; 2],
            matrix_blood2_enables: [[0.0; 16]; 2],
            texture_rust_a: 0.0,
            paint_color: [0.0, 0.5, 0.5, 1.0],
            reflection: [0.0, 0.5, 0.4],
        }
    }
}

/// One mesh of a vehicle (`Model.DrawVehicle`): the base model draw (its
/// matrices, lights, ambient, tint, `targetDepth` and alpha; `texture` is
/// unused) plus the vehicle shader, its textures and uniforms. Drawn with
/// blending (SRC_ALPHA, ONE_MINUS_SRC_ALPHA), depth LESS with writes,
/// alpha test GREATER 0 and front faces culled (back faces with
/// `invertX`).
#[derive(Clone, Debug)]
pub struct VehicleDraw {
    pub base: ModelDraw,
    pub shader: VehicleShader,
    /// The `_noreflect` program (`bPerfReflections` off).
    pub noreflect: bool,
    pub invert_x: bool,
    pub textures: VehicleTextures,
    pub uniforms: VehicleUniforms,
}
