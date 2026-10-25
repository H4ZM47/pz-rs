// Ports of the vehicle shaders (`vehicle`, `vehicle_multiuv`,
// `vehicle_norandom_multiuv`, each with and without `_noreflect`, and
// `vehiclewheel`; static and skinned vertex stages). The Rust side
// prepends `SHADER` (0 vehicle, 1 multiuv, 2 norandom multiuv, 3 wheel)
// and `REFLECT`.
//
// The enable matrices (`TextureLightsEnables1` ..) arrive as the arrays the
// game uploads with `transpose = true`: GLSL element [c][r] is array
// element r * 4 + c, kept here as row vectors `en[m * 4 + r]`.

struct Vehicle {
    mvp: mat4x4<f32>,
    transform: mat4x4<f32>,
    palette: array<mat4x4<f32>, 60>,
    light_dir: array<vec4<f32>, 5>,
    light_col: array<vec4<f32>, 5>,
    ambient: vec4<f32>,
    tint: vec4<f32>,
    // targetDepth, TextureRustA, target y flip (-1 into a texture), unused.
    params: vec4<f32>,
    // TexturePainColor (Alpha for the wheel shader).
    paint: vec4<f32>,
    // ReflectionParam.
    reflection: vec4<f32>,
    // Uninstall 1/2, LightsEnables 1/2, Damage1Enables 1/2,
    // Damage2Enables 1/2, MatBlood1Enables 1/2, MatBlood2Enables 1/2.
    en: array<vec4<f32>, 48>,
};

@group(0) @binding(0) var<uniform> v: Vehicle;
@group(1) @binding(0) var t_tex0: texture_2d<f32>;
@group(1) @binding(1) var t_rust: texture_2d<f32>;
@group(1) @binding(2) var t_mask: texture_2d<f32>;
@group(1) @binding(3) var t_lights: texture_2d<f32>;
@group(1) @binding(4) var t_d1o: texture_2d<f32>;
@group(1) @binding(5) var t_d1s: texture_2d<f32>;
@group(1) @binding(6) var t_d2o: texture_2d<f32>;
@group(1) @binding(7) var t_d2s: texture_2d<f32>;
@group(1) @binding(8) var t_refa: texture_2d<f32>;
@group(1) @binding(9) var t_refb: texture_2d<f32>;
@group(1) @binding(10) var s_tex0: sampler;
@group(1) @binding(11) var s_rust: sampler;
@group(1) @binding(12) var s_mask: sampler;
@group(1) @binding(13) var s_lights: sampler;
@group(1) @binding(14) var s_d1o: sampler;
@group(1) @binding(15) var s_d1s: sampler;
@group(1) @binding(16) var s_d2o: sampler;
@group(1) @binding(17) var s_d2s: sampler;
@group(1) @binding(18) var s_refa: sampler;
@group(1) @binding(19) var s_refb: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    @location(3) eye: vec4<f32>,
};

fn finish(local: vec4<f32>, normal: vec3<f32>, uv: vec2<f32>, uv1: vec2<f32>) -> VsOut {
    let o = v.mvp * local;
    var clip = (o.z + 1.0) / 2.0;
    clip = clip + v.params.x - 0.5;
    var out: VsOut;
    // GL window depth `clip`; wgpu takes clip-space z in 0..w.
    out.pos = vec4<f32>(o.x, o.y * v.params.z, clip * o.w, o.w);
    out.normal = normal;
    out.uv = uv;
    out.uv1 = uv1;
    out.eye = o - vec4<f32>(-0.2, 0.2, 0.2, 0.0);
    return out;
}

// `<shader>_static.vert`: attribute 2 is the first UV set, 3 the second
// (`vehiclewheel_static` reads its texture coordinates from 3).
@vertex
fn vs_static(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) uv2: vec2<f32>) -> VsOut {
    let n = (v.transform * vec4<f32>(normal, 0.0)).xyz;
    var t0 = uv;
    var t1 = uv2;
    if (SHADER == 3u) {
        t0 = uv2;
    } else if (SHADER == 0u) {
        t1 = uv;
    }
    return finish(v.transform * vec4<f32>(pos, 1.0), n, t0, t1);
}

