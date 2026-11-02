//! Renderizador: monta instâncias e desenha em uma única passada na GPU.
//!
//! O gargalo medido do bzmux Python era a **alternância** de `fill` de fundo e
//! `show_layout` por run (13,0 ms/frame; 7,9 ms só agrupando). Aqui fundo e
//! glifo são a mesma instância de quad — não existe alternância para pagar, e
//! o frame custa o que custa montar vértices (dezenas de µs), não o desenho.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use bzmux_core::cell::{Cell, INV};
use bzmux_core::config::{Config, CursorShape, Theme};
use bzmux_core::{Color, Span, Term};
use swash::scale::ScaleContext;
use winit::window::Window;

use crate::atlas::{bind_atlas, globals_buffer, globals_layout, Atlas};
use crate::glyph::{self, Fonts};
use crate::prof;

/// Fundo e frente usados quando a configuração não diz nada (os mesmos do
/// bzmux Python, `BG_DEFAULT`/`FG_DEFAULT`).
pub const BG: (u8, u8, u8) = (0, 0, 0);
pub const FG: (u8, u8, u8) = (214, 214, 214);

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Instance {
    rect: [f32; 4],
    color: [f32; 4],
    /// u0, v0, u1, v1 — os quatro iguais = quad sólido (pixel branco).
    uv: [f32; 4],
}

/// A célula está invertida nesta linha/coluna? `SGR 7` inverte sempre; a
/// seleção inverte o que está dentro dela.
#[inline]
fn is_swap(cell: &Cell, sel: Option<(usize, usize, usize, usize)>, y: usize, x: usize) -> bool {
    let inv = cell.flags & INV != 0;
    let dentro = sel.is_some_and(|(r1, c1, r2, c2)| {
        y >= r1
            && y <= r2
            && x >= (if y == r1 { c1 } else { 0 })
            && x <= (if y == r2 { c2 } else { usize::MAX })
    });
    inv ^ dentro
}

/// Instâncias de uma linha: a máscara (texto, fundo, decoração) e o emoji
/// colorido, que vai em draw call própria porque tem atlas RGBA.
#[derive(Default, Clone)]
struct RowQuads {
    mask: Vec<Instance>,
    emoji: Vec<Instance>,
}

#[derive(Copy, Clone)]
struct GlyphQuad {
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    uv: [f32; 4],
    /// Veio do atlas RGBA (emoji colorido) em vez da máscara.
    color: bool,
}

pub struct Renderer {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    atlas_bind: wgpu::BindGroup,
    inst_buf: wgpu::Buffer,
    inst_cap: usize,
    pub atlas: Atlas,
    pub fonts: Fonts,
    scale_ctx: ScaleContext,
    cache: HashMap<(u8, char), Option<GlyphQuad>>,
    /// Cache do mesmo glifo resolvido por (face, gid) — é o que o shaping usa.
    cache_gid: HashMap<(u8, u16), Option<GlyphQuad>>,
    /// Shaping por trecho de linha (ligaduras e o que mais a fonte fizer).
    shaper: crate::shape::Shaper,
    /// Modo de ligadura vindo da configuração.
    ligatures: bzmux_core::config::Ligatures,
    instances: Vec<Instance>,
    pub palette: Vec<(u8, u8, u8)>,
    /// Tema efetivo (config + defaults); o `build` resolve célula por célula.
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub cursor: (u8, u8, u8),
    /// Margem em px em volta da grade (`window_padding_width`).
    pub pad: f32,
    /// Altura da barra de abas em px (0 = uma aba só, sem barra). Ela empurra
    /// a grade para baixo: o `origin` do shader é `pad + bar`.
    bar: f32,
    /// Largura da sidebar de workspaces em px (0 = um workspace só). Empurra
    /// a grade para a direita: o `origin` vira `pad + side`.
    side: f32,
    pub cursor_shape: CursorShape,
    /// Onde o texto acaba e as imagens entram (as imagens ficam por cima do
    /// texto e abaixo do cursor/cromo, como no kitty).
    img_split: usize,
    img_pipeline: wgpu::RenderPipeline,
    img_buf: wgpu::Buffer,
    img_instances: Vec<Instance>,
    /// Id da imagem de cada instância (a textura é ligada por id).
    img_ids: Vec<u32>,
    img_layout: wgpu::BindGroupLayout,
    img_sampler: wgpu::Sampler,
    /// Atlas RGBA dos glifos coloridos (emoji). Prateleira igual à da máscara,
    /// mas com cor: emoji não aceita tingimento.
    emoji: Atlas,
    emoji_bind: wgpu::BindGroup,
    emoji_instances: Vec<Instance>,
    emoji_buf: wgpu::Buffer,
    emoji_cap: usize,
    /// Quads de emoji da linha em montagem (y relativo, como a máscara).
    emoji_row: Vec<Instance>,
    /// Textura por imagem (id → bind group). O bind group segura a textura viva.
    img_cache: HashMap<u32, ImgTex>,
    /// Instâncias da grade, guardadas **por linha**: só a linha suja é
    /// remontada (A17/G17). A lista final (`instances`) é a concatenação delas
    /// mais as imagens, o cromo e o cursor, na ordem de blend.
    por_linha: Vec<RowQuads>,
    /// Signature do último frame: o que mudou de tela inteira (ver `dano`).
    assinatura: crate::damage::Signature,
    /// Geração do tema: sobe a cada `apply_theme` e força rebuild completo.
    tema_gen: u64,
    /// Contadores de `state` (prof): prova do dano sem olhar a tela.
    pub draws: u64,
    /// Resultado do último frame ("drawn"/"skipped"/"occluded"...): a primeira
    /// pergunta quando a janela não atualiza é se o frame saiu.
    pub ultimo: &'static str,
    /// Decisão do dano no último frame (`all`/`rows: N`): responde na hora por
    /// que `rows_built` cresce demais.
    pub last_damage: String,
    /// Quantos frames foram "tudo" e a maior faixa já remontada: separa
    /// "o dano não decidiu" de "decidiu e o núcleo sujou tudo".
    pub frames_all: u64,
    pub rows_max: usize,
    /// Por que o último frame foi "tudo": `global` (dano do núcleo) ou
    /// `signature` (algo de tela inteira mudou). Sem isso o contador só diz
    /// que aconteceu, não quem pediu.
    pub all_reason: &'static str,
    pub linhas_remontadas: u64,
    pub celulas_remontadas: u64,
}

struct ImgTex {
    versao: u64,
    bind: wgpu::BindGroup,
}

