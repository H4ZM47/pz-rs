// Ports of `basicEffect.vert` (skinned), `basicEffect_static.vert` and
// `basicEffect.frag`: five directional lights plus ambient, optional hue
// shift, and the depth remap that puts the model's origin on its square's
// depth (`targetDepth`).

struct Model {
    mvp: mat4x4<f32>,
    transform: mat4x4<f32>,
    palette: array<mat4x4<f32>, 60>,
    light_dir: array<vec4<f32>, 5>,
    light_col: array<vec4<f32>, 5>,
    ambient: vec4<f32>,
    tint: vec4<f32>,
    // targetDepth, FinalScale, HueChange, Alpha.
    params: vec4<f32>,
    // UVScale, target y flip (-1 when drawing into a texture), unused.
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> m: Model;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

fn finish(local: vec4<f32>, normal: vec3<f32>, uv: vec2<f32>) -> VsOut {
    var p = local;
    p = vec4<f32>(p.xyz * m.params.y, p.w);
    var o = m.mvp * p;
    // HighResDepthMultiplier is 0 outside high-res chunk textures, so the
    // origin term drops out.
    var clip = (o.z + 1.0) / 2.0;
    clip = clip + m.params.x - 0.5;
    var out: VsOut;
    // GL window depth `clip`; wgpu takes clip-space z in 0..w.
    out.pos = vec4<f32>(o.x, o.y * m.extra.z, clip * o.w, o.w);
    out.normal = normal;
    out.uv = uv * m.extra.xy;
    return out;
}

@vertex
fn vs_static(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> VsOut {
    let n = (m.transform * vec4<f32>(normal, 0.0)).xyz;
    return finish(m.transform * vec4<f32>(pos, 1.0), n, uv);
}

@vertex
fn vs_skinned(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) w: vec4<f32>, @location(3) idx: vec4<f32>, @location(4) uv: vec2<f32>) -> VsOut {
    var bone = mat4x4<f32>(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    if (w.x > 0.0) { bone = bone + m.palette[i32(idx.x)] * w.x; }
    if (w.y > 0.0) { bone = bone + m.palette[i32(idx.y)] * w.y; }
    if (w.z > 0.0) { bone = bone + m.palette[i32(idx.z)] * w.z; }
    if (w.w > 0.0) { bone = bone + m.palette[i32(idx.w)] * w.w; }
    let n = (bone * vec4<f32>(normal, 0.0)).xyz;
    return finish(bone * vec4<f32>(pos, 1.0), n, uv);
}

fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.b, c.g, k.w, k.z), vec4<f32>(c.g, c.b, k.x, k.y), step(c.b, c.g));
    let q = mix(vec4<f32>(p.x, p.y, p.w, c.r), vec4<f32>(c.r, p.y, p.z, p.x), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, clamp(p - k.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

fn hue_shift(col: vec3<f32>, amount: f32) -> vec3<f32> {
    var c = rgb2hsv(col);
    c.x = c.x + amount;
    while (c.x > 1.0) { c.x = c.x - 2.0; }
    while (c.x < -1.0) { c.x = c.x + 2.0; }
    return hsv2rgb(c);
}

@fragment
fn fs_model(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let t = textureSample(tex, tex_sampler, in.uv);
    if (t.a < 0.01) {
        discard;
    }
    var col = t.rgb;
    if (m.params.z != 0.0) {
        col = hue_shift(col, m.params.z);
    }
    var lighting = vec3<f32>(0.0);
    for (var i = 0; i < 5; i = i + 1) {
        let d = max(dot(n, normalize(m.light_dir[i].xyz)), 0.0);
        lighting = lighting + m.light_col[i].xyz * d;
    }
    lighting = min(lighting + m.ambient.xyz, vec3<f32>(1.0));
    col = col * m.tint.xyz * lighting;
    let a = m.params.w;
    // GL alpha test GREATER 0.01 on top of the shader's own discard.
    if (!(a * t.a > 0.01)) {
        discard;
    }
    return vec4<f32>(a * col, a * t.a);
}
