// Port of the `fog` shader (`FogShader`, fog.vert / fog.frag) that
// `ImprovedFogDrawer` draws `ImprovedFog`'s rectangles with.
//
// The uniforms travel in the shared uniform block:
// screen[0] = screenInfo, screen[1] = rectangleInfo,
// surface[0] = worldOffset, surface[1] = paramInfo, surface[2] = cameraInfo,
// surface[3] = (colorInfo.rgb, textureInfo.z),
// surface[4] = (scalingInfo.x, scalingInfo.y, textureInfo.x, textureInfo.w)
// (fog.frag reads no other components), and `extra` the height to flip
// `gl_FragCoord` against on the main target (texture targets are drawn
// y-flipped, so their rows already count from the bottom as GL's do).
// The vertex's depth (`aDepth`) is written as the fragment depth.

struct Globals {
    mvp: mat4x4<f32>,
    alpha_func: u32,
    alpha_ref: f32,
    clip_z: f32,
    extra: f32,
    screen: array<vec4<f32>, 2>,
    surface: array<vec4<f32>, 5>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var noise_tex: texture_2d<f32>;
@group(1) @binding(1) var noise_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_fog(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) uv1: vec2<f32>, @location(4) uv2: vec2<f32>, @location(5) depth: vec2<f32>) -> VsOut {
    var o: VsOut;
    let p = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    // gl_FragDepth = vDepth.
    o.pos = vec4<f32>(p.x, p.y, depth.x * p.w, p.w);
    return o;
}

fn screen_info() -> vec4<f32> { return g.screen[0]; }
fn rectangle_info() -> vec4<f32> { return g.screen[1]; }
fn world_offset() -> vec4<f32> { return g.surface[0]; }
fn param_info() -> vec4<f32> { return g.surface[1]; }
fn camera_info() -> vec4<f32> { return g.surface[2]; }
fn color_info() -> vec3<f32> { return g.surface[3].xyz; }
fn scaling_info() -> vec2<f32> { return g.surface[4].xy; }
fn texture_info_x() -> f32 { return g.surface[4].z; }
fn texture_info_z() -> f32 { return g.surface[3].w; }
fn texture_info_w() -> f32 { return g.surface[4].w; }

fn noise(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(noise_tex, noise_sampler, uv);
}

// GLSL's smoothstep.
fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn fog_noise(fcoord: vec2<f32>) -> f32 {
    let si = screen_info();
    let sc = scaling_info();
    let scaling = 0.0008;
    var uv = vec2<f32>(fcoord.x * si.x * scaling, fcoord.y * si.y * scaling) + world_offset().xy * scaling;
    uv = uv * 0.25;

    var scaling_mod = 0.00035;
    var uv2 = vec2<f32>(uv.x + sc.x * scaling_mod, uv.y + sc.y * scaling_mod);

    var value = 0.0;
    var amplitude = 0.5;
    var amp_tot = amplitude;

    var tex1 = noise(uv2);
    value = value + amplitude * tex1.r;
    amplitude = amplitude * 0.5;
    amp_tot = amp_tot + amplitude;

    scaling_mod = 0.00070;
    uv2 = vec2<f32>(uv.x + sc.x * scaling_mod, uv.y + sc.y * scaling_mod);
    tex1 = noise(uv2);
    value = value + amplitude * tex1.g;
    amplitude = amplitude * 0.5;
    amp_tot = amp_tot + amplitude;

    scaling_mod = 0.00040;
    uv2 = vec2<f32>(uv.x + sc.x * scaling_mod, uv.y + sc.y * scaling_mod);

    uv2 = uv2 * 3.0;
    tex1 = noise(uv2);
    value = value + amplitude * tex1.b;

    value = 0.5 + 0.50 * (value / amp_tot);
    return clamp(value, 0.0, 1.0);
}

