// vboRenderer_SpriteOutline (FBORenderObjectOutline.Drawer2): a sprite or
// atlas entry drawn into the outline mask as a flat colour wherever its
// texture's alpha is at least 0.5.

struct Globals {
    mvp: mat4x4<f32>,
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
fn vs_sprite_outline(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>) -> VsOut {
    var o: VsOut;
    let p = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    o.pos = vec4<f32>(p.x, p.y, 0.5 * p.w, p.w);
    o.col = col;
    o.uv = uv;
    return o;
}

@fragment
fn fs_sprite_outline(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(diffuse, diffuse_sampler, in.uv);
    return vec4<f32>(in.col.rgb, in.col.a * step(0.5, texel.a));
}
