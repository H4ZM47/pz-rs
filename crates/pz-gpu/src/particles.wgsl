// Ports of the `fire` and `smoke` shaders (`FireShader`, `SmokeShader`)
// that `ModelManager.RenderParticles` draws `ParticlesFire` with.
//
// Each particle is one quad of the shared vertex layout: `pos` is the
// corner of the unit quad (`Vertex`, -1..1) and `uv`/`uv1` the instance
// attribute `Particule` (x, y, id, slot fraction). `surface[0].x` holds
// `FireTime` and `surface[1..4]` the columns of `FireParam`.

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
@group(1) @binding(0) var fire_tex: texture_2d<f32>;
@group(1) @binding(1) var fire_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) alpha: f32,
};

fn fire_time() -> f32 { return g.surface[0].x; }

// FireParam[col][row].
fn fire_param(c: i32, r: i32) -> f32 {
    return g.surface[1 + c][r];
}

fn hash(p: vec2<f32>, id: f32) -> f32 {
    let h = dot(p, vec2<f32>(127.1 * id, 311.7 * id));
    return fract(sin(h) * 43758.5453123);
}

fn noise(p: vec2<f32>, id: f32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return -1.0 + 2.0 * mix(mix(hash(i + vec2<f32>(0.0, 0.0), id), hash(i + vec2<f32>(1.0, 0.0), id), u.x),
                            mix(hash(i + vec2<f32>(0.0, 1.0), id), hash(i + vec2<f32>(1.0, 1.0), id), u.x), u.y);
}

fn get_rotation(angle: f32) -> mat3x3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return mat3x3<f32>(vec3<f32>(c, s, 0.0), vec3<f32>(-s, c, 0.0), vec3<f32>(0.0, 0.0, 1.0));
}

// GLSL's smoothstep, also with reversed edges.
fn ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn glsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// The cell `id` of a `tiles_x` x `tiles_y` atlas for this corner.
fn atlas(vertex: vec2<f32>, id: f32, tiles_x: f32, tiles_y: f32) -> vec2<f32> {
    let tx = glsl_mod(id, tiles_x);
    let ty = floor(id / tiles_x);
    return ((vertex * 0.5) + vec2<f32>(0.5, 0.5)) / vec2<f32>(tiles_x, tiles_y) + vec2<f32>(tx / tiles_x, ty / tiles_y);
}

@vertex
fn vs_fire(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) uv1: vec2<f32>, @location(4) uv2: vec2<f32>, @location(5) depth: vec2<f32>) -> VsOut {
    let part = vec4<f32>(uv, uv1);
    let vertex = vec3<f32>(pos, 0.0);
    let t = fire_time();
    let lifetime = 0.60;
    let time_shift = part.w * lifetime;
    let hash_a = noise(vec2<f32>(part.z, 18.0 * floor((t + time_shift) / lifetime)), 1.0);
    let life = fract((t + time_shift) / lifetime);
    var size = 2.0 * life * (1.0 - life) + 0.5;
    size *= 1.0 + hash_a * 0.5;
    let dy = life * 150.0 + fire_param(0, 1) * life * 250.0;
    let dx = fire_param(0, 0) * life * 500.0;
    let y = part.y - dy;
    let x = part.x - dx;
    let particle_rot = 3.1415 * (0.13 * hash_a);
    var o: VsOut;
    o.pos = g.mvp * vec4<f32>(vertex * 120.0 * 0.15 * 5.0 * size * get_rotation(particle_rot) + vec3<f32>(x, y, 0.0), 1.0);
    o.alpha = ss(0.0, 0.1, life) * ss(1.0, 0.6, life) * 0.5;
    o.tex_coord = atlas(pos, floor(life * 8.0 * 4.0 * 1.0), 8.0, 4.0);
    return o;
}

@fragment
fn fs_fire(i: VsOut) -> @location(0) vec4<f32> {
    let tex1 = textureSample(fire_tex, fire_sampler, i.tex_coord);
    return vec4<f32>(tex1.rgb, tex1.r * i.alpha);
}

@vertex
fn vs_smoke(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) uv1: vec2<f32>, @location(4) uv2: vec2<f32>, @location(5) depth: vec2<f32>) -> VsOut {
    let part = vec4<f32>(uv, uv1);
    let vertex = vec3<f32>(pos, 0.0);
    let t = fire_time();
    let hash_v = noise(part.xz, 1.0);
    let lifetime = 1.60 + 0.2 * hash_v;
    let time_shift = part.w * lifetime;
    let hash_a = noise(vec2<f32>(part.x, 18.0 * floor((t + time_shift) / lifetime)), 1.0);
    let life = fract((t + time_shift) / lifetime);
    let size = 2.0 * life + 0.5;
    let dy = life * 400.0 + fire_param(0, 1) * life * 1500.0;
    let dx = life * hash_a * 60.0 + fire_param(0, 0) * life * 500.0 * 2.66;
    let y = part.y - dy;
    let x = part.x - dx;
    let particle_rot = 3.1415 * hash_a;
    var o: VsOut;
    o.tex_coord = atlas(pos, floor(2.0 * fract(2.0 + hash_a)), 2.0, 2.0);
    o.pos = g.mvp * vec4<f32>(vertex * 120.0 * 0.8 * size * get_rotation(particle_rot) + vec3<f32>(x, y, 0.0), 1.0);
    o.alpha = ss(0.0, 2.0, life) * ss(1.0, 0.3, life) * 2.0;
    return o;
}

@fragment
fn fs_smoke(i: VsOut) -> @location(0) vec4<f32> {
    let color = textureSample(fire_tex, fire_sampler, i.tex_coord);
    return vec4<f32>(color.rgb, color.a * i.alpha);
}
