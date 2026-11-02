// Um único pipeline para fundo, glifo, sublinhado e cursor: o que muda é o UV.
// Fundo/sublinhado/cursor apontam para o pixel branco do atlas; glifo aponta
// para a máscara de cobertura. A ordem das instâncias é a ordem de desenho, o
// que resolve sobreposição (cursor por cima do texto) sem draw call extra.

struct Globals {
    screen: vec2<f32>,
    // margem em px (`window_padding_width`): a grade é desenhada a partir daqui
    origin: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var atlas: texture_2d<f32>;
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
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
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
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let m = textureSample(atlas, samp, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * m);
}
