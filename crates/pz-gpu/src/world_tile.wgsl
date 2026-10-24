// `tileWithDepth` and `opaqueWithDepth`; appended to world.wgsl.

@group(2) @binding(0) var depth_map: texture_2d<f32>;
@group(2) @binding(1) var depth_sampler: sampler;

struct DepthOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_tile(in: VsOut) -> DepthOut {
    var c = textureSample(diffuse, diffuse_sampler, in.uv);
    let d = textureSample(depth_map, depth_sampler, in.uv1).r;
    c = c * in.col;
    c = vec4<f32>(c.rgb * in.col.a, c.a);
    if (!(d > 0.0) || !alpha_pass(c.a)) {
        discard;
    }
    var o: DepthOut;
    o.color = c;
    o.depth = (in.depth.y - in.depth.x) * d + in.depth.x;
    return o;
}

@fragment
fn fs_opaque(in: VsOut) -> DepthOut {
    let c0 = textureSample(diffuse, diffuse_sampler, in.uv);
    let d = textureSample(depth_map, depth_sampler, in.uv1).r;
    var c = c0 * in.col;
    c = vec4<f32>(c.rgb * in.col.a, c.a);
    if (!(c0.a > 0.8 && d > 0.0) || !alpha_pass(c.a)) {
        discard;
    }
    var o: DepthOut;
    o.color = c;
    o.depth = (in.depth.y - in.depth.x) * d + in.depth.x;
    return o;
}

