// `chunkShader`; appended to world.wgsl.

struct DepthOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@group(2) @binding(0) var chunk_depth: texture_depth_2d;

@fragment
fn fs_composite(in: VsOut) -> DepthOut {
    let c = textureSample(diffuse, diffuse_sampler, in.uv) * in.col;
    // The depth texture is sampled GL_NEAREST.
    let size = vec2<f32>(textureDimensions(chunk_depth));
    let texel = vec2<i32>(clamp(floor(in.uv * size), vec2<f32>(0.0), size - 1.0));
    let d = textureLoad(chunk_depth, texel, 0);
    if (!alpha_pass(c.a)) {
        discard;
    }
    var o: DepthOut;
    o.color = c;
    o.depth = in.depth.x + d;
    return o;
}