/// Escolhe o adaptador com uma regra explícita, em vez de deixar o `wgpu`
/// pegar o primeiro que aparecer.
///
/// Regra: adaptador de hardware (discrete/integrated/virtual) ganha de
/// adaptador de CPU (llvmpipe/lavapipe: VM, X remoto, container sem
/// `/dev/dri`); empatado, vale a ordem do `wgpu`. A lista existe para o caso de
/// CPU parar de ser escolhido por acidente — e para registrar o que se aprendeu
/// de cada backend em DEBUG.md.
///
/// `WGPU_BACKEND` continua mandando: quem pediu um backend explícito recebe
/// aquele backend, ou o erro de sempre.
fn escolhe_adaptador(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, String> {
    let pedido = std::env::var("WGPU_BACKEND").unwrap_or_default();
    let todos = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let candidatos: Vec<wgpu::Adapter> = todos
        .into_iter()
        .filter(|a| a.is_surface_supported(surface))
        .collect();
    let de_cpu = |a: &wgpu::Adapter| a.get_info().device_type == wgpu::DeviceType::Cpu;
    let escolhido = if pedido.is_empty() {
        // hardware primeiro; sem hardware, a ordem natural do `wgpu`
        // (Vulkan antes de GL). Não se prefere GL para CPU: em VM sem
        // `/dev/dri` o GL/llvmpipe depende de MIT-SHM para chegar na janela e,
        // quando o SHM não vai, a janela fica vazia, enquanto o
        // Vulkan/llvmpipe desenha pelo Present.
        candidatos
            .iter()
            .find(|a| !de_cpu(a))
            .or_else(|| candidatos.first())
    } else {
        candidatos.first()
    };
    let escolhido = escolhido.ok_or_else(|| {
        if candidatos.is_empty() {
            "adapter: nenhum adaptador serve para a superfície".to_string()
        } else {
            "adapter: nenhum adaptador utilizável".to_string()
        }
    })?;
    Ok(escolhido.clone())
}

impl Renderer {
    #[allow(clippy::too_many_lines)] // setup de GPU é uma sequência única, partir não ajuda
    pub fn new(
        window: &Arc<Window>,
        fonts: Fonts,
        vsync: bool,
        cfg: &Config,
    ) -> Result<Renderer, String> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        // o display handle não é opcional para todo backend: o EGL (backend
        // GL) precisa dele para casar a superfície com o display, e sem isto
        // `WGPU_BACKEND=gl` morre com "gl not compatible with provided
        // surface". Vulkan não reclama, então o defeito só aparece na máquina
        // que não tem Vulkan — justamente a que precisava do GL.
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("surface: {e}"))?;
        let adapter = escolhe_adaptador(&instance, &surface)?;
        // qual backend/placa foi escolhido: é a primeira pergunta quando a
        // janela não desenha em outra máquina (ver DEBUG.md)
        let info = adapter.get_info();
        eprintln!(
            "bzmux: GPU: {:?} {} (driver {})",
            info.backend, info.name, info.driver
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bzmux"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        // formato não-srgb: as cores da célula já são sRGB e não devem ser
        // linearizadas de novo pelo alvo
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: w,
            height: h,
            present_mode,
            // `Opaque` explícito: pegar o primeiro da lista deu janela
            // translúcida no backend GL (visual ARGB com o fundo vazando o que
            // está atrás). Terminal é opaco; transparência não é paridade.
            alpha_mode: if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                caps.alpha_modes[0]
            },
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bzmux"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let atlas = Atlas::new(&device);
        let (atlas_layout, atlas_bind) = bind_atlas(&device, &atlas);
        let emoji = Atlas::new_rgba(&device);
        let emoji_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("emoji-instances"),
            size: (1 << 16) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let gl = globals_layout(&device);
        let pad = cfg.padding as f32;
        let globals = globals_buffer(&device, w as f32, h as f32, pad);
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bind"),
            layout: &gl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bzmux-layout"),
            bind_group_layouts: &[Some(&gl), Some(&atlas_layout)],
            immediate_size: 0,
        });

        let attrs = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0u64,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16u64,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 32u64,
                shader_location: 2,
            },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bzmux-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // pipeline das imagens: mesmo layout de globals/atlas e o mesmo
        // formato de instância, só troca o shader (amostra RGB em vez da
        // máscara de cobertura)
        let img_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bzmux-image"),
            source: wgpu::ShaderSource::Wgsl(include_str!("image.wgsl").into()),
        });
        let (img_layout, img_sampler) = crate::atlas::bind_image(&device);
        // o emoji usa a pipeline de imagem (textura filtrável), então o bind
        // group dele tem que seguir o mesmo layout, não o do atlas de máscara
        let emoji_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("emoji-bind"),
            layout: &img_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&emoji.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&img_sampler),
                },
            ],
        });
        let img_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bzmux-image-layout"),
            bind_group_layouts: &[Some(&gl), Some(&img_layout)],
            immediate_size: 0,
        });
        let img_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bzmux-image-pipeline"),
            layout: Some(&img_pl),
            vertex: wgpu::VertexState {
                module: &img_shader,
                entry_point: Some("vs_img"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &img_shader,
                entry_point: Some("fs_img"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let img_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("img-instances"),
            size: 1 << 16,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let inst_cap = 1 << 20;
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: inst_cap as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Renderer {
            surface,
            device,
            queue,
            config,
            pipeline,
            globals,
            globals_bind,
            atlas_bind,
            emoji,
            emoji_bind,
            emoji_instances: Vec::new(),
            emoji_buf,
            emoji_cap: 1 << 16,
            emoji_row: Vec::new(),
            inst_buf,
            inst_cap,
            atlas,
            fonts,
            scale_ctx: ScaleContext::new(),
            cache: HashMap::new(),
            cache_gid: HashMap::new(),
            shaper: crate::shape::Shaper::new(),
            ligatures: bzmux_core::config::Ligatures::default(),
            instances: Vec::with_capacity(4096),
            palette: cfg.palette256(),
            fg: tema(cfg, cfg.fg, FG),
            bg: tema(cfg, cfg.bg, BG),
            cursor: tema(cfg, cfg.cursor, FG),
            pad: cfg.padding as f32,
            bar: 0.0,
            side: 0.0,
            cursor_shape: cfg.cursor_shape,
            img_split: 0,
            img_pipeline,
            img_buf,
            img_instances: Vec::new(),
            img_ids: Vec::new(),
            img_layout,
            img_sampler,
            img_cache: HashMap::new(),
            por_linha: Vec::new(),
            assinatura: crate::damage::Signature::default(),
            tema_gen: 0,
            draws: 0,
            ultimo: "nenhum",
            last_damage: String::new(),
            frames_all: 0,
            rows_max: 0,
            all_reason: "",
            linhas_remontadas: 0,
            celulas_remontadas: 0,
        })
    }

    /// Tema novo sem recriar a GPU (auto-reload e `OSC 4/10/11/12`).
    pub fn apply_theme(&mut self, t: &Theme) {
        // qualquer coisa que mude cor de célula invalida o damage: as linhas
        // limpas ficariam com a paleta velha
        self.tema_gen = self.tema_gen.wrapping_add(1);
        self.palette = t.palette256();
        self.fg = t.rgb(t.fg, FG);
        self.bg = t.rgb(t.bg, BG);
        self.cursor = t.rgb(t.cursor, FG);
        self.write_globals();
    }

    /// O que não é cor: forma do cursor, margem e ligaduras (só muda por
    /// arquivo).
    pub fn apply_layout(&mut self, cfg: &Config) {
        self.cursor_shape = cfg.cursor_shape;
        self.pad = cfg.padding as f32;
        if self.ligatures != cfg.ligatures {
            // plano shapeado é por modo: trocar o modo invalida o cache
            self.ligatures = cfg.ligatures;
            self.shaper = crate::shape::Shaper::new();
            // o dano compara `tema_gen`: mexer nele força remontar a grade
            self.tema_gen = self.tema_gen.wrapping_add(1);
        }
        self.write_globals();
    }

    fn write_globals(&self) {
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::cast_slice(&[
                self.config.width as f32,
                self.config.height as f32,
                self.pad + self.side,
                self.pad + self.bar,
            ]),
        );
    }

    /// Reserva a faixa da barra de abas (0 = sem barra). Chamar sempre que o
    /// número de abas muda; a grade já vem com a altura descontada.
    pub fn set_bar(&mut self, h: f32) {
        if (h - self.bar).abs() < 0.01 {
            return;
        }
        self.bar = h;
        self.write_globals();
    }

    /// Reserva a coluna da sidebar de workspaces (0 = um workspace só).
    pub fn set_side(&mut self, w: f32) {
        if (w - self.side).abs() < 0.01 {
            return;
        }
        self.side = w;
        self.write_globals();
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 || (w == self.config.width && h == self.config.height) {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::cast_slice(&[
                w as f32,
                h as f32,
                self.pad + self.side,
                self.pad + self.bar,
            ]),
        );
    }

    #[inline]
    fn color(&self, c: Color, default: (u8, u8, u8)) -> [f32; 3] {
        let (r, g, b) = bzmux_core::color::resolve(c, &self.palette, default);
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ]
    }

    /// Quad a partir do bitmap, no atlas certo (RGBA para emoji, máscara para
    /// o resto). É o único lugar que escreve nos dois atlas.
    fn quad_of(&mut self, b: &glyph::GlyphBitmap, color: bool) -> Option<GlyphQuad> {
        let uv = if color {
            self.emoji.add(b.w, b.h, &b.data)
        } else {
            self.atlas.add(b.w, b.h, &b.data)
        }?;
        Some(GlyphQuad {
            left: b.left,
            top: b.top,
            w: b.w,
            h: b.h,
            uv,
            color,
        })
    }

    /// Quad de um glifo **já resolvido** pelo shaping. Cache por (face, gid): o
    /// mesmo glifo pode vir de caracteres diferentes e de trechos diferentes.
    fn quad_gid(&mut self, face: usize, gid: u16) -> Option<GlyphQuad> {
        let key = (face as u8, gid);
        if let Some(v) = self.cache_gid.get(&key) {
            return *v;
        }
        let bitmap = self.fonts.bitmap_for(&mut self.scale_ctx, face, gid);
        let quad = bitmap.and_then(|(b, color)| self.quad_of(&b, color));
        self.cache_gid.insert(key, quad);
        quad
    }

    /// Quad resolvendo o caractere pela cadeia de fontes (caminho sem shaping:
    /// trecho de uma célula, ou glifo que o shaping não desenhou).
    fn glyph_quad(&mut self, ch: char, bold: bool, italic: bool) -> Option<GlyphQuad> {
        // a chave é o estilo pedido, não a face escolhida: quem decide a face é
        // a cadeia de fontes, e o resultado já vem resolvido
        let key = (u8::from(bold) | (u8::from(italic) << 1), ch);
        if let Some(v) = self.cache.get(&key) {
            return *v;
        }
        let bitmap = self
            .fonts
            .glyph_bitmap(&mut self.scale_ctx, ch, bold, italic);
        let quad = match bitmap {
            Some((b, color, _, _)) => self.quad_of(&b, color),
            None => None,
        };
        self.cache.insert(key, quad);
        quad
    }

    fn push_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let uv = self.atlas.uv_white;
        self.instances.push(Instance {
            rect: [x, y, w, h],
            color,
            uv,
        });
    }

    /// Selo de uma linha no canto superior direito (indicador de histórico).
    /// Desenhado por último: tem que ficar por cima do que estiver lá.
    /// Barra de abas: ` 1 zsh │ 2 vim  ` na faixa acima da grade.
    ///
    /// A aba ativa fica invertida (fundo = cor do texto). Aba com BELL recente
    /// troca a cor de destaque — é o que faz "aquele terminal apitou" ser
    /// visível sem olhar para ele.
    fn tabbar(&mut self, tabs: &[TabInfo], cols: usize) {
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let ascent = self.fonts.ascent;
        // a barra começa à direita da sidebar: as duas dividem a linha 1, e a
        // sidebar não fica com um buraco em cima (o cartão é a altura de 2,4
        // linhas, então ela passa por baixo da barra de qualquer jeito)
        let x0 = 0.0;
        let largo = cols as f32 * cw;
        let fundo = self.color(Color::Idx(0), self.bg);
        // texto sobre a aba ativa usa a cor de fundo da grade (inversão)
        let sobre = self.color(Color::Default, self.bg);
        // `origin.y` é `pad + bar`: para a barra cair na faixa reservada (e não
        // sobre a primeira linha da grade) ela é desenhada em y = -bar
        let y0 = -self.bar;
        self.push_rect(x0, y0, largo, chh, [fundo[0], fundo[1], fundo[2], 1.0]);
        let mut x = x0;
        for (i, t) in tabs.iter().enumerate() {
            let rotulo = format!(" {} {} ", i + 1, t.titulo);
            let n = rotulo.chars().count();
            let largura = (n as f32 * cw).min(largo - x);
            if largura <= 0.0 {
                break;
            }
            let (bg, fg) = if t.ativa {
                (self.color(Color::Default, self.fg), sobre)
            } else if t.alerta {
                (self.color(Color::Idx(3), self.fg), sobre)
            } else {
                (fundo, self.color(Color::Idx(8), self.fg))
            };
            // um retângulo por aba (não um por letra): o fundo do rótulo é uma
            // peça só, e o texto é desenhado por cima
            self.push_rect(x, y0, largura, chh, [bg[0], bg[1], bg[2], 1.0]);
            for (k, ch) in rotulo.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let px = x + k as f32 * cw;
                if px + cw > x + largura {
                    break;
                }
                if let Some(q) = self.glyph_quad(ch, false, false) {
                    self.instances.push(Instance {
                        rect: [
                            px + q.left as f32,
                            y0 + ascent - q.top as f32,
                            q.w as f32,
                            q.h as f32,
                        ],
                        color: [fg[0], fg[1], fg[2], 1.0],
                        uv: q.uv,
                    });
                }
            }
            x += largura;
        }
    }

    /// Sidebar de workspaces (I6): uma coluna à esquerda com um cartão por
    /// workspace. Ocupa a faixa que o `origin` reservou (x = -side) e começa na
    /// linha 1, junto com a barra de abas (y = -bar).
    fn sidebar(&mut self, cards: &[WsCard]) {
        if self.side <= 0.0 {
            return;
        }
        let chh = self.fonts.ch;
        let card_h = chh * 2.4;
        let x0 = -self.side;
        let largura = self.side - self.pad;
        let fundo = self.color(Color::Idx(0), self.bg);
        let sobre = self.color(Color::Default, self.bg);
        let topo = -self.bar;
        for (i, c) in cards.iter().enumerate() {
            let y0 = topo + i as f32 * card_h;
            let ativo = c.ativa;
            let (bg, fg) = if ativo {
                (self.color(Color::Default, self.fg), sobre)
            } else {
                (fundo, self.color(Color::Idx(8), self.fg))
            };
            self.push_rect(x0, y0, largura, card_h, [bg[0], bg[1], bg[2], 1.0]);
            // linha 1: "n nome"
            let n = cards.len();
            let nome = format!("{} {}", i + 1, c.nome);
            self.text_line(x0, y0 + 3.0, &nome, fg, largura);
            // linha 2: "titulo (k abas)"
            let info = format!(
                "{} ({} aba{})",
                c.titulo,
                c.n_tabs,
                if c.n_tabs == 1 { "" } else { "s" }
            );
            self.text_line(x0, y0 + chh + 3.0, &info, fg, largura);
            let _ = n;
        }
    }

    /// Uma linha de texto da sidebar, cortada na largura do cartão.
    fn text_line(&mut self, x0: f32, y: f32, texto: &str, fg: [f32; 3], largura: f32) {
        let cw = self.fonts.cw;
        let ascent = self.fonts.ascent;
        let cabe = (largura / cw).floor().max(1.0) as usize;
        // reticências em vez de corte seco: o cartão estreito não come o fim
        // do título sem avisar (o Python faz o mesmo com `ellipsize`)
        let texto: String = if texto.chars().count() > cabe {
            let mut t: String = texto.chars().take(cabe.saturating_sub(1)).collect();
            t.push('…');
            t
        } else {
            texto.to_string()
        };
        for (i, ch) in texto.chars().take(cabe).enumerate() {
            if ch == ' ' {
                continue;
            }
            if let Some(q) = self.glyph_quad(ch, false, false) {
                self.instances.push(Instance {
                    rect: [
                        x0 + self.pad + i as f32 * cw + q.left as f32,
                        y + ascent - q.top as f32,
                        q.w as f32,
                        q.h as f32,
                    ],
                    color: [fg[0], fg[1], fg[2], 1.0],
                    uv: q.uv,
                });
            }
        }
    }

    /// Toast (I12): uma faixa na última linha, com as cores invertidas.
    ///
    /// Fica embaixo porque em cima está a barra de abas (o cabeçalho é dela);
    /// o aviso é do terminal, não do programa, então cobre a última linha da
    /// grade em vez de disputar o topo. O texto é cortado na largura da
    /// grade — quebra de linha num aviso transitório só confunde.
    fn toast(&mut self, text: &str, cols: usize, rows: usize) {
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let ascent = self.fonts.ascent;
        let bg = self.color(Color::Default, self.fg);
        let fg = self.color(Color::Default, self.bg);
        let n = cols.saturating_sub(2);
        let curto: String = text.chars().take(n).collect();
        let w = curto.chars().count();
        if w == 0 {
            return;
        }
        // margem de meia célula de cada lado para o texto não tocar a borda
        let x0 = -self.side;
        let y0 = rows.saturating_sub(1) as f32 * chh;
        self.push_rect(x0, y0, (w + 2) as f32 * cw, chh, [bg[0], bg[1], bg[2], 1.0]);
        for (i, ch) in curto.chars().enumerate() {
            if let Some(q) = self.glyph_quad(ch, false, false) {
                self.instances.push(Instance {
                    rect: [
                        x0 + (i + 1) as f32 * cw + q.left as f32,
                        y0 + ascent - q.top as f32,
                        q.w as f32,
                        q.h as f32,
                    ],
                    color: [fg[0], fg[1], fg[2], 1.0],
                    uv: q.uv,
                });
            }
        }
    }

    fn badge(&mut self, text: &str, cols: usize) {
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let ascent = self.fonts.ascent;
        let n = text.chars().count();
        let x0 = cols.saturating_sub(n + 1);
        let bg = self.color(Color::Default, self.fg);
        let fg = self.color(Color::Default, self.bg);
        self.push_rect(
            x0 as f32 * cw,
            0.0,
            n as f32 * cw,
            chh,
            [bg[0], bg[1], bg[2], 1.0],
        );
        for (i, ch) in text.chars().enumerate() {
            if let Some(q) = self.glyph_quad(ch, false, false) {
                self.instances.push(Instance {
                    rect: [
                        (x0 + i) as f32 * cw + q.left as f32,
                        ascent - q.top as f32,
                        q.w as f32,
                        q.h as f32,
                    ],
                    color: [fg[0], fg[1], fg[2], 1.0],
                    uv: q.uv,
                });
            }
        }
    }

    /// Glifo + decoração de uma célula. A ordem aqui é a ordem de blend:
    /// tudo do texto sai antes do cursor.
    /// `glifo` = glifo já resolvido pelo shaping (`face`, `gid`, deslocamento em
    /// px). `None` manda resolver o caractere pela cadeia — é o caminho de
    /// trecho de uma célula. Glifo do shaping que não desenha (face que mapeia e
    /// não tem bitmap) também cai na cadeia.
    fn draw_cell(
        &mut self,
        cell: &Cell,
        x: usize,
        py: f32,
        swap: bool,
        glifo: Option<(usize, u16, f32)>,
    ) {
        use bzmux_core::cell::{
            BOLD, FAINT, ITALIC, OVERLINE, STRIKE, UL_CURLY, UL_DASHED, UL_DOTTED, UL_DOUBLE,
        };
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let ascent = self.fonts.ascent;
        let fg = if swap { cell.bg } else { cell.fg };
        let mut c = self.color(fg, if swap { self.bg } else { self.fg });
        // esmaecido: o mesmo tom com menos opacidade (Pango usava alpha 0.55)
        let alpha = if cell.flags & FAINT != 0 { 0.55 } else { 1.0 };

        if cell.ch != ' ' {
            let bold = cell.flags & BOLD != 0;
            let italic = cell.flags & ITALIC != 0;
            let quad = glifo
                .and_then(|(face, gid, _)| self.quad_gid(face, gid))
                .or_else(|| self.glyph_quad(cell.ch, bold, italic));
            let desloc = glifo.map_or(0.0, |(_, _, dx)| dx);
            if let Some(q) = quad {
                let gx = x as f32 * cw + desloc + q.left as f32;
                let gy = py + ascent - q.top as f32;
                // emoji colorido não recebe tingimento: só o alfa da célula
                let (cor, alvo) = if q.color {
                    ([1.0, 1.0, 1.0, alpha], &mut self.emoji_row)
                } else {
                    ([c[0], c[1], c[2], alpha], &mut self.instances)
                };
                alvo.push(Instance {
                    rect: [gx, gy, q.w as f32, q.h as f32],
                    color: cor,
                    uv: q.uv,
                });
            }
        }

        // cor da decoração: a do texto, salvo SGR 58 (que não inverte)
        if cell.ul_color != Color::Default {
            c = self.color(cell.ul_color, self.fg);
        }
        if cell.ul != bzmux_core::cell::UL_NONE {
            let base = py + ascent + 1.0;
            let line = |r: &mut Self, x0: f32, w: f32, y: f32, h: f32| {
                r.push_rect(x0, y, w, h, [c[0], c[1], c[2], 1.0]);
            };
            match cell.ul {
                UL_DOUBLE => {
                    line(self, x as f32 * cw, cw, base, 1.0);
                    line(self, x as f32 * cw, cw, base + 2.0, 1.0);
                },
                UL_CURLY => {
                    // aproximação senoidal em 4 pedaços; sem geometria curva
                    // no pipeline de retângulos
                    for k in 0..4 {
                        let dy = if k % 2 == 0 { 0.0 } else { 1.5 };
                        line(
                            self,
                            x as f32 * cw + k as f32 * cw / 4.0,
                            cw / 4.0,
                            base + dy,
                            1.5,
                        );
                    }
                },
                UL_DOTTED => {
                    let mut dx = 0.0;
                    while dx + 1.0 <= cw {
                        line(self, x as f32 * cw + dx, 1.0, base, 1.5);
                        dx += 3.0;
                    }
                },
                UL_DASHED => {
                    let mut dx = 0.0;
                    while dx + 2.0 <= cw {
                        line(self, x as f32 * cw + dx, 2.0, base, 1.5);
                        dx += 4.0;
                    }
                },
                _ => line(self, x as f32 * cw, cw, base, 1.0), // UL_SINGLE
            }
        }

        // riscado no meio da altura de x (não no meio da célula: o vazio do
        // descendente deixaria o risco alto demais)
        if cell.flags & STRIKE != 0 {
            let y = py + ascent - chh * 0.18;
            self.push_rect(x as f32 * cw, y, cw, 1.0, [c[0], c[1], c[2], 1.0]);
        }
        if cell.flags & OVERLINE != 0 {
            self.push_rect(x as f32 * cw, py + 1.0, cw, 1.0, [c[0], c[1], c[2], 1.0]);
        }
    }

    /// Monta as instâncias do frame. Público para o teste de sanidade do formato.
    #[allow(clippy::too_many_lines)] // um laço por categoria, na ordem de blend
    #[allow(clippy::many_single_char_names)] // x/y/cw/ch/k: geometria de célula
    /// Monta as instâncias das imagens visíveis e garante a textura de cada uma.
    ///
    /// H12: a colocação vive em coordenadas da **grade**, então a linha na tela
    /// é `linha + view_off` — sem isso a imagem ficava parada enquanto o texto
    /// rolava por baixo dela.
    fn build_images(&mut self, f: &Frame<'_>) {
        let term = f.term;
        let (cw, chh) = (self.fonts.cw, self.fonts.ch);
        let view = i32::try_from(f.view_off).unwrap_or(0);
        let linhas = i32::try_from(term.rows).unwrap_or(0);
        let colunas = term.cols;
        let mut quads: Vec<(u32, Instance)> = Vec::new();
        for pl in term.graphics.placements() {
            let y = pl.linha + view;
            // fora da tela (por cima ou por baixo) não vira instância
            if y >= linhas || y + i32::from(pl.rows) <= 0 || usize::from(pl.col) >= colunas {
                continue;
            }
            let Some(img) = term.graphics.images().get(&pl.image_id).cloned() else {
                continue;
            };
            if !self.garante_textura(&img) {
                continue; // imagem ilegível (PNG torto) não derruba o frame
            }
            quads.push((
                pl.image_id,
                Instance {
                    rect: [
                        f32::from(pl.col) * cw + pl.dx as f32,
                        y as f32 * chh + pl.dy as f32,
                        f32::from(pl.cols.max(1)) * cw,
                        f32::from(pl.rows.max(1)) * chh,
                    ],
                    color: [1.0, 1.0, 1.0, 1.0],
                    uv: [0.0, 0.0, 1.0, 1.0],
                },
            ));
        }
        // imagens que o app apagou não precisam de textura viva
        self.img_cache
            .retain(|id, _| term.graphics.images().contains_key(id));
        self.img_ids = quads.iter().map(|(id, _)| *id).collect();
        self.img_instances = quads.into_iter().map(|(_, i)| i).collect();
        let need = self.img_instances.len() * std::mem::size_of::<Instance>();
        if need > self.img_buf.size() as usize {
            self.img_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("img-instances"),
                size: (need * 2) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if need > 0 {
            self.queue
                .write_buffer(&self.img_buf, 0, bytemuck::cast_slice(&self.img_instances));
        }
    }

    /// Sobe a textura da imagem se ela ainda não está no cache (ou se o app
    /// reenviou com o mesmo id e a versão mudou).
    fn garante_textura(&mut self, img: &bzmux_core::graphics::Image) -> bool {
        if self
            .img_cache
            .get(&img.id)
            .is_some_and(|t| t.versao == img.versao)
        {
            return true;
        }
        let Some((w, h, rgba)) = decodifica(img) else {
            return false;
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("img"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("img-bind"),
            layout: &self.img_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.img_sampler),
                },
            ],
        });
        self.img_cache.insert(
            img.id,
            ImgTex {
                versao: img.versao,
                bind,
            },
        );
        true
    }

    /// Cursor (bloco, sublinhado ou barra) — por cima do texto e das imagens.
    fn build_cursor(&mut self, f: &Frame<'_>) {
        let term = f.term;
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let view_off = f.view_off;
        let cols = term.cols;
        let blink = f.blink;
        if blink.cursor
            && view_off == 0
            && term.cursor_visible
            && term.cy < term.rows
            && term.cx < cols
        {
            let cx = term.cx as f32 * cw;
            let cy = term.cy as f32 * chh;
            let style = term.app_cursor_style.map_or_else(
                || match self.cursor_shape {
                    CursorShape::Block => 0,
                    CursorShape::Underline => 1,
                    CursorShape::Beam => 2,
                },
                |(f, _)| f,
            );
            let cor = self.color(Color::Default, self.cursor);
            match style {
                1 | 2 => {
                    // sublinhado / barra
                    let h = if style == 1 { 2.0 } else { chh };
                    let w = if style == 1 { cw } else { 2.0 };
                    let y = if style == 1 { cy + chh - 3.0 } else { cy };
                    self.push_rect(cx, y, w, h, [cor[0], cor[1], cor[2], 0.9]);
                },
                _ => {
                    self.push_rect(cx, cy, cw, chh, [cor[0], cor[1], cor[2], 0.85]);
                },
            }
        }
    }

    pub fn build(&mut self, f: &Frame<'_>) {
        let Frame {
            aba,
            term,
            view_off,
            sel,
            blink,
            toast,
            tabs,
            sidebar,
        } = *f;
        let cols = term.cols;
        // retângulo da seleção na visão (preso à tela); fora dela nada muda
        let sel = sel.and_then(|s| s.visible(term, view_off));
        let rows = term.visible_rows(view_off);
        let ctx = RowCtx {
            cols,
            sel,
            blink,
            cursor: quebra_no_cursor(term, view_off, rows.len()),
        };

        // A17/G17: quais linhas remontar. `blink` só entra na assinatura se
        // houver célula com SGR 5 à vista — o pisca do cursor não mora aqui.
        let assinatura = crate::damage::Signature {
            aba,
            cols,
            rows: rows.len(),
            view_off,
            sel,
            blink: blink.fase && Self::tem_blink(&rows),
            tema: self.tema_gen,
            fg: self.fg,
            bg: self.bg,
        };
        let changed = self.assinatura != assinatura;
        let dano = crate::damage::decide(
            &assinatura,
            &mut self.assinatura,
            term.dirty_all,
            &term.dirty,
            rows.len(),
        );
        if matches!(dano, crate::damage::Damage::All) {
            self.frames_all = self.frames_all.saturating_add(1);
            self.all_reason = if term.dirty_all {
                "global"
            } else if changed {
                "signature"
            } else {
                "?"
            };
        }
        self.last_damage = match &dano {
            crate::damage::Damage::All => "all".to_string(),
            crate::damage::Damage::Rows(ls) if ls.is_empty() => "rows: 0".to_string(),
            #[allow(clippy::cast_precision_loss)]
            crate::damage::Damage::Rows(ls) if ls.len() > self.rows_max => {
                self.rows_max = ls.len();
                format!("rows: {}", ls.len())
            },
            crate::damage::Damage::Rows(ls) => format!("rows: {}", ls.len()),
        };
        if self.por_linha.len().ne(&rows.len()) {
            // grade mudou de tamanho: nada de reaproveitar faixa antiga
            self.assinatura = assinatura;
            self.por_linha = vec![RowQuads::default(); rows.len()];
            for (y, row) in rows.iter().enumerate() {
                self.por_linha[y] = self.linha(row, y, ctx);
            }
        } else {
            let shift = if term.scroll_conflict {
                0
            } else {
                term.scroll_shift
            };
            self.rebuild_rows(&dano, &rows, ctx, shift);
        }

        self.concat_linhas();

        // aqui acaba o texto da grade: as imagens (H1–H13) entram por cima
        // dele e abaixo do cromo/cursor, que é a ordem do kitty
        self.img_split = self.instances.len();
        self.build_images(f);

        // 2b) barra de abas: fica na faixa que o `origin` reservou acima da
        // grade, então não precisa de conta de recorte
        if let Some(tabs) = tabs {
            self.tabbar(tabs, cols);
        }
        // 2c) sidebar de workspaces: coluna à esquerda, na faixa reservada
        if let Some(cards) = sidebar {
            self.sidebar(cards);
        }

        // 3) cursor (por cima; a ordem das instâncias é a ordem de blend)
        self.build_cursor(f);

        // 4) indicador de histórico (D8): "↑N" — sem ele, rolar para trás é
        // indistinguível de o app ter parado de escrever
        if view_off > 0 {
            let txt = format!("↑{view_off}");
            self.badge(&txt, cols);
        }

        // 5) toast (I12/K13): aviso do próprio terminal, por cima de tudo. Some
        // sozinho; quem controla o tempo é o app.
        if let Some(t) = toast {
            self.toast(t, cols, rows.len());
        }
    }

    /// Lista final na ordem de blend: grade (por linha) → imagens → cromo →
    /// cursor → badge → toast. As linhas prontas estão em y = 0; a posição
    /// vertical entra aqui, e é isso que deixa a rolagem só deslocar linha.
    fn concat_linhas(&mut self) {
        self.instances.clear();
        self.emoji_instances.clear();
        let chh = self.fonts.ch;
        for (y, linha) in self.por_linha.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let dy = y as f32 * chh;
            for inst in &linha.mask {
                let mut copia = *inst;
                copia.rect[1] += dy;
                self.instances.push(copia);
            }
            for inst in &linha.emoji {
                let mut copia = *inst;
                copia.rect[1] += dy;
                self.emoji_instances.push(copia);
            }
        }
    }

    /// `true` se alguma célula visível tem SGR 5 (só aí a fase do pisca obriga
    /// a remontar a grade).
    fn tem_blink(rows: &[&bzmux_core::grid::Row]) -> bool {
        rows.iter().any(|r| {
            r.cells
                .iter()
                .any(|c| c.flags & bzmux_core::cell::BLINK != 0)
        })
    }

    /// Monta as instâncias de uma linha sem tocar na lista do frame.
    /// Remonta o que o dano pediu. Rolagem de tela inteira desloca as linhas
    /// prontas: só as que entram (topo ou fim) precisam de glifo novo.
    fn rebuild_rows(
        &mut self,
        dano: &crate::damage::Damage,
        rows: &[&bzmux_core::grid::Row],
        ctx: RowCtx,
        shift: isize,
    ) {
        let crate::damage::Damage::Rows(linhas) = dano else {
            for (y, row) in rows.iter().enumerate() {
                self.por_linha[y] = self.linha(row, y, ctx);
            }
            return;
        };
        let mut rebuild = linhas.clone();
        if shift != 0 {
            let n = shift.unsigned_abs().min(self.por_linha.len());
            for _ in 0..n {
                if shift > 0 {
                    self.por_linha.remove(0);
                    self.por_linha.push(RowQuads::default());
                    rebuild.push(self.por_linha.len() - 1);
                } else {
                    self.por_linha.pop();
                    self.por_linha.insert(0, RowQuads::default());
                }
            }
            if shift < 0 {
                rebuild.extend(0..n);
            }
        }
        rebuild.sort_unstable();
        rebuild.dedup();
        for y in rebuild {
            if let Some(row) = rows.get(y) {
                self.por_linha[y] = self.linha(row, y, ctx);
            }
        }
    }

    /// Monta uma linha **relativa ao topo da grade** (y = 0). A posição
    /// vertical entra na concatenação, e é isso que permite rolar a tela
    /// deslocando a linha pronta em vez de remontar o glifo.
    fn linha(&mut self, row: &bzmux_core::grid::Row, y: usize, ctx: RowCtx) -> RowQuads {
        let guardado = std::mem::take(&mut self.instances);
        let guardado_cor = std::mem::take(&mut self.emoji_row);
        self.instances.clear();
        self.emoji_row.clear();
        self.pinta_linha(row, y, ctx);
        RowQuads {
            mask: std::mem::replace(&mut self.instances, guardado),
            emoji: std::mem::replace(&mut self.emoji_row, guardado_cor),
        }
    }

    /// Glifos e decoração de uma linha, agrupados em **trechos** de mesma face
    /// e estilo: é o contexto do trecho que faz a fonte trocar o glifo de cada
    /// célula pela peça da ligadura (`calt`). Trecho de uma célula não tem
    /// contexto e vai direto pela cadeia de fontes — é o caso de quem pinta
    /// célula a célula (doom-fire: 2400 trechos de 1).
    fn pinta_glifos(&mut self, row: &bzmux_core::grid::Row, y: usize, ctx: RowCtx) {
        let RowCtx {
            cols,
            sel,
            blink,
            cursor,
        } = ctx;
        let py = y as f32 * self.fonts.ch;
        let limite = cols.min(row.cells.len());
        let mut x = 0usize;
        while x < limite {
            let cell = row.cells[x];
            if cell.is_cont() || (cell.flags & bzmux_core::cell::BLINK != 0 && !blink.fase) {
                x += 1;
                continue;
            }
            let (face, bold, italic, swap) = self.estilo(&cell, sel, y, x);
            let mut cells = vec![x];
            let mut text = String::new();
            text.push(cell.ch);
            let mut fim = x + 1;
            while fim < limite {
                let c2 = row.cells[fim];
                if c2.is_cont() {
                    // a metade de um caractere largo pertence ao caractere
                    fim += 1;
                    continue;
                }
                if c2.flags & bzmux_core::cell::BLINK != 0 && !blink.fase {
                    break;
                }
                // o cursor quebra o trecho (ver `cursor` no build)
                if cursor == Some((fim, y)) || cursor == Some((fim.wrapping_sub(1), y)) {
                    break;
                }
                if self.estilo(&c2, sel, y, fim) != (face, bold, italic, swap) {
                    break;
                }
                cells.push(fim);
                text.push(c2.ch);
                fim += 1;
            }
            let glifos = match face {
                Some(face) if cells.len() > 1 => {
                    let cw = self.fonts.cw;
                    let px = self.fonts.px;
                    let plano = self.shaper.plan(
                        self.fonts.face(face),
                        &text,
                        &cells,
                        self.ligatures,
                        cw,
                        px,
                    );
                    plano.map(|p| (face, p))
                },
                _ => None,
            };
            for (i, &cx) in cells.iter().enumerate() {
                let c = row.cells[cx];
                let glifo = match &glifos {
                    Some((face, plano)) => plano.piece_at(i).map(|p| (*face, p.gid, p.x)),
                    None => None,
                };
                self.draw_cell(&c, cx, py, is_swap(&c, sel, y, cx), glifo);
            }
            x = fim;
        }
    }

    /// Desenha **uma** linha da grade (A17/G17: só roda para as linhas sujas).
    fn pinta_linha(&mut self, row: &bzmux_core::grid::Row, y: usize, ctx: RowCtx) {
        let RowCtx { cols, sel, .. } = ctx;
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let py = y as f32 * chh;

        // 1) fundos, agrupados por cor (uma instância por trecho). O inverso
        // (SGR 7) e a seleção usam a mesma regra: troca fg/bg e resolve o
        // default pelo *outro* extremo (senão seleção em célula sem cor não
        // apareceria).
        let mut x = 0usize;
        while x < cols {
            let cell = match row.cells.get(x) {
                Some(c) => *c,
                None => break,
            };
            let swap = is_swap(&cell, sel, y, x);
            let bg = if swap { cell.fg } else { cell.bg };
            let mut end = x + 1;
            while end < cols {
                let Some(c2) = row.cells.get(end) else { break };
                if is_swap(c2, sel, y, end) != swap {
                    break;
                }
                let bg2 = if swap { c2.fg } else { c2.bg };
                if bg2 != bg {
                    break;
                }
                end += 1;
            }
            if swap || bg != Color::Default {
                let c = self.color(bg, if swap { self.fg } else { self.bg });
                self.push_rect(
                    x as f32 * cw,
                    py,
                    (end - x) as f32 * cw,
                    chh,
                    [c[0], c[1], c[2], 1.0],
                );
            }
            x = end;
        }

        self.pinta_glifos(row, y, ctx);
        self.linhas_remontadas = self.linhas_remontadas.saturating_add(1);
        self.celulas_remontadas = self.celulas_remontadas.saturating_add(cols as u64);
    }

    /// Face e atributos que definem um trecho de shaping, mais a inversão
    /// (seleção/SGR 7), que troca a cor e por isso separa trechos.
    #[allow(clippy::type_complexity)]
    fn estilo(
        &self,
        cell: &bzmux_core::cell::Cell,
        sel: Option<(usize, usize, usize, usize)>,
        y: usize,
        x: usize,
    ) -> (Option<usize>, bool, bool, bool) {
        let bold = cell.flags & bzmux_core::cell::BOLD != 0;
        let italic = cell.flags & bzmux_core::cell::ITALIC != 0;
        (
            self.fonts.face_of(cell.ch, bold, italic),
            bold,
            italic,
            is_swap(cell, sel, y, x),
        )
    }

    /// Sobe os dois buffers de instância (máscara e emoji). Crescem dobrando:
    /// redimensionar no meio do desenho não pode realocar a cada frame.
    fn upload_instances(&mut self) {
        let masks = bytemuck::cast_slice::<Instance, u8>(&self.instances);
        if let Some(b) = grow_buffer(
            &self.device,
            &mut self.inst_cap,
            masks.len(),
            1 << 20,
            "instances",
        ) {
            self.inst_buf = b;
        }
        if !masks.is_empty() {
            self.queue.write_buffer(&self.inst_buf, 0, masks);
        }
        let colors = bytemuck::cast_slice::<Instance, u8>(&self.emoji_instances);
        if let Some(b) = grow_buffer(
            &self.device,
            &mut self.emoji_cap,
            colors.len(),
            1 << 16,
            "emoji-instances",
        ) {
            self.emoji_buf = b;
        }
        if !colors.is_empty() {
            self.queue.write_buffer(&self.emoji_buf, 0, colors);
        }
    }

    pub fn render(&mut self, f: &Frame<'_>) -> FrameOutcome {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.ultimo = "skipped (timeout/occluded)";
                return FrameOutcome::Skipped;
            },
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.ultimo = "reconfigure";
                return FrameOutcome::Reconfigure;
            },
            wgpu::CurrentSurfaceTexture::Validation => {
                self.ultimo = "fatal";
                return FrameOutcome::Fatal;
            },
        };

        let t0 = std::time::Instant::now();
        self.draws = self.draws.saturating_add(1);
        self.ultimo = "drawn";
        self.build(f);
        self.atlas.flush(&self.queue);
        self.emoji.flush(&self.queue);

        self.upload_instances();

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (r, g, b) = self.bg;
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bzmux-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(r) / 255.0,
                            g: f64::from(g) / 255.0,
                            b: f64::from(b) / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.globals_bind, &[]);
            rp.set_bind_group(1, &self.atlas_bind, &[]);
            rp.set_vertex_buffer(0, self.inst_buf.slice(..));
            // texto da grade → imagens → cromo e cursor. Duas faixas do mesmo
            // buffer porque a imagem tem pipeline próprio (amostra RGB).
            let split = self.img_split.min(self.instances.len()) as u32;
            if split > 0 {
                rp.draw(0..6, 0..split);
            }
            // emoji colorido: mesma pipeline de imagem, textura é o atlas RGBA
            if !self.emoji_instances.is_empty() {
                rp.set_pipeline(&self.img_pipeline);
                rp.set_bind_group(1, &self.emoji_bind, &[]);
                rp.set_vertex_buffer(0, self.emoji_buf.slice(..));
                rp.draw(0..6, 0..self.emoji_instances.len() as u32);
                rp.set_pipeline(&self.pipeline);
                rp.set_bind_group(1, &self.atlas_bind, &[]);
                rp.set_vertex_buffer(0, self.inst_buf.slice(..));
            }
            if !self.img_instances.is_empty() {
                rp.set_pipeline(&self.img_pipeline);
                rp.set_vertex_buffer(0, self.img_buf.slice(..));
                // uma draw call por imagem: cada uma tem a sua textura
                for (i, _) in self.img_instances.iter().enumerate() {
                    let Some(id) = self.img_ids.get(i).copied() else {
                        break;
                    };
                    if let Some(t) = self.img_cache.get(&id) {
                        rp.set_bind_group(1, &t.bind, &[]);
                        rp.draw(0..6, i as u32..(i as u32 + 1));
                    }
                }
                rp.set_pipeline(&self.pipeline);
                rp.set_bind_group(1, &self.atlas_bind, &[]);
                rp.set_vertex_buffer(0, self.inst_buf.slice(..));
            }
            let total = self.instances.len() as u32;
            if total > split {
                rp.draw(0..6, split..total);
            }
        }
        self.queue.submit(Some(enc.finish()));
        self.queue.present(frame);

        if prof::enabled() {
            prof::add_draw(t0.elapsed().as_nanos() as u64);
        }
        FrameOutcome::Drawn
    }
}

