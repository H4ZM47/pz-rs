// Ports of `water` / `water_hq` (IsoWater) and `puddles_hq` / `_mq` / `_lq`
// (IsoPuddles, `puddles_common.frag`).
//
// The surfaces' per-vertex attributes travel in the shared vertex layout:
// `uv` holds attributes 2 and 3 (water depth and flow; puddles dirNE and
// dirNW), `uv1` attributes 4 and 5 (water speed and external; puddles
// dirAll and dirNone) and `depth.x` `aFragDepth`.
//
// `gl_FragCoord` is rebuilt from the wgpu fragment position: texture
// targets are drawn y-flipped, so their rows already count from the
// bottom as GL's do; on the main target `surface[0].z` holds the height to
// flip against.

struct Globals {
    mvp: mat4x4<f32>,
    alpha_func: u32,
    alpha_ref: f32,
    clip_z: f32,
    extra: f32,
    screen: array<vec4<f32>, 2>,
    // [0] = (WTime, WReflectionParam, flip height, 0), [1] = WOffset,
    // [2] = WViewport, [3] and [4] = the water parameters or PuddlesParams[0]
    // and [1].
    surface: array<vec4<f32>, 5>,
};

@group(0) @binding(0) var<uniform> g: Globals;
// Unit 0: WaterGroundTex (water) or PuddlesHM (puddles).
@group(1) @binding(0) var ground_tex: texture_2d<f32>;
@group(1) @binding(1) var ground_sampler: sampler;
@group(2) @binding(0) var refl_a: texture_2d<f32>;
@group(2) @binding(1) var refl_a_sampler: sampler;
@group(3) @binding(0) var refl_b: texture_2d<f32>;
@group(3) @binding(1) var refl_b_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) col: vec4<f32>,
    // water: (depth, flow, speed, external); puddles: (NE, NW, All, None)
    @location(1) attr: vec4<f32>,
    // water_hq: the interpolated flow vector.
    @location(2) flow2: vec2<f32>,
    @location(3) frag_depth: f32,
};

@vertex
fn vs_surface(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) uv1: vec2<f32>, @location(4) uv2: vec2<f32>, @location(5) depth: vec2<f32>) -> VsOut {
    var o: VsOut;
    let p = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    o.pos = vec4<f32>(p.x, p.y, 0.0, p.w);
    o.col = col;
    o.attr = vec4<f32>(uv.x, uv.y, uv1.x, uv1.y);
    // water_hq.vert: waterFlow = vec2(sin(isExternal * aFlow), cos(...)).
    o.flow2 = vec2<f32>(sin(uv1.y * uv.y), cos(uv1.y * uv.y));
    o.frag_depth = depth.x;
    return o;
}

fn gl_frag_coord(p: vec4<f32>) -> vec2<f32> {
    let h = g.surface[0].z;
    if (h > 0.0) {
        return vec2<f32>(p.x, h - p.y);
    }
    return p.xy;
}

fn w_time() -> f32 { return g.surface[0].x; }
fn w_reflection_param() -> f32 { return g.surface[0].y; }
fn w_offset() -> vec4<f32> { return g.surface[1]; }
fn w_viewport() -> vec4<f32> { return g.surface[2]; }

// GLSL's smoothstep, also with reversed edges.
fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn glsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

const LD: vec3<f32> = vec3<f32>(0.0, 0.14834045, 0.98893636); // normalize(0, 2.1, 14)

fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return -1.0 + 2.0 * mix(mix(hash(i + vec2<f32>(0.0, 0.0)), hash(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn sphere_map(in_normal: vec3<f32>, ec_position3: vec3<f32>) -> vec2<f32> {
    let u = normalize(ec_position3);
    let r = reflect(u, -in_normal);
    let m = 2.0 * sqrt(r.x * r.x + r.y * r.y + (r.z + 1.0) * (r.z + 1.0));
    return vec2<f32>(r.x / m + 0.5, r.y / m + 0.5);
}

fn water_n(uv: vec2<f32>) -> f32 {
    return noise(uv * 0.8);
}

fn river_rot() -> mat2x2<f32> {
    return mat2x2<f32>(vec2<f32>(0.4, 0.4), vec2<f32>(-0.24, 0.27));
}

// `river` with `levels` octaves (1 in water, 4 in water_hq).
fn river(uv_in: vec2<f32>, levels: f32) -> f32 {
    var uv = uv_in;
    var s = 0.0;
    let r = river_rot();
    for (var i = 1.0; i < levels + 1.0; i = i + 1.0) {
        uv = uv * r;
        s = s + water_n(uv * i * 2.0);
    }
    s = s / (levels + 1.0);
    return s;
}

fn get_sky_map_vec(uv_in: vec2<f32>) -> vec3<f32> {
    let uv = uv_in - 0.5;
    var dir: vec3<f32>;
    dir.x = uv.x;
    dir.z = uv.y;
    dir.y = 0.18 * sqrt(1.0 + dir.x * dir.x - dir.z * dir.z);
    return normalize(dir);
}

fn rain_wave(d: f32) -> f32 {
    return sin(31.0 * d) * ss(-1.0, -0.5, d) * ss(0.0, -0.5, d);
}

// ---- water ---------------------------------------------------------------

fn hash3(p: vec2<f32>) -> vec3<f32> {
    let q = vec3<f32>(dot(p, vec2<f32>(145.5, 361.5)), dot(p, vec2<f32>(276.5, 159.5)), dot(p, vec2<f32>(471.3, 375.4)));
    return fract(sin(q) * 44531.4578);
}

fn bnoise(x: vec2<f32>) -> f32 {
    let fp = floor(x);
    let ff = fract(x);
    var k = 0.0;
    var k2 = 0.0;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var xx = -2; xx <= 2; xx = xx + 1) {
            let gg = vec2<f32>(f32(xx), f32(y));
            let o = hash3(fp + gg);
            let r = gg - ff + o.xy;
            let d = dot(r, r);
            let ww = 1.0 - ss(0.0, 1.414, d);
            k = k + o.z * ww;
            k2 = k2 + ww;
        }
    }
    return k / k2;
}

fn water_map_height(rp: vec3<f32>, water_depth: f32) -> f32 {
    let level = pow(water_depth, 2.0);
    let levelf = bnoise(5.0 * rp.xz) * min(pow(level, 0.3), 1.0) * 0.2 + pow(level, 0.2) * 1.0;
    return rp.y + levelf;
}

fn water_seagrad(uv_in: vec2<f32>, bump: f32, t: f32, levels: f32) -> vec3<f32> {
    let uv = uv_in * 14.0;
    let hc = river(uv, levels);
    let off = vec2<f32>(3.0 / t, 0.0);
    let hh = river(uv + off, levels);
    let hv = river(uv + off.yx, levels);
    let h = normalize(vec3<f32>(bump, hh - hc, 0.0));
    let v = normalize(vec3<f32>(0.0, hv - hc, bump));
    return -normalize(cross(h, v));
}

fn water_rain(uv_in: vec2<f32>, is_external: f32) -> vec3<f32> {
    var uv = uv_in;
    uv.x = uv.x * 50.0;
    uv.y = uv.y * 100.0;
    let fuv = floor(uv);
    let rain = g.surface[3].w;
    var circles = vec2<f32>(0.0);
    for (var k = 0; k < 2; k = k + 1) {
        let i = f32(k) * 0.3;
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let coord = fuv + vec2<f32>(f32(x), f32(y));
                let subcoord = fract(0.6 * w_time() + noise(vec2<f32>(coord.x + i, coord.y - i)));
                let intensive = ss(1.0 - rain, (1.01 - rain) * 1.2, noise(vec2<f32>(coord.y + i, coord.x - i)));
                let v = coord - uv + i;
                let d = length(v) - rain * subcoord * 1.8;
                let f1 = rain_wave(d - 0.001);
                let f2 = rain_wave(d + 0.001);
                circles = circles + intensive * normalize(v) * (f2 - f1) * (1.0 - subcoord) * is_external;
            }
        }
    }
    circles = circles * 100.0;
    return vec3<f32>(circles, (1.0 - dot(circles, circles)) * (1.0 - rain));
}

