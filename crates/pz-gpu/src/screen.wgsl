// Port of the `screen` shader `MultiTextureFBO2.render` draws each
// player's part of the offscreen world buffer with (WeatherShader):
// bicubic resampling, the drunk deform, the short-sighted blur, then the
// world grade (search mode: blur, desaturate and darken outside the
// circle; desaturate, contrast, film grain) or the night-vision goggles.

struct Globals {
    mvp: mat4x4<f32>,
    alpha_func: u32,
    alpha_ref: f32,
    clip_z: f32,
    // gl_FragCoord.y is this minus the framebuffer row (the main target's
    // height), or the row itself when 0 (texture targets).
    flip: f32,
    // DesaturationVal, timer, bgl_RenderedTextureWidth/Height.
    screen0: vec4<f32>,
    // TextureSize, timerWrap, NightValue.
    screen1: vec4<f32>,
    search_mode: vec4<f32>,
    screen_info: vec4<f32>,
    param_info: vec4<f32>,
    var_info: vec4<f32>,
    // NightVisionGoggles, DrunkFactor, BlurFactor, Zoom.
    screen2: vec4<f32>,
    // Light, LightIntensity (the live shader reads neither).
    light: vec4<f32>,
    // Exterior (not read).
    screen3: vec4<f32>,
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
const GRAIN_SIZE: f32 = 1.0;
const AVG_LUMIN: vec3<f32> = vec3<f32>(0.4, 0.4, 0.4);
// Night vision.
const CONTRAST2: f32 = 0.5;
const LUMVEC: vec3<f32> = vec3<f32>(0.30, 0.59, 0.11);
const INTENSITY_ADJUST: f32 = 1.3;
// Blur: the original loops `d` from 0 by blur_pi / 16 while below
// blur_pi and `i` from 1/3 by 1/3 while at most 1.01, both in f32: 16
// directions of 3 steps, 49 samples divided by 51.
const BLUR_PI: f32 = 6.28318530718;
const BLUR_DIRECTIONS: f32 = 16.0;
const BLUR_QUALITY: f32 = 3.0;
const BLUR_SIZE: f32 = 12.0;

fn timer() -> f32 {
    return g.screen0.y;
}

fn sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(diffuse, diffuse_sampler, uv, 0.0);
}

fn rnm(tc: vec2<f32>) -> vec4<f32> {
    let t = timer();
    let noise = sin(dot(tc + vec2<f32>(t, t), vec2<f32>(12.9898, 78.233))) * 43758.5453;
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

// util/math's desaturate.
fn desaturate(color: vec3<f32>, amount: f32) -> vec3<f32> {
    let gray = vec3<f32>(dot(vec3<f32>(0.2126, 0.7152, 0.0722), color));
    return mix(color, gray, amount);
}

// GLSL's smoothstep, also for edge0 >= edge1.
fn gl_smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
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
    let s0 = sample(offset.xz);
    let s1 = sample(offset.yz);
    let s2 = sample(offset.xw);
    let s3 = sample(offset.yw);
    let sx = s.x / (s.x + s.y);
    let sy = s.z / (s.z + s.w);
    return mix(mix(s3, s2, sx), mix(s1, s0, sx), sy);
}

// The circle's centre in screen fractions, moved by the right-click view
// offset.
fn view_centre() -> vec2<f32> {
    return vec2<f32>(0.5 - g.screen_info.z / g.screen_info.x, 0.5 + g.screen_info.w / g.screen_info.y);
}

// blur(): the unfiltered buffer around `uv`, mixed over `col` by `alpha`.
fn blur(col: vec3<f32>, alpha: f32, uv: vec2<f32>) -> vec3<f32> {
    let rad = BLUR_SIZE / g.screen_info.xy;
    var c = sample(uv).rgb;
    var d = 0.0;
    for (var k = 0; k < 16; k++) {
        var i = 1.0 / BLUR_QUALITY;
        for (var j = 0; j < 3; j++) {
            c += sample(uv + vec2<f32>(cos(d), sin(d)) * rad * i).rgb;
            i += 1.0 / BLUR_QUALITY;
        }
        d += BLUR_PI / BLUR_DIRECTIONS;
    }
    c /= BLUR_QUALITY * (BLUR_DIRECTIONS + 1.0);
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return col * (1.0 - alpha) + c * alpha;
}

fn search_circle(rad: f32, coord: vec2<f32>) -> f32 {
    let dist = distance(coord, view_centre()) * g.screen_info.x;
    let r = rad * g.param_info.y;
    return gl_smoothstep(max(0.0, r - g.param_info.z), r + g.param_info.z, dist);
}

fn screen_world(pixel: vec3<f32>, noise: vec3<f32>, frag: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    let desat = g.screen0.x;
    var d: vec3<f32>;
    if g.var_info.x == 0.0 {
        d = desaturate(pixel, desat);
    } else {
        let zoom = g.param_info.x;
        let coord = (vec2<f32>(frag.x - g.search_mode.z, frag.y - g.search_mode.w + (56.0 / zoom)) * zoom) / g.screen_info.xy;
        let circle_alpha = search_circle(g.search_mode.y, coord);
        d = blur(pixel, circle_alpha * g.search_mode.x, uv);
        d = desaturate(d, max(desat, circle_alpha * g.param_info.w));
        d *= 1.0 - g.var_info.y * circle_alpha;
    }
    d = clamp(d, vec3<f32>(0.0), vec3<f32>(1.0));
    let intensity = 0.299 * d.r + 0.587 * d.g + 0.114 * d.b;
    var col = contrast(desaturate(d, 0.1), 1.2);
    var inv_i = 1.0 - intensity;
    inv_i = inv_i * inv_i;
    col = col + (noise * GRAIN_AMOUNT) * (inv_i * 5.0);
    return col;
}

