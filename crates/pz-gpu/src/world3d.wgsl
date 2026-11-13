// Perspective 3D scene quads (first-person view): position, uv and an
// ABGR vertex colour multiplied with the texel.
//
// Sprites are magnified with a dedicated nearest sampler (group 2) so that
// transparent texels' colour never bleeds into edges. Coverage is an alpha
// test: texels below 0.5 alpha are discarded and the rest are opaque.

struct Scene {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(0) var tex_sampler: sampler;

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
    // The test is on the texel's own alpha: the vertex colour is light.
    let texel = textureSample(tex, tex_sampler, in.uv);
    let t = vec4<f32>(texel.rgb * in.color.rgb, texel.a);
    if (t.a < 0.5) {
        discard;
    }
    return vec4<f32>(t.rgb, 1.0);
}

// Translucent faces (window glass): blended over what is already drawn, no
// depth write; only nearly invisible texels are dropped.
@fragment
fn fs_translucent(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, tex_sampler, in.uv);
    let t = vec4<f32>(texel.rgb * in.color.rgb, texel.a);
    if (t.a < 0.02) {
        discard;
    }
    return t;
}