fn water_image(frag_coord: vec2<f32>, gl_coord: vec2<f32>, in: VsOut, hq: bool) -> vec4<f32> {
    let water_depth = in.attr.x;
    let is_external = in.attr.w;
    let water_speed = is_external * in.attr.z;
    let levels = select(1.0, 4.0, hq);
    let off = w_offset();
    let vp = w_viewport();
    let wind = g.surface[3].xy;
    let wind_speed = g.surface[3].z;

    let uv = vec2<f32>(frag_coord.x * off.z * 0.0008, frag_coord.y * off.w * 0.0008) + off.xy * 0.0008;
    let rain_waves = water_rain(uv, is_external);
    var level = pow(water_depth, 2.0);
    var levelf = min(pow(level, 0.3), 1.0) + level;

    var rp = vec3<f32>(0.0, 15.0, 5.0 - 0.001);
    let rd = vec3<f32>(uv.x, -0.4, uv.y);
    let ro = rp;
    // trace
    let dify = water_map_height(rp, water_depth);
    if (!(dify < 0.0)) {
        rp = rp + rd * dify;
    }
    let n = vec3<f32>(0.0, 1.0, 0.0);
    let t = (0.0 - dot(ro, n)) / dot(n, rd);
    let p = ro + rd * t;

    let tt = 4.0;
    let cycle = glsl_mod(w_time(), tt) / tt;
    var mv = cycle * 2.0;
    if (mv > 1.0) {
        mv = 2.0 - mv;
    }
    let o1 = cycle;
    let o2 = glsl_mod(o1 + 0.5, 1.0);

    var flow: vec2<f32>;
    if (hq) {
        flow = in.flow2 * 0.02;
    } else {
        let wf = is_external * in.attr.y;
        flow = vec2<f32>(sin(wf), cos(wf)) * 0.02;
    }
    let speed = 50.0 * water_speed * 2.0;
    let scale = vec2<f32>(0.35, 0.4);
    let bmp = 0.075;
    let g1 = water_seagrad(scale * p.xz + flow * o1 * speed, bmp, t, levels);
    let g2 = water_seagrad(scale * p.xz + vec2<f32>(0.2, 0.2) + flow * o2 * speed, bmp, t, levels);
    let g3 = water_seagrad(scale * p.xz + vec2<f32>(0.1, 0.2) + flow * o1 * speed * 0.4, bmp, t, levels);
    let g4 = water_seagrad(scale * p.xz + vec2<f32>(0.3, 0.2) + flow * o2 * speed * 0.4, bmp, t, levels);
    let g5 = water_seagrad(scale * p.xz * 2.0 + vec2<f32>(0.2, 0.2) + wind * 0.2, bmp, t, levels);
    let g6 = water_seagrad(scale * p.xz * 1.5 + wind * 0.2, bmp, t, levels);
    var gm = mix(g2, g1, vec3<f32>(mv));
    gm = gm + mix(g4, g3, vec3<f32>(mv));
    gm = gm + mix(vec3<f32>(0.0, 1.0, 0.0), (g5 + g6) * 0.5, vec3<f32>(wind_speed));
    gm = gm + rain_waves.xxy * 0.3;
    gm = gm * 0.7;
    gm = normalize(gm);

    var blue = vec4<f32>(0.0, 95.0, 110.0, 0.0) / 255.0;
    blue = blue * 0.75;
    var wd = dot(gm, LD);
    wd = max(0.0, wd);
    wd = (wd + 0.5) / 1.5;
    wd = clamp(wd, 0.0, 1.0);
    var frag = blue * wd * 0.3;
    frag = vec4<f32>(frag.rgb + rain_waves.z * 0.01, frag.a);

    let pos_eye = get_sky_map_vec(vec2<f32>((gl_coord.x - vp.x) / vp.z, 1.0 - (gl_coord.y - vp.y) / vp.w));
    let ref_tex_coord = sphere_map(gm, pos_eye);
    let tex_ref_a = textureSample(refl_a, refl_a_sampler, ref_tex_coord).rgb;
    let tex_ref_b = textureSample(refl_b, refl_b_sampler, ref_tex_coord).rgb;
    let tex_ref = mix(tex_ref_b, tex_ref_a, vec3<f32>(min(w_reflection_param(), 1.0)));
    frag = vec4<f32>(mix(frag.rgb, tex_ref, vec3<f32>(0.25 * dot(gm, pos_eye))), frag.a);
    frag = vec4<f32>(pow(frag.rgb * 1.7, vec3<f32>(1.0 / 2.2)), frag.a);

    let banklevel = noise(vec2<f32>(42.0 * uv.x + 18.0 * uv.y, 42.0 * uv.y)) + 0.6;
    level = ss(0.0, 1.0, 1.2 - max(banklevel, level) * level * 1.2);
    levelf = max(banklevel, levelf) * levelf;
    let ground = textureSample(ground_tex, ground_sampler, uv * 10.0 + ref_tex_coord * 0.1) * 1.3;
    frag = mix(frag, ground, vec4<f32>(max(0.0, level - 0.3)));
    frag = frag * in.col;
    frag.a = min(levelf, 1.0);
    return frag;
}