/// Imagem do protocolo → (largura, altura, RGBA8) para a textura.
///
/// `f=24` vira RGBA com alfa 255; `f=32` passa direto; `f=100` é PNG, que o app
/// decodifica aqui (o core não carrega decodificador de PNG).
fn decodifica(img: &bzmux_core::graphics::Image) -> Option<(u32, u32, Vec<u8>)> {
    use bzmux_core::graphics::Formato;
    let (largura, altura) = (img.width, img.height);
    if largura == 0 || altura == 0 {
        return None;
    }
    match img.formato {
        Formato::Rgba => {
            let need = (largura as usize) * (altura as usize) * 4;
            (img.data.len() >= need).then(|| (largura, altura, img.data[..need].to_vec()))
        },
        Formato::Rgb => {
            let need = (largura as usize) * (altura as usize) * 3;
            if img.data.len() < need {
                return None;
            }
            let mut out = Vec::with_capacity((largura as usize) * (altura as usize) * 4);
            for px in img.data[..need].as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Some((largura, altura, out))
        },
        Formato::Png => {
            let mut leitor = png::Decoder::new(std::io::Cursor::new(&img.data))
                .read_info()
                .ok()?;
            let mut buf = vec![0; leitor.output_buffer_size()];
            let info = leitor.next_frame(&mut buf).ok()?;
            let (pw, ph) = (info.width, info.height);
            let mut out = Vec::with_capacity((pw as usize) * (ph as usize) * 4);
            match info.color_type {
                png::ColorType::Rgba => out.extend_from_slice(&buf[..info.buffer_size()]),
                png::ColorType::Rgb => {
                    for px in buf[..info.buffer_size()].as_chunks::<3>().0 {
                        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                    }
                },
                png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha => {
                    let n = if info.color_type == png::ColorType::Grayscale {
                        1
                    } else {
                        2
                    };
                    for px in buf[..info.buffer_size()].chunks_exact(n) {
                        let a = if n == 2 { px[1] } else { 255 };
                        out.extend_from_slice(&[px[0], px[0], px[0], a]);
                    }
                },
                png::ColorType::Indexed => return None, // raro: o icat manda RGBA
            }
            Some((pw, ph, out))
        },
    }
}