fn alpha_borders(coord: vec2<f32>, fcoord: vec2<f32>) -> f32 {
    let si = screen_info();
    let ri = rectangle_info();
    let pi = param_info();
    let ci = camera_info();
    var border_alpha = 1.0;

    var scaling = 0.00035;
    var uv = vec2<f32>(fcoord.x * si.x * scaling, fcoord.y * si.y * scaling) + world_offset().xy * scaling;

    // Row-based offsets.
    uv.x = uv.x + ci.w;
    uv.y = uv.y + ci.w;

    var tex1 = noise(uv);
    var n = tex1.a;

    tex1 = noise(uv + 0.5);
    let n2 = tex1.a;

    // Irregular alpha fading borders at the top and bottom, linear at the
    // left and right of the rectangle.
    var rec_coord = ri.xy / si.xy;
    let rec_dim = vec2<f32>(ri.z, ri.w) / si.xy;
    rec_coord.y = 1.0 - rec_coord.y;

    var p = coord;
    p.y = (rec_coord.y - coord.y) / rec_dim.y;
    p.x = (coord.x - rec_coord.x) / rec_dim.x;

    // top
    var height = pi.x - ((pi.x * 0.5) * n);
    border_alpha = min(p.y / height, 1.0);
    // bottom
    height = pi.y - ((pi.y * 0.5) * n2);
    border_alpha = min(max((1.0 - p.y) / height, 0.0), border_alpha);

    var sides_width = (32.0 * texture_info_w()) / si.x;
    sides_width = sides_width / rec_dim.x;
    // right
    border_alpha = min(max((1.0 - p.x) / sides_width, 0.0), border_alpha);
    // left
    border_alpha = min(p.x / sides_width, border_alpha);

    // A last noise over the areas left below full alpha.
    scaling = 0.00085;
    uv = vec2<f32>(fcoord.x * si.x * scaling, fcoord.y * si.y * scaling) + world_offset().xy * scaling;

    uv.x = uv.x + ci.w;
    uv.y = uv.y + ci.w;

    tex1 = noise(uv);
    n = tex1.a;
    border_alpha = border_alpha + (1.0 - border_alpha) * (n * border_alpha);
    border_alpha = border_alpha * border_alpha;

    border_alpha = clamp(border_alpha, 0.0, 1.0);
    return border_alpha;
}

fn alpha_circle(alpha: f32, rad: f32, coord: vec2<f32>, zoom: f32) -> f32 {
    let si = screen_info();
    let wo = world_offset();
    var center = vec2<f32>(0.5, 0.5);
    // The right-click view-in-distance offset.
    center.x = center.x - wo.z / si.x;
    center.y = center.y + wo.w / si.y;
    let dist = distance(coord.xy, center);
    let base_alpha = ss(0.01, 0.99, dist * rad * zoom);
    return alpha + ((1.0 - alpha) * base_alpha);
}

@fragment
fn fs_fog(in: VsOut) -> @location(0) vec4<f32> {
    var frag = in.pos.xy;
    if (g.extra > 0.0) {
        frag.y = g.extra - frag.y;
    }
    let si = screen_info();
    let ci = camera_info();
    let pi = param_info();
    let zoom = si.z;
    let coord = (vec2<f32>(frag.x - ci.x, frag.y - ci.y) * zoom) / si.xy;
    let alpha = alpha_circle(pi.z, pi.w, coord, zoom);

    let fcoord = vec2<f32>(frag.x / (si.x / zoom), 1.0 - frag.y / (si.y / zoom));
    let layer_alpha = si.w;

    let border_alpha = alpha_borders(coord, fcoord);
    let n = fog_noise(fcoord);

    if (texture_info_x() >= 1.0) {
        let alp = clamp(texture_info_z() * alpha * border_alpha * layer_alpha, 0.0, 1.0);
        return vec4<f32>(1.0, 1.0, 0.0, alp);
    }

    var alp = max(0.0, (n - 0.50) / 0.50);
    alp = alp * texture_info_z() * alpha * border_alpha * layer_alpha;
    alp = clamp(alp, 0.0, 1.0);

    let c = color_info();
    return vec4<f32>(n * c.r, n * c.g, n * c.b, alp);
}
