// The full-viewport passes of `ModelOutlines.endFrame`: `aim_outline_h`,
// `aim_outline_v` and `aim_outline_blit`. The original's `renderTexture`
// quad covers the viewport with texture coordinates addressing the same
// texels of a texture as large as the target, so each fragment samples its
// own position (`position / textureDimensions`).

// The thickening and blit passes.
struct Pass {
    // u_color.
    color: vec4<f32>,
    // u_resolution, u_radius, unused.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> pass_u: Pass;
@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var src_sampler: sampler;
@group(2) @binding(0) var mask: texture_2d<f32>;
@group(2) @binding(1) var mask_sampler: sampler;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn own_uv(frag: vec4<f32>) -> vec2<f32> {
    return frag.xy / vec2<f32>(textureDimensions(src));
}

fn tap(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(src, src_sampler, uv, 0.0).a;
}

// aim_outline_h.frag. The sum is clamped as a fixed-point target clamps
// the fragment colour before blending.
@fragment
fn fs_thicken_h(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = own_uv(frag);
    let blur = pass_u.params.z / pass_u.params.x;
    var sum = tap(uv);
    sum = sum + tap(vec2<f32>(uv.x - 4.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x - 3.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x - 2.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x - 1.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x + 1.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x + 2.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x + 3.0 * blur, uv.y));
    sum = sum + tap(vec2<f32>(uv.x + 4.0 * blur, uv.y));
    return clamp(vec4<f32>(pass_u.color.rgb, sum * pass_u.color.a), vec4<f32>(0.0), vec4<f32>(1.0));
}

// aim_outline_v.frag.
@fragment
fn fs_thicken_v(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = own_uv(frag);
    let blur = pass_u.params.z / pass_u.params.y;
    var sum = tap(uv);
    sum = sum + tap(vec2<f32>(uv.x, uv.y - 4.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y - 3.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y - 2.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y - 1.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y + 1.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y + 2.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y + 3.0 * blur));
    sum = sum + tap(vec2<f32>(uv.x, uv.y + 4.0 * blur));
    return clamp(vec4<f32>(pass_u.color.rgb, sum * pass_u.color.a), vec4<f32>(0.0), vec4<f32>(1.0));
}

// aim_outline_blit.frag: `texture` is the thickened outline, `mask` the
// solid mask.
@fragment
fn fs_blit(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = own_uv(frag);
    let alpha = 1.0 - textureSampleLevel(mask, mask_sampler, uv, 0.0).a;
    return textureSampleLevel(src, src_sampler, uv, 0.0) * vec4<f32>(1.0, 1.0, 1.0, alpha);
}
