// `DeadBodyAtlas`; appended after world_composite.wgsl (atlas colour in
// group 1, its depth in group 2): colour times texel, depth blended from
// zDepthBlendZ (in.depth.x) to zDepthBlendToZ (in.depth.y) by the stored
// depth, texels with alpha * depth == 0 discarded.

@fragment
fn fs_atlas(in: VsOut) -> DepthOut {
    let texel = textureSample(diffuse, diffuse_sampler, in.uv);
    // The depth texture is sampled GL_NEAREST.
    let size = vec2<f32>(textureDimensions(chunk_depth));
    let uv = fract(in.uv);
    let t = vec2<i32>(clamp(floor(uv * size), vec2<f32>(0.0), size - 1.0));
    let d = textureLoad(chunk_depth, t, 0);
    if (!(texel.a * d > 0.0)) {
        discard;
    }
    var o: DepthOut;
    o.color = in.col * texel;
    o.depth = (in.depth.y - in.depth.x) * d + in.depth.x;
    return o;
}

// vs_world with the quad's z held inside the clip volume: the fragment
// depth comes from the atlas, so the vertex depth only must not clip.
@vertex
fn vs_atlas(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) uv1: vec2<f32>, @location(4) uv2: vec2<f32>, @location(5) depth: vec2<f32>) -> VsOut {
    var o: VsOut;
    let p = g.mvp * vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    o.pos = vec4<f32>(p.x, p.y, 0.5 * p.w, p.w);
    o.col = col;
    o.uv = uv;
    o.uv1 = uv1;
    o.depth = depth;
    o.uv2 = uv2;
    return o;
}