// `<shader>.vert`: the skinned stage (the second UV set at attribute 5).
@vertex
fn vs_skinned(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) w: vec4<f32>, @location(3) idx: vec4<f32>, @location(4) uv: vec2<f32>, @location(5) uv2: vec2<f32>) -> VsOut {
    var bone = mat4x4<f32>(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    if (w.x > 0.0) { bone = bone + v.palette[i32(idx.x)] * w.x; }
    if (w.y > 0.0) { bone = bone + v.palette[i32(idx.y)] * w.y; }
    if (w.z > 0.0) { bone = bone + v.palette[i32(idx.z)] * w.z; }
    if (w.w > 0.0) { bone = bone + v.palette[i32(idx.w)] * w.w; }
    let n = (bone * vec4<f32>(normal, 0.0)).xyz;
    var t1 = uv2;
    if (SHADER == 0u) {
        t1 = uv;
    }
    return finish(bone * vec4<f32>(pos, 1.0), n, uv, t1);
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

fn desaturate(color: vec3<f32>, amount: f32) -> vec3<f32> {
    let gray = vec3<f32>(dot(vec3<f32>(0.2126, 0.7152, 0.0722), color));
    return mix(color, gray, amount);
}

fn quantise(amount: f32, steps: f32) -> f32 {
    return ceil(amount * steps) / steps;
}

fn glsl_mod1(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x);
}

// `util/sphereMap`.
fn sphere_map(n: vec3<f32>, ec: vec3<f32>) -> vec2<f32> {
    let u = normalize(ec);
    let r = reflect(u, -n);
    let m = 2.0 * sqrt(r.x * r.x + r.y * r.y + (r.z + 1.0) * (r.z + 1.0));
    return vec2<f32>(r.x / m + 0.5, r.y / m + 0.5);
}

// The five lights plus ambient, each dot product quantised to thirds.
fn lighting(n: vec3<f32>) -> vec3<f32> {
    var l = v.ambient.xyz;
    for (var i = 0; i < 5; i = i + 1) {
        let d = quantise(max(dot(n, normalize(v.light_dir[i].xyz)), 0.0), 3.0);
        l = l + v.light_col[i].xyz * d;
    }
    return clamp(l, vec3<f32>(0.0), vec3<f32>(1.0));
}

// `1 - step(0.01, length(mask - zone))`.
fn zone(mask: vec3<f32>, c: vec3<f32>) -> f32 {
    return 1.0 - step(0.01, length(mask - c));
}

// The 27 mask zones: `vehicle_common.frag.h` (multiuv) or `vehicle.frag`'s
// own table (zones 13 to 16 differ). Index `c * 4 + r` is texen1[c][r]
// for zones 1-16 and 16 + `c * 4 + r` is texen2[c][r].
fn zones(mask: vec3<f32>) -> array<f32, 32> {
    var z: array<f32, 32>;
    z[0] = zone(mask, vec3<f32>(1.00, 0.00, 0.00));
    z[1] = zone(mask, vec3<f32>(0.00, 1.00, 0.00));
    z[2] = zone(mask, vec3<f32>(0.00, 1.00, 1.00));
    z[3] = zone(mask, vec3<f32>(1.00, 1.00, 0.00));
    z[4] = zone(mask, vec3<f32>(1.00, 0.00, 1.00));
    z[5] = zone(mask, vec3<f32>(0.00, 0.00, 1.00));
    z[6] = zone(mask, vec3<f32>(0.00, 0.50, 0.50));
    z[7] = zone(mask, vec3<f32>(0.50, 0.50, 0.00));
    z[8] = zone(mask, vec3<f32>(0.50, 0.00, 0.50));
    z[9] = zone(mask, vec3<f32>(0.00, 0.00, 0.50));
    z[10] = zone(mask, vec3<f32>(0.50, 0.00, 0.00));
    z[11] = zone(mask, vec3<f32>(0.00, 0.50, 0.00));
    if (SHADER == 0u) {
        z[12] = zone(mask, vec3<f32>(0.75, 0.00, 0.75));
        z[13] = zone(mask, vec3<f32>(0.00, 0.00, 0.75));
        z[14] = zone(mask, vec3<f32>(0.00, 0.75, 0.75));
        z[15] = zone(mask, vec3<f32>(0.75, 0.75, 0.00));
    } else {
        z[12] = zone(mask, vec3<f32>(0.00, 0.75, 0.75));
        z[13] = zone(mask, vec3<f32>(0.75, 0.75, 0.00));
        z[14] = zone(mask, vec3<f32>(0.75, 0.00, 0.75));
        z[15] = zone(mask, vec3<f32>(0.00, 0.00, 0.75));
    }
    z[16] = zone(mask, vec3<f32>(0.00, 0.00, 0.00));
    z[17] = zone(mask, vec3<f32>(0.25, 0.00, 0.00));
    z[18] = zone(mask, vec3<f32>(0.75, 0.00, 0.00));
    z[19] = zone(mask, vec3<f32>(0.00, 0.75, 0.00));
    z[20] = zone(mask, vec3<f32>(0.00, 0.25, 0.00));
    z[21] = zone(mask, vec3<f32>(0.50, 0.25, 0.00));
    z[22] = zone(mask, vec3<f32>(0.50, 0.75, 0.00));
    z[23] = zone(mask, vec3<f32>(0.75, 0.75, 0.75));
    z[24] = zone(mask, vec3<f32>(0.25, 0.25, 0.25));
    z[25] = zone(mask, vec3<f32>(1.00, 0.00, 0.50));
    z[26] = zone(mask, vec3<f32>(0.00, 1.00, 0.50));
    return z;
}

// `dommat4(texen1, M1) + dommat4(texen2, M2)` for matrix pair `m`.
fn enables(z: array<f32, 32>, m: i32) -> f32 {
    var s = 0.0;
    for (var c = 0; c < 4; c = c + 1) {
        for (var r = 0; r < 4; r = r + 1) {
            s = s + z[c * 4 + r] * v.en[(m * 2) * 4 + r][c];
            s = s + z[16 + c * 4 + r] * v.en[(m * 2 + 1) * 4 + r][c];
        }
    }
    return s;
}

// The paint's hue and the texel's saturation and value moved by it.
fn paint_hsv(rgb: vec3<f32>, paint: vec3<f32>) -> vec3<f32> {
    var f = rgb2hsv(rgb);
    f.x = paint.x;
    f.y = clamp(f.y + paint.y - 0.5, 0.0, 0.9999);
    f.z = clamp(f.z + paint.z - 0.5, 0.0, 0.9999);
    return hsv2rgb(glsl_mod1(f));
}

// `vehicle_common.frag` addDamage.
fn add_damage(col: vec3<f32>, dmg: vec4<f32>, paint: vec3<f32>, alpha: f32) -> vec3<f32> {
    return mix(col, paint_hsv(dmg.xyz, paint), dmg.a * 0.75 * alpha);
}

// `vehicle_common.frag` addBlood.
fn add_blood(blood: vec4<f32>, colmask: vec4<f32>, intensity: f32, alpha2: f32, col_out: vec3<f32>) -> vec3<f32> {
    var col = blood.rgb;
    let a = 1.0 - pow(1.0 - blood.a, 3.0);
    let mask_a = 1.0 - pow(1.0 - colmask.a, 3.0);
    var fa = a * mask_a;
    let intens = clamp(intensity, 0.0, 1.0);
    if (intens < 0.0001) {
        return col_out;
    }
    fa = clamp((fa - (1.0 - intens)) / intens, 0.0, 1.0);
    col = col * fa;
    return mix(col_out, col, fa * blood.a * alpha2);
}

@fragment
fn fs_vehicle(in: VsOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let tex = textureSample(t_tex0, s_tex0, in.uv);
    var col = tex.xyz;
    let light = lighting(normal);
    let tint = desaturate(v.tint.xyz, 0.3);
    if (SHADER == 3u) {
        // vehiclewheel.frag.
        col = col * light * tint;
        if (!(v.paint.w > 0.0)) {
            discard;
        }
        return vec4<f32>(col, v.paint.w);
    }
    let mask = textureSample(t_mask, s_mask, in.uv);
    let rust = textureSample(t_rust, s_rust, in.uv1);
    let lights = textureSample(t_lights, s_lights, in.uv);
    let d1o = textureSample(t_d1o, s_d1o, in.uv1);
    let d1s = textureSample(t_d1s, s_d1s, in.uv1);
    let d2o = textureSample(t_d2o, s_d2o, in.uv1);
    let d2s = textureSample(t_d2s, s_d2s, in.uv1);
    let ref_coord = sphere_map(normal, in.eye.xyz);
    let ref_a = textureSample(t_refa, s_refa, ref_coord).xyz;
    let ref_b = textureSample(t_refb, s_refb, ref_coord).xyz;

    let z = zones(mask.xyz);
    let t1en = step(0.5, enables(z, 1));
    let t2en = step(0.5, enables(z, 2));
    let t3en = step(0.5, enables(z, 3));
    let t4en = step(0.5, enables(z, 0));
    let paint = v.paint.xyz;
    // texen1[1][2] .. texen1[2][3]: the window zones.
    let ref_en = z[6] + z[7] + z[8] + z[9] + z[10] + z[11];
    let tex_ref = mix(ref_b, ref_a, v.reflection.x);

    if (SHADER == 0u) {
        col = mix(col, paint_hsv(col, paint), 1.0 - tex.a);
        if (REFLECT) {
            col = mix(col, tex_ref, ref_en * (0.1 + v.reflection.y * 0.5));
            col = mix(col, tex_ref / 4.0, (1.0 - ref_en) * (0.05 + v.reflection.z * 0.3));
        } else {
            col = mix(col, lights.xyz, lights.a * t1en);
        }
        col = mix(col, rust.xyz, rust.a * v.params.y);
        col = mix(col, paint_hsv(d1s.xyz, paint), d1s.a * t2en);
        col = mix(col, d1o.xyz, d1o.a * t2en);
        col = mix(col, paint_hsv(d2s.xyz, paint), d2s.a * t3en);
        col = mix(col, d2o.xyz, d2o.a * t3en);
        col = mix(col, vec3<f32>(0.2), t4en);
        col = col * light * tint;
        if (REFLECT) {
            col = mix(col, lights.xyz, lights.a * t1en);
        }
    } else {
        let window_alpha = clamp(ref_en, 0.0, 1.0);
        let front_alpha = clamp(z[0] + z[17] + z[18], 0.0, 1.0);
        let tail_alpha = clamp(z[1] + z[19] + z[20] + z[21] + z[22], 0.0, 1.0);
        let no_tint_alpha = clamp(window_alpha + front_alpha + tail_alpha, 0.0, 1.0);
        if (SHADER == 1u) {
            col = mix(col, paint_hsv(col, paint), 1.0 - tex.a);
        }
        if (REFLECT) {
            col = mix(col, tex_ref, ref_en * (0.1 + v.reflection.y * 0.5));
            col = mix(col, tex_ref / 4.0, (1.0 - ref_en) * (0.05 + v.reflection.z * 0.3));
        }
        col = mix(col, rust.xyz, rust.a * v.params.y);
        var paint_color = paint;
        if (SHADER == 2u) {
            paint_color = rgb2hsv(tex.rgb);
        }
        let intensity = enables(z, 4);
        let mask_alpha = step(0.5, enables(z, 5));
        col = add_blood(d2o, d1o, intensity, mask_alpha * window_alpha, col);
        col = add_damage(col, d1s, paint_color, t2en * (1.0 - no_tint_alpha));
        col = mix(col, d1s.xyz, d1s.a * t2en * no_tint_alpha);
        col = add_damage(col, d2s, paint_color, t3en * (1.0 - no_tint_alpha));
        col = mix(col, d2s.xyz, d2s.a * t3en * no_tint_alpha);
        col = add_blood(d2o, d1o, intensity, mask_alpha * (1.0 - window_alpha), col);
        col = mix(col, vec3<f32>(0.2), t4en);
        col = col * light * tint;
        col = mix(col, lights.xyz, lights.a * t1en);
    }
    // glAlphaFunc(GL_GREATER, 0).
    if (!(v.paint.w > 0.0)) {
        discard;
    }
    return vec4<f32>(col, v.paint.w);
}
