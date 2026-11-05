// Images of the graphics protocol (H1–H13): the same instancing as
// shader.wgsl (rectangle + uv), but sampling the image texture as RGB with
// alpha instead of the atlas' coverage mask. It reuses the atlas' bind group
// layout — only which texture is bound in group 1 changes.

struct Globals {
    screen: vec2<f32>,
    origin: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Inst {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec4<f32>,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_img(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let k = corners[vi];
    let p = g.origin + inst.rect.xy + k * inst.rect.zw;
    var o: VOut;
    o.pos = vec4<f32>(
        p.x / g.screen.x * 2.0 - 1.0,
        1.0 - p.y / g.screen.y * 2.0,
        0.0,
        1.0,
    );
    o.color = inst.color;
    o.uv = inst.uv.xy + k * (inst.uv.zw - inst.uv.xy);
    return o;
}

@fragment
fn fs_img(in: VOut) -> @location(0) vec4<f32> {
    // image with alpha (PNG, raw RGBA) goes in as is; the instance colour
    // multiplies so the app can erase/tint in the future
    let c = textureSample(tex, samp, in.uv);
    return vec4<f32>(c.rgb * in.color.rgb, c.a * in.color.a);
}
