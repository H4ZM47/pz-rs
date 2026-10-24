// Ports of `skybox` and `skybox_hires` (SkyBoxShader): the sky as a sphere
// map, rendered into a 512x512 reflection texture for the water and
// puddle shaders.
//
// `ModelManager.RenderSkyBox` draws one quad over ortho(0, 1, 1, 0) whose
// texture coordinates are transposed: vUV1 = (y, 1 - x). In a GL
// framebuffer y = 1 - row / 512 (rows from the bottom, which are also the
// texture's memory rows), so for a fragment in memory row `row` and column
// `col` vUV1 = (1 - row / 512, 1 - col / 512).

struct Sky {
    // (SBTime, CloudCount, CloudSize, Fog)
    a: vec4<f32>,
    sun_light: vec4<f32>,
    sun_colour: vec4<f32>,
    sky_h: vec4<f32>,
    sky_l: vec4<f32>,
    // (CloudLight, Stars, hires, 0)
    b: vec4<f32>,
    wind: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sky: Sky;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

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
    return mix(mix(r1, r2, f.y), mix(r3, r4, f.y), f.z);
}

fn hires() -> bool {
    return sky.b.z > 0.5;
}

fn perlin_noise(p_in: vec3<f32>) -> f32 {
    var p = p_in * 0.5;
    var f = 0.5 * func_noise(p);
    p = p * 3.0;
    f = f + 0.25 * func_noise(p);
    if (hires()) {
        p = p * 3.0;
        f = f + 0.125 * func_noise(p);
        p = p * 3.0;
        f = f + 0.0625 * func_noise(p);
        p = p * 3.0;
        f = f + 0.0312 * func_noise(p);
        p = p * 3.0;
        f = f + 0.0156 * func_noise(p);
    }
    return f;
}

fn map_noise(p: vec3<f32>) -> f32 {
    return perlin_noise(p) - sky.a.y;
}

fn background_stars(rd: vec3<f32>) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    var s = func_noise(rd * 134.0);
    s = s + func_noise(rd * 470.0);
    s = pow(s, 19.0) * 0.00001;
    if (s > 0.5) {
        col = col + vec3<f32>(s) * 0.5 * vec3<f32>(0.95, 0.8, 0.9);
    }
    return col;
}

fn render(pos: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    let sun_light = sky.sun_light.xyz;
    let sun_colour = sky.sun_colour.xyz;
    let cloud_size = sky.a.z;
    let sun_amount = max(dot(rd, sun_light), 0.0) * length(sun_light);
    let sun_zenit = 1.0 - min(pow(max(dot(vec3<f32>(1.0, 0.0, 0.0), sun_light), 0.0), 10.0), 1.0) * length(sun_light);
    let cloud_lower = 1000.0 + cloud_size * 5000.0;
    let cloud_upper = 2000.0 + cloud_size * 5000.0;

    var s = mix(sky.sky_h.xyz, sky.sky_l.xyz, vec3<f32>(1.0 - pow(abs(rd.y), 0.5)));
    s = s + sun_colour * min(pow(sun_amount, 10.0) * 0.75, 1.0);
    s = s + normalize(sun_colour) * min(pow(sun_amount, 1500.0) * 2.0, 1.0);
    s = s + background_stars(rd) * sky.b.y * (1.0 - min(s.x + s.y + s.z, 1.0));

    let beg = (cloud_lower - pos.y) / rd.y;
    let end = (cloud_upper - pos.y) / rd.y;
    let p = vec3<f32>(pos.x + rd.x * beg, cloud_lower, pos.z + rd.z * beg);
    var d = 0.0;
    let steps = select(20, 35, hires());
    let add = (end - beg) / f32(steps);
    let alpha_step = select(0.3, 0.2, hires());
    var clouds = vec4<f32>(0.1, 0.1, 0.1, 0.0);
    var col = vec4<f32>(0.0, 0.0, 0.0, pow(1.0 - rd.y, 30.0) * 0.2);
    for (var i = 0; i < steps; i = i + 1) {
        if (clouds.a >= 1.0) {
            continue;
        }
        let ps = p + rd * d;
        let h = map_noise(ps * 0.001);
        col.a = col.a + max(-h, 0.0) * alpha_step;
        let t = mix(min(pow(sun_amount, 2.0) * 0.75, 1.0) * 0.02, max(0.3 - col.a, 0.0) * 0.04, sun_zenit);
        col = vec4<f32>(mix(vec3<f32>((ps.y - cloud_lower) / (cloud_upper - cloud_lower)) * col.a * sky.b.x, sun_colour, vec3<f32>(t)), col.a);
        clouds = clouds + col * (1.0 - clouds.a);
        d = d + add;
    }
    clouds = vec4<f32>(clouds.xyz + min((1.0 - clouds.a) * pow(sun_amount, 3.0), 1.0), clouds.a);
    s = mix(s, clouds.xyz, vec3<f32>(clouds.a));
    s = mix(s, vec3<f32>(0.5) + normalize(sun_colour) * min(pow(sun_amount, 3.0) * 0.75, 1.0), vec3<f32>(sky.a.w));
    return clamp(s, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn camera_vector(t: f32) -> vec3<f32> {
    return vec3<f32>(3200.0 * sin(0.02 * t), 0.0, 3200.0 * cos(0.015 * t)) + sky.wind.xyz * 1000.0;
}

@fragment
fn fs_sky(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let g_time = sky.a.x * 0.5 - 0.8;
    var uv = vec2<f32>(1.0 - frag.y / 512.0, 1.0 - frag.x / 512.0);
    uv = 2.0 * uv - 1.0;
    let r2 = 1.0 - uv.x * uv.x - uv.y * uv.y;
    // Outside the sphere the original takes sqrt of a negative number and
    // renders NaN, which the clamp turns to black on the GPUs it targets.
    if (r2 < 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    var dir = vec3<f32>(uv.x, sqrt(r2) * 0.2, uv.y);
    var dir4: vec4<f32>;
    if (length(dir) >= 1.0) {
        dir4 = vec4<f32>(0.0, 0.001, 0.999, 0.0);
    } else {
        dir4 = vec4<f32>(normalize(dir), 1.0);
    }
    var col = render(camera_vector(g_time), dir4.xyz);
    col = pow(col, vec3<f32>(0.6));
    return vec4<f32>(col, dir4.a);
}