/// O que fazer depois de um frame (o `get_current_texture` do wgpu 30 devolve
/// um enum em vez de `Result`).
/// Estado do pisca neste frame (G12).
///
/// O que o desenho de **uma linha** precisa saber do frame. Existe para não
/// arrastar cinco parâmetros por `build` → `rebuild_rows` → `linha`.
#[derive(Clone, Copy)]
struct RowCtx {
    cols: usize,
    sel: Option<(usize, usize, usize, usize)>,
    blink: Blink,
    /// Célula do cursor, quando ele está nesta visão (quebra ligadura).
    cursor: Option<(usize, usize)>,
}

/// O cursor quebra a ligadura: a célula dele é shapeada sozinha, senão ele pinta
/// no meio de um glifo que cobre mais de uma célula — é o
/// `disable_ligatures=cursor` do kitty. Fora da visão (rolado) não há cursor.
fn quebra_no_cursor(term: &Term, view_off: usize, rows: usize) -> Option<(usize, usize)> {
    (view_off == 0 && term.cursor_visible && term.cy < rows).then_some((term.cx, term.cy))
}

/// `fase` alterna a cada 500 ms e vale para o texto com SGR 5; `cursor` já vem
/// resolvido pelo app (config `cursor_blink` + `CSI 6 q` do programa).
#[derive(Clone, Copy, Default)]
pub struct Blink {
    pub fase: bool,
    pub cursor: bool,
}

