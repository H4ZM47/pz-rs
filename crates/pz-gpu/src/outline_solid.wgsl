// Ports of the outline shaders `ModelOutlines` and `ModelSlotRenderData`
// use: `aim_outline_solid` (skinned) and `aim_outline_solid_static` with
// `aim_outline_solid.frag` for the mask, `aim_outline_h`/`aim_outline_v`
// for the thickening and `aim_outline_blit` for the final draw. This
// module has the mask draw; outline_pass.wgsl the full-viewport passes.

// The `Model` block of model.wgsl (same buffer layout).
struct Model {
    mvp: mat4x4<f32>,
    transform: mat4x4<f32>,
    palette: array<mat4x4<f32>, 60>,
    light_dir: array<vec4<f32>, 5>,
    light_col: array<vec4<f32>, 5>,
    ambient: vec4<f32>,
    // u_color.
    tint: vec4<f32>,
    // targetDepth, FinalScale, HueChange, Alpha.
    params: vec4<f32>,
    // UVScale, target y flip (-1 when drawing into a texture), unused.
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> m: Model;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct SolidOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

fn solid_finish(o: vec4<f32>, uv: vec2<f32>) -> SolidOut {
    var out: SolidOut;
    // GL clip z (-w..w) to wgpu's (0..w); nothing is depth tested.
    out.pos = vec4<f32>(o.x, o.y * m.extra.z, (o.z + o.w) * 0.5, o.w);
    out.uv = uv;
    return out;
}

// aim_outline_solid_static.vert: ModelViewProjection * transform * position.
@vertex
fn vs_solid_static(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> SolidOut {
    return solid_finish(m.mvp * (m.transform * vec4<f32>(pos, 1.0)), uv);
}

// aim_outline_solid.vert: the skinned position times FinalScale.
@vertex
fn vs_solid_skinned(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) w: vec4<f32>, @location(3) idx: vec4<f32>, @location(4) uv: vec2<f32>) -> SolidOut {
    var bone = mat4x4<f32>(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    if (w.x > 0.0) { bone = bone + m.palette[i32(idx.x)] * w.x; }
    if (w.y > 0.0) { bone = bone + m.palette[i32(idx.y)] * w.y; }
    if (w.z > 0.0) { bone = bone + m.palette[i32(idx.z)] * w.z; }
    if (w.w > 0.0) { bone = bone + m.palette[i32(idx.w)] * w.w; }
    var p = bone * vec4<f32>(pos, 1.0);
    p = vec4<f32>(p.xyz * m.params.y, p.w);
    return solid_finish(m.mvp * p, uv);
}

// aim_outline_solid.frag, with the alpha test (GREATER 0) the draw
// enables.
@fragment
fn fs_solid(in: SolidOut) -> @location(0) vec4<f32> {
    let alpha = step(0.5, textureSample(tex, tex_sampler, in.uv).a);
    let a = m.tint.a * alpha;
    if (!(a > 0.0)) {
        discard;
    }
    return vec4<f32>(m.tint.rgb, a);
}
