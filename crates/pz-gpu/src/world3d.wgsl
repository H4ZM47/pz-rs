// Perspective 3D scene quads (first-person view): position, uv and an
// ABGR vertex colour multiplied with the texel.

struct Scene {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    let c = scene.view_proj * vec4<f32>(pos, 1.0);
    var out: VsOut;
    // GL clip depth -1..1 to wgpu 0..1.
    out.pos = vec4<f32>(c.x, c.y, (c.z + c.w) * 0.5, c.w);
    out.uv = uv;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, tex_sampler, in.uv) * in.color;
    if (t.a < 0.01) {
        discard;
    }
    return t;
}
