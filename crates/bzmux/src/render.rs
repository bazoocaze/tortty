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

#[derive(Copy, Clone)]
struct GlyphQuad {
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    uv: [f32; 4],
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

        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("adapter: {e}"))?;
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
            alpha_mode: caps.alpha_modes[0],
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
            inst_buf,
            inst_cap,
            atlas,
            fonts,
            scale_ctx: ScaleContext::new(),
            cache: HashMap::new(),
            instances: Vec::with_capacity(4096),
            palette: cfg.palette256(),
            fg: tema(cfg, cfg.fg, FG),
            bg: tema(cfg, cfg.bg, BG),
            cursor: tema(cfg, cfg.cursor, FG),
            pad: cfg.padding as f32,
            bar: 0.0,
            side: 0.0,
            cursor_shape: cfg.cursor_shape,
        })
    }

    /// Tema novo sem recriar a GPU (auto-reload e `OSC 4/10/11/12`).
    pub fn apply_theme(&mut self, t: &Theme) {
        self.palette = t.palette256();
        self.fg = t.rgb(t.fg, FG);
        self.bg = t.rgb(t.bg, BG);
        self.cursor = t.rgb(t.cursor, FG);
        self.write_globals();
    }

    /// O que não é cor: forma do cursor e margem (só muda por arquivo).
    pub fn apply_layout(&mut self, cfg: &Config) {
        self.cursor_shape = cfg.cursor_shape;
        self.pad = cfg.padding as f32;
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

    fn glyph_quad(&mut self, ch: char, bold: bool, italic: bool) -> Option<GlyphQuad> {
        let (face, gid) = self.fonts.pick(ch, bold, italic);
        let key = (face as u8, ch);
        if let Some(v) = self.cache.get(&key) {
            return *v;
        }
        let quad = if gid == 0 {
            None
        } else {
            let bitmap = glyph::rasterize(
                &mut self.scale_ctx,
                self.fonts.face(face),
                self.fonts.px,
                gid,
            );
            bitmap.and_then(|b| {
                self.atlas.add(b.w, b.h, &b.data).map(|uv| GlyphQuad {
                    left: b.left,
                    top: b.top,
                    w: b.w,
                    h: b.h,
                    uv,
                })
            })
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
        // a barra é o cabeçalho da janela: vai de ponta a ponta, inclusive em
        // cima da sidebar (que começa abaixo dela, junto com a grade)
        let x0 = -self.side;
        let largo = cols as f32 * cw + x0.abs();
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
    /// workspace. Fica na faixa que o `origin` reservou (x = -side), como a
    /// barra de abas fica em y = -bar.
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
        for (i, c) in cards.iter().enumerate() {
            let y0 = i as f32 * card_h;
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
    fn draw_cell(&mut self, cell: &Cell, x: usize, py: f32, swap: bool) {
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
            if let Some(q) = self.glyph_quad(cell.ch, bold, italic) {
                let gx = x as f32 * cw + q.left as f32;
                let gy = py + ascent - q.top as f32;
                self.instances.push(Instance {
                    rect: [gx, gy, q.w as f32, q.h as f32],
                    color: [c[0], c[1], c[2], alpha],
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
    pub fn build(&mut self, f: &Frame<'_>) {
        let Frame {
            term,
            view_off,
            sel,
            blink,
            toast,
            tabs,
            sidebar,
        } = *f;
        self.instances.clear();
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let cols = term.cols;
        let wide = term.in_alt_screen();
        let _ = wide;
        // retângulo da seleção na visão (preso à tela); fora dela nada muda
        let sel = sel.and_then(|s| s.visible(term, view_off));

        let rows = term.visible_rows(view_off);
        for (y, row) in rows.iter().enumerate() {
            let py = y as f32 * chh;

            // 1) fundos, agrupados por cor (uma instância por trecho). O
            // inverso (SGR 7) e a seleção usam a mesma regra: troca fg/bg e
            // resolve o default pelo *outro* extremo (senão seleção em célula
            // sem cor não apareceria).
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

            // 2) glifos e decoração (segunda passada: nada de alternar fundo
            // e texto). O sublinhado é por célula porque o estilo de 4:x é
            // desenhado no pedaço, não na largura do run.
            for x in 0..cols.min(row.cells.len()) {
                let cell = row.cells[x];
                if cell.is_cont() {
                    continue;
                }
                // SGR 5: na fase apagada a célula não é desenhada — o fundo da
                // linha já foi pintado, então "apagar" é só não escrever
                if cell.flags & bzmux_core::cell::BLINK != 0 && !blink.fase {
                    continue;
                }
                self.draw_cell(&cell, x, py, is_swap(&cell, sel, y, x));
            }
        }

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
            let c = self.color(Color::Default, self.cursor);
            match style {
                1 | 2 => {
                    // sublinhado / barra
                    let h = if style == 1 { 2.0 } else { chh };
                    let w = if style == 1 { cw } else { 2.0 };
                    let y = if style == 1 { cy + chh - 3.0 } else { cy };
                    self.push_rect(cx, y, w, h, [c[0], c[1], c[2], 0.9]);
                },
                _ => {
                    self.push_rect(cx, cy, cw, chh, [c[0], c[1], c[2], 0.85]);
                },
            }
        }

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

    pub fn render(&mut self, f: &Frame<'_>) -> FrameOutcome {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return FrameOutcome::Skipped;
            },
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                return FrameOutcome::Reconfigure;
            },
            wgpu::CurrentSurfaceTexture::Validation => return FrameOutcome::Fatal,
        };

        let t0 = std::time::Instant::now();
        self.build(f);
        self.atlas.flush(&self.queue);

        let needed = self.instances.len() * std::mem::size_of::<Instance>();
        if needed > self.inst_cap {
            self.inst_cap = (needed * 2).max(1 << 20);
            self.inst_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: self.inst_cap as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if needed > 0 {
            self.queue
                .write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&self.instances));
        }

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
            if !self.instances.is_empty() {
                rp.draw(0..6, 0..self.instances.len() as u32);
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

/// O que fazer depois de um frame (o `get_current_texture` do wgpu 30 devolve
/// um enum em vez de `Result`).
/// Estado do pisca neste frame (G12).
///
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