struct SurfaceOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

fn water_main(in: VsOut, hq: bool) -> SurfaceOut {
    let gl = gl_frag_coord(in.pos);
    let vp = w_viewport();
    var o: SurfaceOut;
    o.color = water_image(vec2<f32>(gl.x / vp.z, 1.0 - gl.y / vp.w), gl, in, hq);
    o.depth = in.frag_depth;
    return o;
}

@fragment
fn fs_water(in: VsOut) -> SurfaceOut {
    return water_main(in, false);
}

@fragment
fn fs_water_hq(in: VsOut) -> SurfaceOut {
    return water_main(in, true);
}

// ---- puddles (puddles_common.frag) -----------------------------------------

fn func_hash(p_in: vec3<f32>) -> f32 {
    let p = vec3<f32>(dot(p_in, vec3<f32>(17.1, 31.7, 32.6)), dot(p_in, vec3<f32>(29.5, 13.3, 42.6)), 0.0);
    return fract(sin(p.x * 2.1 + 1.1) + sin(p.y * 2.5 + 1.5));
}

fn func_noise(p: vec3<f32>) -> f32 {
    let j = floor(p);
    var f = fract(p);
    f = f * f * (4.5 - 3.5 * f);
    let r1 = mix(func_hash(j), func_hash(j + vec3<f32>(1.0, 0.0, 0.0)), f.x);
    let r2 = mix(func_hash(j + vec3<f32>(0.0, 1.0, 0.0)), func_hash(j + vec3<f32>(1.0, 1.0, 0.0)), f.x);
    let r3 = mix(func_hash(j + vec3<f32>(0.0, 0.0, 1.0)), func_hash(j + vec3<f32>(1.0, 0.0, 1.0)), f.x);
    let r4 = mix(func_hash(j + vec3<f32>(0.0, 1.0, 1.0)), func_hash(j + vec3<f32>(1.0, 1.0, 1.0)), f.x);
    let r12 = mix(r1, r2, f.y);
    let r34 = mix(r3, r4, f.y);
    return mix(r12, r34, f.z);
}

fn perlin_noise(p: vec3<f32>) -> f32 {
    return func_noise(p) * 0.4;
}

fn puddles_river(uv_in: vec2<f32>) -> f32 {
    var uv = uv_in * river_rot();
    var s = water_n(uv * 2.0);
    s = s / 2.0;
    return s;
}

fn puddles_seagrad(uv_in: vec2<f32>, bump: f32, t: f32) -> vec3<f32> {
    let uv = uv_in * 14.0;
    let hc = puddles_river(uv);
    let off = vec2<f32>(2.0 / t, 0.0);
    let hh = puddles_river(uv + off);
    let h = normalize(vec3<f32>(bump, hh - hc, 0.0));
    let v = normalize(vec3<f32>(0.0, hh - hc, bump));
    return -normalize(cross(h, v));
}

fn puddles_rain(uv_in: vec2<f32>) -> vec3<f32> {
    var uv = uv_in;
    uv.x = uv.x * 50.0;
    uv.y = uv.y * 100.0;
    let fuv = floor(uv);
    let rain = g.surface[4].w;
    let coord = fuv + vec2<f32>(0.5, 0.5);
    let subcoord = fract(0.6 * w_time() + func_hash(vec3<f32>(coord.x, 0.0, coord.y)));
    let intensive = ss(1.0 - rain, (1.01 - rain) * 1.2, func_hash(vec3<f32>(coord.y, 0.0, coord.x)));
    let v = coord - uv;
    let d = length(v) - rain * subcoord * 1.8;
    let f1 = rain_wave(d - 0.001);
    let f2 = rain_wave(d + 0.001);
    var circles = intensive * normalize(v) * (f2 - f1) * (1.0 - subcoord);
    circles = circles * 100.0;
    return vec3<f32>(circles, (1.0 - dot(circles, circles)) * (1.0 - rain * 0.5));
}