fn screen_nightvision(pixel: vec3<f32>, noise: vec3<f32>) -> vec3<f32> {
    let night = g.screen1.w;
    var d = pixel;
    let intensity = 0.299 * d.r + 0.587 * d.g + 0.114 * d.b;
    d = mix(d, vec3<f32>(intensity), (night * 0.8) * (1.0 - intensity));
    d = ((1.0 - night) * d) + (night * d * d) * 1.5;
    let shift = night * 0.25 * (1.0 - intensity);
    d = vec3<f32>(d.r - shift, d.g - shift, d.b + shift);
    var col = contrast(desaturate(d, 0.1), 1.2);
    var inv_i = 1.0 - intensity;
    inv_i = inv_i * inv_i;
    col = vec3<f32>(col.r + 0.45, col.g + 0.45, col.b);
    col = col * 1.9 + (noise * 0.015) * (inv_i * 5.0);
    var intensity2 = dot(LUMVEC, col);
    intensity2 = clamp(CONTRAST2 * (intensity2 - 0.5) + 0.5, 0.0, 1.0);
    let green = clamp(intensity2 / 0.59, 0.0, 1.0) * INTENSITY_ADJUST;
    return col * vec3<f32>(0.0, green, 0.0);
}

fn drunk_boundaries(coord: vec2<f32>) -> f32 {
    let centre = view_centre();
    let dist_x = abs((coord.x - (0.5 - centre.x)) - centre.x);
    let dist_y = abs((coord.y - (0.5 - centre.y)) - centre.y);
    return 1.0 - max(dist_x / 0.5, dist_y / 0.5);
}

fn drunk_circle(coord: vec2<f32>) -> f32 {
    let dist = distance(coord, view_centre()) * g.screen_info.x;
    let gradient_width = 20.0;
    let gradient_width_alpha = (gradient_width * 64.0) / 2.0;
    return gl_smoothstep(max(0.0, gradient_width - gradient_width_alpha), gradient_width + gradient_width_alpha, dist);
}

// The offscreen-left/top-relative fragment in screen fractions.
fn view_coord(frag: vec2<f32>) -> vec2<f32> {
    return (vec2<f32>(frag.x - g.search_mode.z, frag.y - g.search_mode.w) * g.param_info.x) / g.screen_info.xy;
}

fn drunken_deform(pixel: vec3<f32>, frag: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    var col = clamp(pixel, vec3<f32>(0.0), vec3<f32>(1.0));
    let drunk = g.screen2.y;
    let coord = view_coord(frag);
    let circle_alpha = drunk_circle(coord);
    let drunk_alpha = drunk_boundaries(coord);
    let drunk_alpha_b = drunk_alpha * circle_alpha;
    let jiggle1 = 7.3;
    let jiggle2 = 13.3;
    let jiggle_mag = 0.005 * drunk;
    let t = timer();
    let wrap = g.screen1.z;
    let coord2 = uv + (jiggle_mag * vec2<f32>(cos(t / (wrap * jiggle1)), sin(t / (wrap * jiggle2))) / g.param_info.x);
    let coord3 = uv + (jiggle_mag * vec2<f32>(cos(t / (wrap * jiggle2)), sin(t / (wrap * jiggle1))) / g.param_info.x);
    let pixel2 = texture_bicubic(coord2).xyz;
    let pixel3 = texture_bicubic(coord3).xyz;
    let pixel_blur = blur(col, drunk, uv);
    col = mix(col, pixel2, vec3<f32>(drunk_alpha_b));
    col = mix(col, pixel3, vec3<f32>(drunk_alpha_b));
    col = mix(col, pixel_blur, vec3<f32>(1.0 - drunk_alpha));
    return clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn blur_circle(coord: vec2<f32>, rad: f32) -> f32 {
    let zoom = g.param_info.x;
    let dist = distance(coord, view_centre()) * g.screen_info.x;
    let radius_pixel_w = (rad * 64.0) / 2.0;
    let grad_pixel_w = (radius_pixel_w * 3.0) * zoom;
    let gradient_width = grad_pixel_w * (rad / 32.0 / zoom);
    let gradient_width_alpha = (gradient_width * 64.0) / 2.0;
    return gl_smoothstep(max(0.0, gradient_width - gradient_width_alpha), gradient_width + gradient_width_alpha, dist);
}

// "my glasses!": the short-sighted blur away from the player.
fn screen_blur(pixel: vec3<f32>, frag: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    let col = clamp(pixel, vec3<f32>(0.0), vec3<f32>(1.0));
    let circle_alpha = blur_circle(view_coord(frag), 3.0);
    return blur(col, g.screen2.z * circle_alpha, uv);
}

@fragment
fn fs_screen(in: VsOut) -> @location(0) vec4<f32> {
    let frag = vec2<f32>(in.pos.x, select(in.pos.y, g.flip - in.pos.y, g.flip > 0.0));
    let width = g.screen0.z;
    let height = g.screen0.w;
    let rot = coord_rot(in.uv, timer() + 1.425);
    let noise = vec3<f32>(pnoise3d(vec3<f32>(rot * vec2<f32>(width / GRAIN_SIZE, height / GRAIN_SIZE), 0.0)));
    let pixel = texture_bicubic(in.uv).xyz;
    var col = pixel;
    if g.screen2.y > 0.0 {
        col = drunken_deform(pixel, frag, in.uv);
    }
    if g.screen2.z > 0.0 {
        col = screen_blur(col, frag, in.uv);
    }
    if g.screen2.x < 0.5 {
        col = screen_world(col, noise, frag, in.uv);
    } else {
        col = screen_nightvision(col, noise);
    }
    return vec4<f32>(col, 1.0);
}
