// Port of the `screen` shader `MultiTextureFBO2.render` draws the
// offscreen world buffer with (WeatherShader): bicubic resampling, the
// world grade (desaturate, contrast) and film grain. Night-vision, drunk,
// blur and search-mode branches are not ported.

struct Globals {
    mvp: mat4x4<f32>,
    alpha_func: u32,
    alpha_ref: f32,
    clip_z: f32,
    extra: f32,
    // DesaturationVal, timer, bgl_RenderedTextureWidth/Height.
    screen0: vec4<f32>,
    // TextureSize, unused, unused.
    screen1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_screen(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.pos = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    o.pos.z = 0.5;
    o.uv = uv;
    return o;
}

const PERM_TEX_UNIT: f32 = 1.0 / 256.0;
const PERM_TEX_UNIT_HALF: f32 = 0.5 / 256.0;
const GRAIN_AMOUNT: f32 = 0.0003;
const AVG_LUMIN: vec3<f32> = vec3<f32>(0.4, 0.4, 0.4);

fn rnm(tc: vec2<f32>) -> vec4<f32> {
    let timer = g.screen0.y;
    let noise = sin(dot(tc + vec2<f32>(timer, timer), vec2<f32>(12.9898, 78.233))) * 43758.5453;
    return vec4<f32>(fract(noise) * 2.0 - 1.0, fract(noise * 1.2154) * 2.0 - 1.0, fract(noise * 1.3453) * 2.0 - 1.0, fract(noise * 1.3647) * 2.0 - 1.0);
}

fn fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn pnoise3d(p: vec3<f32>) -> f32 {
    let pi = PERM_TEX_UNIT * floor(p) + PERM_TEX_UNIT_HALF;
    let pf = fract(p);
    let perm00 = rnm(pi.xy).a;
    let n000 = dot(rnm(vec2<f32>(perm00, pi.z)).rgb * 4.0 - 1.0, pf);
    let n001 = dot(rnm(vec2<f32>(perm00, pi.z + PERM_TEX_UNIT)).rgb * 4.0 - 1.0, pf - vec3<f32>(0.0, 0.0, 1.0));
    let perm01 = rnm(pi.xy + vec2<f32>(0.0, PERM_TEX_UNIT)).a;
    let n010 = dot(rnm(vec2<f32>(perm01, pi.z)).rgb * 4.0 - 1.0, pf - vec3<f32>(0.0, 1.0, 0.0));
    let n011 = dot(rnm(vec2<f32>(perm01, pi.z + PERM_TEX_UNIT)).rgb * 4.0 - 1.0, pf - vec3<f32>(0.0, 1.0, 1.0));
    let perm10 = rnm(pi.xy + vec2<f32>(PERM_TEX_UNIT, 0.0)).a;
    let n100 = dot(rnm(vec2<f32>(perm10, pi.z)).rgb * 4.0 - 1.0, pf - vec3<f32>(1.0, 0.0, 0.0));
    let n101 = dot(rnm(vec2<f32>(perm10, pi.z + PERM_TEX_UNIT)).rgb * 4.0 - 1.0, pf - vec3<f32>(1.0, 0.0, 1.0));
    let perm11 = rnm(pi.xy + vec2<f32>(PERM_TEX_UNIT, PERM_TEX_UNIT)).a;
    let n110 = dot(rnm(vec2<f32>(perm11, pi.z)).rgb * 4.0 - 1.0, pf - vec3<f32>(1.0, 1.0, 0.0));
    let n111 = dot(rnm(vec2<f32>(perm11, pi.z + PERM_TEX_UNIT)).rgb * 4.0 - 1.0, pf - vec3<f32>(1.0, 1.0, 1.0));
    let n_x = mix(vec4<f32>(n000, n001, n010, n011), vec4<f32>(n100, n101, n110, n111), fade(pf.x));
    let n_xy = mix(n_x.xy, n_x.zw, fade(pf.y));
    return mix(n_xy.x, n_xy.y, fade(pf.z));
}

fn coord_rot(tc: vec2<f32>, angle: f32) -> vec2<f32> {
    let aspect = g.screen0.z / g.screen0.w;
    var rot_x = ((tc.x * 2.0 - 1.0) * aspect * cos(angle)) - ((tc.y * 2.0 - 1.0) * sin(angle));
    var rot_y = ((tc.y * 2.0 - 1.0) * cos(angle)) + ((tc.x * 2.0 - 1.0) * aspect * sin(angle));
    rot_x = (rot_x / aspect) * 0.5 + 0.5;
    rot_y = rot_y * 0.5 + 0.5;
    return vec2<f32>(rot_x, rot_y);
}

fn contrast(color: vec3<f32>, amount: f32) -> vec3<f32> {
    return (color - AVG_LUMIN) * amount + AVG_LUMIN;
}

fn desaturate(color: vec3<f32>, amount: f32) -> vec3<f32> {
    let gray = vec3<f32>(dot(vec3<f32>(0.2126, 0.7152, 0.0722), color));
    return mix(color, gray, amount);
}

fn cubic(v: f32) -> vec4<f32> {
    let n = vec4<f32>(1.0, 2.0, 3.0, 4.0) - v;
    let s = n * n * n;
    let x = s.x;
    let y = s.y - 4.0 * s.x;
    let z = s.z - 4.0 * s.y + 6.0 * s.x;
    let w = 6.0 - x - y - z;
    return vec4<f32>(x, y, z, w) * (1.0 / 6.0);
}

fn texture_bicubic(tc_in: vec2<f32>) -> vec4<f32> {
    let tex_size = g.screen1.xy;
    let inv = 1.0 / tex_size;
    var tc = tc_in * tex_size - 0.5;
    let fxy = fract(tc);
    tc = tc - fxy;
    let xc = cubic(fxy.x);
    let yc = cubic(fxy.y);
    let c = tc.xxyy + vec2<f32>(-0.5, 1.5).xyxy;
    let s = vec4<f32>(xc.xz + xc.yw, yc.xz + yc.yw);
    var offset = c + vec4<f32>(xc.yw, yc.yw) / s;
    offset = offset * inv.xxyy;
    let s0 = textureSample(diffuse, diffuse_sampler, offset.xz);
    let s1 = textureSample(diffuse, diffuse_sampler, offset.yz);
    let s2 = textureSample(diffuse, diffuse_sampler, offset.xw);
    let s3 = textureSample(diffuse, diffuse_sampler, offset.yw);
    let sx = s.x / (s.x + s.y);
    let sy = s.z / (s.z + s.w);
    return mix(mix(s3, s2, sx), mix(s1, s0, sx), sy);
}

@fragment
fn fs_screen(in: VsOut) -> @location(0) vec4<f32> {
    let width = g.screen0.z;
    let height = g.screen0.w;
    let rot = coord_rot(in.uv, g.screen0.y + 1.425);
    let noise = vec3<f32>(pnoise3d(vec3<f32>(rot * vec2<f32>(width, height), 0.0)));
    let pixel = texture_bicubic(in.uv).xyz;
    // screenWorld with search mode off.
    var diffuse_c = desaturate(pixel, g.screen0.x);
    diffuse_c = clamp(diffuse_c, vec3<f32>(0.0), vec3<f32>(1.0));
    let intensity = 0.299 * diffuse_c.r + 0.587 * diffuse_c.g + 0.114 * diffuse_c.b;
    var col = contrast(desaturate(diffuse_c, 0.1), 1.2);
    var inv_i = 1.0 - intensity;
    inv_i = inv_i * inv_i;
    col = col + (noise * GRAIN_AMOUNT) * (inv_i * 5.0);
    return vec4<f32>(col, 1.0);
}