const PI_P: f32 = 3.1415;

fn get_puddles(uv_in: vec2<f32>, a: vec4<f32>) -> f32 {
    let uv = uv_in * 10.0;
    var s = 1.02 * g.surface[4].z;
    s = s + a.x * sin((uv.x * 1.0 + uv.y * 2.0) * PI_P * 1.0) * cos((uv.x * 1.0 + uv.y * 2.0) * PI_P * 1.0) * 2.0;
    s = s + a.y * sin((uv.x * 1.0 - uv.y * 2.0) * PI_P * 1.0) * cos((uv.x * 1.0 - uv.y * 2.0) * PI_P * 1.0) * 2.0;
    s = s + a.z * 0.3;
    var b = perlin_noise(vec3<f32>(uv.x * 1.0, 0.0, uv.y * 2.0));
    let aa = min(0.7, s * b);
    b = min(0.7, perlin_noise(vec3<f32>(uv.x * 0.7, 1.0, uv.y * 0.7)));
    return aa + b;
}

// The shared tail of mainImage: alpha from the puddle level, colour from
// `dark` and `reflection`.
fn puddles_colour(levelf: f32, dir_none: f32, reflection: vec3<f32>) -> vec4<f32> {
    let muddy = g.surface[4].x;
    let wet_land = g.surface[4].y * dir_none;
    let levelp = g.surface[4].z;
    let alpha_none = ss(0.24, 0.25, levelf * levelp);
    let alpha_dark = min(1.0, wet_land + ss(0.25, 0.34, levelf * levelp));
    let alpha_ambient = ss(0.34, 0.341, levelf * levelp) * (1.0 - muddy);
    let alpha_reflection = ss(0.341, 0.35, levelf * levelp);
    let bluedark = vec3<f32>(0.1, 0.1, 0.2);
    let muddydark = vec3<f32>(0.30, 0.21, 0.18);
    let dark = mix(bluedark, muddydark, vec3<f32>(muddy));
    var c: vec4<f32>;
    c = vec4<f32>(mix(dark, reflection, vec3<f32>(0.3)), mix(0.0, 0.3, alpha_none));
    c.a = mix(c.a, 0.3, alpha_dark);
    c = vec4<f32>(mix(c.rgb, reflection, vec3<f32>(alpha_ambient)), mix(c.a, 0.7, alpha_ambient));
    c = vec4<f32>(mix(c.rgb, mix(dark, reflection, vec3<f32>(0.8 - muddy * 0.4)), vec3<f32>(alpha_reflection)), mix(c.a, 0.5 + muddy * 0.3, alpha_reflection));
    return c;
}

fn puddles_uv(frag_coord: vec2<f32>) -> vec2<f32> {
    let off = w_offset();
    return vec2<f32>(frag_coord.x * off.z * 0.0008, frag_coord.y * off.w * 0.0008) + off.xy * 0.0008 + vec2<f32>(g.surface[3].w * 7.0);
}

