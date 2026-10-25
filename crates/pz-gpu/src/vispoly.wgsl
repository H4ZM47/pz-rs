// VisibilityPolygon2's shaders: `visPolygon` (the shadow triangles into
// the blur target) and `visibilityBlur` (the darkening over the world).
// Texture targets are drawn with clip-space y flipped, so a fragment's
// position addresses a texture target the way GL's gl_FragCoord does.

struct Vis {
    // ModelViewProjection (column major), y flipped for texture targets.
    mvp: mat4x4<f32>,
    // screenSize, displayOrigin.
    a: vec4<f32>,
    // displaySize, texSize.
    b: vec4<f32>,
    // TextureSize, zoom, opacityScale.
    c: vec4<f32>,
    // The main target's height when drawing into it (gl_FragCoord.y counts
    // from the bottom there), else 0.
    d: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Vis;

// visPolygon.vert / visPolygon.frag.

struct PolyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) dist: f32,
    @location(1) depth: f32,
};

@vertex
fn vs_polygon(@location(0) position: vec3<f32>, @location(1) dist: f32, @location(2) depth: f32) -> PolyOut {
    var o: PolyOut;
    o.dist = dist;
    o.depth = depth - 0.0001;
    o.pos = u.mvp * vec4<f32>(position, 1.0);
    return o;
}

struct PolyFrag {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_polygon(i: PolyOut) -> PolyFrag {
    let d = min(i.dist, 1.0);
    var o: PolyFrag;
    o.depth = i.depth;
    o.colour = vec4<f32>(d, d, d, 1.0);
    return o;
}

// visibilityBlur.vert over RenderTarget.DrawFullScreenQuad's quad
// (positions -1..1, drawn as 2 * (position - 0.5)).

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;
@group(2) @binding(0) var depth_tex: texture_depth_2d;

@vertex
fn vs_blur(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Indices 0, 1, 2, 1, 3, 2 of (-1,-1), (-1,1), (1,-1), (1,1).
    var idx = array<u32, 6>(0u, 1u, 2u, 1u, 3u, 2u);
    var p = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0));
    let q = p[idx[i]];
    // The quad's depth (1.0) is replaced by gl_FragDepth.
    return vec4<f32>(2.0 * (q - vec2<f32>(0.5, 0.5)), 0.0, 1.0);
}

const BLUR_RANGE: i32 = 2;
const BASE_OPACITY: f32 = 0.022;
const BLUR_SAMPLES: f32 = 25.0;
const BLUR_SCALE: f32 = 1.0 / 25.0;

struct BlurFrag {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_blur(@builtin(position) frag: vec4<f32>) -> BlurFrag {
    var fc = frag.xy;
    if (u.d.x > 0.0) {
        fc.y = u.d.x - fc.y;
    }
    let screen_size = u.a.xy;
    let display_origin = u.a.zw;
    let display_size = u.b.xy;
    let tex_size = u.b.zw;
    let texture_size = u.c.xy;
    let zoom = u.c.z;
    let opacity_scale = u.c.w;

    let screen_uv = (fc - display_origin) / display_size;
    let vision = textureSampleLevel(tex, tex_sampler, screen_uv, 0.0);
    var alpha = 0.0;
    // `spread` is computed and unused in the original.
    let zoom_factor = 1.0 / max(zoom, 0.1);
    let spread = mix(2.0, 16.0, zoom_factor * (vision.r * 4.0));
    let max_uv = tex_size / texture_size;
    let pixel = vec2<i32>(fc / screen_size * tex_size);
    let pixel_step = vec2<f32>(1.0, 1.0) / texture_size;
    for (var y = -BLUR_RANGE; y <= BLUR_RANGE; y++) {
        for (var x = -BLUR_RANGE; x <= BLUR_RANGE; x++) {
            let sam_uv = min(vec2<f32>(pixel + vec2<i32>(x, y)) * pixel_step, max_uv);
            alpha += textureSampleLevel(tex, tex_sampler, sam_uv, 0.0).a;
        }
    }
    alpha -= BLUR_SAMPLES * 0.5;
    alpha *= 2.0;
    alpha *= BLUR_SCALE;

    // The depth texture: GL_NEAREST, GL_REPEAT.
    let dims = vec2<f32>(textureDimensions(depth_tex));
    let texel = vec2<i32>(floor(fract(screen_uv) * dims));
    var o: BlurFrag;
    o.depth = textureLoad(depth_tex, texel, 0);
    o.colour = vec4<f32>(0.0, 0.0, 0.0, alpha * (BASE_OPACITY * opacity_scale));
    _ = spread;
    return o;
}
