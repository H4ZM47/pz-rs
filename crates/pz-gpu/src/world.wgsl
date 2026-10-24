// Ports of the world shaders: `default` with world depth, `tileWithDepth`,
// `opaqueWithDepth` and `chunkShader`. Per-quad depth values, uniforms in
// the original, travel in the vertex's texture-2 coordinates so quads with
// different depths still batch: (zDepth in window space, unused) for
// `default`, (zDepthBlendZ, zDepthBlendToZ) for the depth-texture shaders,
// (chunkDepth, unused) for `chunkShader`.

struct Globals {
    mvp: mat4x4<f32>,
    alpha_func: u32,
    alpha_ref: f32,
    clip_z: f32,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) col: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    @location(3) @interpolate(flat) depth: vec2<f32>,
};

@vertex
fn vs_world(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) uv1: vec2<f32>, @location(4) depth: vec2<f32>) -> VsOut {
    var o: VsOut;
    let p = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    o.pos = vec4<f32>(p.x, p.y, depth.x, p.w);
    o.col = col;
    o.uv = uv;
    o.uv1 = uv1;
    o.depth = depth;
    return o;
}

fn alpha_pass(a: f32) -> bool {
    switch g.alpha_func {
        case 0u: { return false; }
        case 1u: { return a < g.alpha_ref; }
        case 2u: { return a == g.alpha_ref; }
        case 3u: { return a <= g.alpha_ref; }
        case 4u: { return a > g.alpha_ref; }
        case 5u: { return a != g.alpha_ref; }
        case 6u: { return a >= g.alpha_ref; }
        default: { return true; }
    }
}

@fragment
fn fs_default(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(diffuse, diffuse_sampler, in.uv) * in.col;
    if (c.a == 0.0 || !alpha_pass(c.a)) {
        discard;
    }
    return c;
}


// `vboRenderer_PositionColorUV`: colour times texel, nearly transparent
// texels discarded, no premultiplication.
@fragment
fn fs_projected(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(diffuse, diffuse_sampler, in.uv);
    if (texel.a < 0.01) {
        discard;
    }
    return in.col * texel;
}