// mainImage (hq) and mainImage_MQ: `hq` adds the second wind gradient.
fn puddles_image(frag_coord: vec2<f32>, gl_coord: vec2<f32>, a: vec4<f32>, hq: bool) -> vec4<f32> {
    let uv = puddles_uv(frag_coord);
    let dir_none = 1.0 - a.w;
    let puddles = get_puddles(uv, a);
    let rain_waves = puddles_rain(uv);
    let level = pow(puddles, 2.0) * dir_none;
    let levelf = min(pow(level, 0.3), 1.0) + level;

    var rp = vec3<f32>(0.0, 15.0, 5.0 - 0.001);
    let rd = vec3<f32>(uv.x, -0.4, uv.y);
    let ro = rp;
    let dify = rp.y + 0.5;
    if (!(dify < 0.0)) {
        rp = rp + rd * dify;
    }
    let n = vec3<f32>(0.0, 1.0, 0.0);
    let t = (-dot(ro, n)) / dot(n, rd);
    let p = ro + rd * t;
    let scale = vec2<f32>(0.35, 0.4);
    let wind = g.surface[3].xy;
    let wind_speed = g.surface[3].z;
    var gm = vec3<f32>(0.0, 1.0, 0.0);
    let g6 = puddles_seagrad(scale * p.xz * 1.5 + wind * 0.2, 0.1, t);
    if (hq) {
        let g5 = puddles_seagrad(scale * p.xz * 2.0 + vec2<f32>(0.2, 0.2) + wind * 0.2, 0.1, t);
        gm = gm + mix(vec3<f32>(0.0, 1.0, 0.0), (g5 + g6) * 0.5, vec3<f32>(wind_speed * 1.0));
    } else {
        gm = gm + mix(vec3<f32>(0.0, 1.0, 0.0), g6, vec3<f32>(wind_speed * 1.0));
    }
    gm = gm + rain_waves.xxy * 0.3;
    gm = gm * 0.7;

    let blue = vec4<f32>(96.0, 48.0, 48.0, 0.0) / 255.0;
    var wd = dot(gm, LD);
    wd = max(0.0, wd);
    wd = (wd + 0.5) / 1.5;
    var frag = blue * wd * 0.2;
    frag = vec4<f32>(frag.rgb + rain_waves.z * 0.11, frag.a);

    let vp = w_viewport();
    let pos_eye = get_sky_map_vec(vec2<f32>((gl_coord.x - vp.x) / vp.z, 1.0 - (gl_coord.y - vp.y) / vp.w));
    let ref_tex_coord = sphere_map(gm, pos_eye);
    let tex_ref_a = textureSample(refl_a, refl_a_sampler, ref_tex_coord).rgb;
    let tex_ref_b = textureSample(refl_b, refl_b_sampler, ref_tex_coord).rgb;
    let tex_ref = mix(tex_ref_b, tex_ref_a, vec3<f32>(min(w_reflection_param(), 1.0)));
    let reflection = vec3<f32>(0.3) + mix(frag.rgb, tex_ref, vec3<f32>(0.2 * dot(gm, pos_eye)));
    return puddles_colour(levelf, dir_none, reflection);
}

fn puddles_image_lq(frag_coord: vec2<f32>, a: vec4<f32>) -> vec4<f32> {
    let uv = puddles_uv(frag_coord);
    let dir_none = 1.0 - a.w;
    let level_tex = textureSample(ground_tex, ground_sampler, uv * 0.5).rgb;
    let level = mix(mix(level_tex.b, level_tex.r, a.x), level_tex.g, a.y) * dir_none;
    let levelf = min(pow(level, 0.3), 1.0) + level;
    return puddles_colour(levelf, dir_none, vec3<f32>(0.5, 0.5, 0.55));
}

fn puddles_frag_coord(gl: vec2<f32>) -> vec2<f32> {
    let vp = w_viewport();
    return vec2<f32>((gl.x - vp.x) / vp.z, 1.0 - (gl.y - vp.y) / vp.w);
}

@fragment
fn fs_puddles_hq(in: VsOut) -> SurfaceOut {
    let gl = gl_frag_coord(in.pos);
    var o: SurfaceOut;
    o.color = puddles_image(puddles_frag_coord(gl), gl, in.attr, true) * in.col;
    o.depth = in.frag_depth;
    return o;
}

@fragment
fn fs_puddles_mq(in: VsOut) -> SurfaceOut {
    let gl = gl_frag_coord(in.pos);
    var o: SurfaceOut;
    o.color = puddles_image(puddles_frag_coord(gl), gl, in.attr, false) * in.col;
    o.depth = in.frag_depth;
    return o;
}

@fragment
fn fs_puddles_lq(in: VsOut) -> SurfaceOut {
    let gl = gl_frag_coord(in.pos);
    var o: SurfaceOut;
    o.color = puddles_image_lq(puddles_frag_coord(gl), in.attr) * in.col;
    o.depth = in.frag_depth;
    return o;
}
