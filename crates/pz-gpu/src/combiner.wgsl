// The SmartShader programs TextureCombiner draws with: blit, hueChange,
// overlayMask, dirtMask, bodyMask, bodyMaskHue, bodyMaskTint, addHole and
// removeHole, with util/math's rgb2hsv/hsv2rgb, util/hueShift and
// util/bodyMask. Their vertex shaders are all ModelViewProjection times
// the position with the texture coordinates passed through (UVScale is
// never set, so 1); the projection is applied on the CPU.

struct Params {
    hue_change: f32,
    r: f32,
    g: f32,
    b: f32,
    intensity: f32,
    blood_dark: f32,
    cutoff_min: f32,
    cutoff_max: f32,
    mask_padding_radius: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Params;
@group(1) @binding(0) var diffuse_t: texture_2d<f32>;
@group(1) @binding(1) var diffuse_s: sampler;
@group(2) @binding(0) var mask_t: texture_2d<f32>;
@group(2) @binding(1) var mask_s: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    return o;
}

// util/math: the overloads of min and clamp it declares behave as the
// built-ins for the values seen here.
fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.b, c.g, K.w, K.z), vec4<f32>(c.g, c.b, K.x, K.y), step(c.b, c.g));
    let q = mix(vec4<f32>(p.x, p.y, p.w, c.r), vec4<f32>(c.r, p.y, p.z, p.x), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

// util/hueShift
fn hue_shift(col: vec3<f32>, amount: f32) -> vec3<f32> {
    var col3 = rgb2hsv(col);
    col3.r += amount;
    while (col3.r > 1.0) {
        col3.r -= 2.0;
    }
    while (col3.r < -1.0) {
        col3.r += 2.0;
    }
    return hsv2rgb(col3);
}

// util/bodyMask
fn sample_mask_alpha(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(mask_t, mask_s, uv, 0.0).a;
}

fn sample_padded_mask_alpha(uv: vec2<f32>, pad_radius: f32) -> f32 {
    var mask_alpha = sample_mask_alpha(uv);
    if (mask_alpha >= 0.99 || pad_radius <= 0.00001) {
        return mask_alpha;
    }
    var offsets = array<vec2<f32>, 8>(
        vec2<f32>(-1.0, 1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0),
    );
    for (var i = 0; i < 8; i++) {
        let offset_uv = clamp(uv + offsets[i] * pad_radius, vec2<f32>(0.0), vec2<f32>(1.0));
        mask_alpha = max(mask_alpha, sample_mask_alpha(offset_uv));
    }
    return mask_alpha;
}

fn perform_padded_body_mask(col: vec4<f32>, uv: vec2<f32>, pad_radius: f32) -> vec4<f32> {
    let mask_alpha = sample_padded_mask_alpha(uv, pad_radius);
    return vec4<f32>(col.xyz, col.a * mask_alpha);
}

@fragment
fn fs_blit(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(diffuse_t, diffuse_s, in.uv);
}

@fragment
fn fs_hue_change(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    var col = col4.xyz;
    col.r *= u.r;
    col.g *= u.g;
    col.b *= u.b;
    col = hue_shift(col, u.hue_change);
    return vec4<f32>(col, col4.a);
}

// overlayMask and dirtMask: blood (red forced to bloodDark) and grime.
fn overlay(col4: vec4<f32>, mask_a: f32, col_in: vec3<f32>) -> vec4<f32> {
    var col = col_in;
    let a = 1.0 - pow(1.0 - col4.a, 3.0);
    let colmask_a = 1.0 - pow(1.0 - mask_a, 3.0);
    var fa = a * colmask_a;
    let intens = clamp(u.intensity, 0.0, 1.0);
    let intensity2 = clamp(u.intensity - 1.0, 0.0, 0.6);
    fa += colmask_a * intensity2;
    fa = clamp(fa, 0.0, 1.0);
    fa = clamp(fa - (1.0 - intens), 0.0, 1.0) / intens;
    col *= fa;
    return vec4<f32>(col, fa);
}

@fragment
fn fs_overlay_mask(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    let colmask = sample_mask_alpha(in.uv);
    var col = col4.xyz;
    col.r = u.blood_dark;
    return overlay(col4, colmask, col);
}

@fragment
fn fs_dirt_mask(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    let colmask = sample_mask_alpha(in.uv);
    return overlay(col4, colmask, col4.xyz);
}

@fragment
fn fs_body_mask(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    return perform_padded_body_mask(col4, in.uv, u.mask_padding_radius);
}

@fragment
fn fs_body_mask_hue(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    var col = perform_padded_body_mask(col4, in.uv, u.mask_padding_radius);
    col = vec4<f32>(hue_shift(col.xyz, u.hue_change), col.a);
    return col;
}

@fragment
fn fs_body_mask_tint(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    var col = perform_padded_body_mask(col4, in.uv, u.mask_padding_radius);
    col.r *= u.r;
    col.g *= u.g;
    col.b *= u.b;
    return col;
}

@fragment
fn fs_add_hole(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    let colmask = sample_mask_alpha(in.uv);
    var maska = (colmask - u.cutoff_min) / (u.cutoff_max - u.cutoff_min);
    maska = clamp(maska, 0.0, 1.0);
    maska = 1.0 - maska;
    let fa = col4.a * maska;
    return vec4<f32>(col4.xyz, fa);
}

@fragment
fn fs_remove_hole(in: VsOut) -> @location(0) vec4<f32> {
    let col4 = textureSample(diffuse_t, diffuse_s, in.uv);
    let colmask = sample_padded_mask_alpha(in.uv, u.mask_padding_radius);
    var maska = (colmask - u.cutoff_min) / (u.cutoff_max - u.cutoff_min);
    maska = clamp(maska, 0.0, 1.0);
    let fa = col4.a * maska;
    return vec4<f32>(col4.xyz, fa);
}
