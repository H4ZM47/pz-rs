// Port of the game's `default` sprite shader (the program SpriteRenderer
// draws UI and sprite quads with): position through ModelViewProjection,
// texture times vertex colour, transparent fragments discarded. GL's
// fixed-function alpha test, which still applies after the shader in the
// compatibility profile, is folded in as `alpha_func`.

struct Globals {
    mvp: mat4x4<f32>,
    // GL comparison (GL_NEVER + n) for the alpha test, or 8 when disabled.
    alpha_func: u32,
    alpha_ref: f32,
    // Window-space depth of the quad (0.5 is GL clip z 0).
    clip_z: f32,
    // Per-program extra uniform (CircleStencil's discardValue).
    extra: f32,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) col: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>) -> VsOut {
    var o: VsOut;
    let p = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    o.pos = vec4<f32>(p.x, p.y, g.clip_z, p.w);
    o.col = col;
    o.uv = uv;
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
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Untextured quads bind a 1x1 white texture, which samples as 1.0.
    var c = textureSample(diffuse, diffuse_sampler, in.uv);
    c = c * in.col;
    if (c.a == 0.0 || !alpha_pass(c.a)) {
        discard;
    }
    return c;
}

// `glClear` drawn as a quad: no alpha test, no discard.
@fragment
fn fs_clear(in: VsOut) -> @location(0) vec4<f32> {
    return in.col;
}