/// O que a barra precisa saber de cada aba (o renderer não conhece `Tab`).
pub struct TabInfo {
    pub titulo: String,
    pub ativa: bool,
    /// BELL recente: o rótulo ganha destaque por ~800 ms (I13).
    pub alerta: bool,
}

/// Tudo que um frame precisa, num pacote só: a assinatura de `build`/`render`
/// crescia um parâmetro por fatia (abas, sidebar, toast...).
pub struct Frame<'a> {
    /// Identidade da aba desenhada (ver `dano`).
    pub aba: u64,
    pub term: &'a Term,
    pub view_off: usize,
    pub sel: Option<Span>,
    pub blink: Blink,
    pub toast: Option<&'a str>,
    pub tabs: Option<&'a [TabInfo]>,
    pub sidebar: Option<&'a [WsCard]>,
}

/// O que a sidebar precisa de cada workspace (I6).
pub struct WsCard {
    pub nome: String,
    pub titulo: String,
    pub n_tabs: usize,
    pub ativa: bool,
}

pub enum FrameOutcome {
    Drawn,
    Skipped,
    Reconfigure,
    Fatal,
}

/// Cor do tema: `Default` cai no padrão do programa, o resto resolve pela
/// paleta (as 16 primeiras podem vir do arquivo de configuração).
fn tema(cfg: &Config, c: Color, padrao: (u8, u8, u8)) -> (u8, u8, u8) {
    match c {
        Color::Default => padrao,
        outro => bzmux_core::color::resolve(outro, &cfg.palette256(), padrao),
    }
}

/// Devolve um buffer novo só quando a capacidade não serve; `None` mantém o
/// atual (evita realocar a cada frame).
fn grow_buffer(
    device: &wgpu::Device,
    cap: &mut usize,
    needed: usize,
    min: usize,
    label: &str,
) -> Option<wgpu::Buffer> {
    if needed <= *cap {
        return None;
    }
    *cap = (needed * 2).max(min);
    Some(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: *cap as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}
