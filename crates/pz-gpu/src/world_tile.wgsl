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


// `seamFix2`: the tile drawn only where the seam mask covers it.
@group(3) @binding(0) var seam_mask: texture_2d<f32>;
@group(3) @binding(1) var seam_sampler: sampler;

@fragment
fn fs_seam(in: VsOut) -> DepthOut {
    var c = textureSample(diffuse, diffuse_sampler, in.uv);
    let d = textureSample(depth_map, depth_sampler, in.uv1).r;
    let m = textureSample(seam_mask, seam_sampler, in.uv2);
    c = c * in.col;
    c = vec4<f32>(c.rgb * in.col.a, c.a);
    if (!(d * m.a > 0.0) || !alpha_pass(c.a)) {
        discard;
    }
    var o: DepthOut;
    o.color = c;
    o.depth = (in.depth.y - in.depth.x) * d + in.depth.x;
    return o;
}

// `CircleStencil`: group 2 is the wall's depth texture, group 3 the
// cutaway stencil; `g.extra` is `discardValue`.
@fragment
fn fs_cutaway(in: VsOut) -> DepthOut {
    var c = textureSample(diffuse, diffuse_sampler, in.uv);
    let st = textureSample(seam_mask, seam_sampler, in.uv2);
    if ((st.r > 0.0) != (g.extra > 0.0)) {
        discard;
    }
    c = vec4<f32>(c.rgb * in.col.rgb, c.a);
    c = c * vec4<f32>(st.r, st.r, st.r, st.a);
    c = vec4<f32>(c.rgb + 0.75 * st.b, c.a);
    c.a = max(c.a, st.b * st.a);
    c.a = max(c.a, st.g * st.a);
    c = vec4<f32>(c.rgb - st.g, c.a);
    c.a = c.a * in.col.a;
    let d = textureSample(depth_map, depth_sampler, in.uv1).r;
    if (c.a == 0.0 || d == 0.0 || !alpha_pass(c.a)) {
        discard;
    }
    var o: DepthOut;
    o.color = c;
    o.depth = (in.depth.y - in.depth.x) * d + in.depth.x;
    return o;
}
