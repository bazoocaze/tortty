//! Renderizador: monta instâncias e desenha em uma única passada na GPU.
//!
//! O gargalo medido do bzmux Python era a **alternância** de `fill` de fundo e
//! `show_layout` por run (13,0 ms/frame; 7,9 ms só agrupando). Aqui fundo e
//! glifo são a mesma instância de quad — não existe alternância para pagar, e
//! o frame custa o que custa montar vértices (dezenas de µs), não o desenho.

use std::collections::HashMap;
use std::sync::Arc;

use bzmux_core::{Color, Term};
use bytemuck::{Pod, Zeroable};
use swash::scale::ScaleContext;
use winit::window::Window;

use crate::atlas::{bind_atlas, globals_buffer, globals_layout, Atlas};
use crate::glyph::{self, Fonts};
use crate::prof;

/// Fundo e frente default do bzmux Python (`BG_DEFAULT`/`FG_DEFAULT`).
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
}

impl Renderer {
    pub fn new(window: Arc<Window>, fonts: Fonts, vsync: bool) -> Result<Renderer, String> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("surface: {}", e))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("adapter: {}", e))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bzmux"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("device: {}", e))?;

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
        let globals = globals_buffer(&device, w as f32, h as f32);
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
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
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
            palette: bzmux_core::color::palette256(),
        })
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
            bytemuck::cast_slice(&[w as f32, h as f32, 0.0f32, 0.0f32]),
        );
    }

    #[inline]
    fn color(&self, c: Color, default: (u8, u8, u8)) -> [f32; 3] {
        let (r, g, b) = bzmux_core::color::resolve(c, &self.palette, default);
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
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

    /// Monta as instâncias do frame. Público para o teste de sanidade do formato.
    pub fn build(&mut self, term: &Term, view_off: usize) {
        self.instances.clear();
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let ascent = self.fonts.ascent;
        let cols = term.cols;
        let wide = term.in_alt_screen();
        let _ = wide;

        let rows = term.visible_rows(view_off);
        for (y, row) in rows.iter().enumerate() {
            let py = y as f32 * chh;

            // 1) fundos, agrupados por cor (uma instância por trecho)
            let mut x = 0usize;
            while x < cols {
                let cell = match row.cells.get(x) {
                    Some(c) => *c,
                    None => break,
                };
                let (bg, fg, flags) = if cell.flags & bzmux_core::cell::INV != 0 {
                    (cell.fg, cell.bg, cell.flags)
                } else {
                    (cell.bg, cell.fg, cell.flags)
                };
                let _ = fg;
                let mut end = x + 1;
                while end < cols {
                    let c2 = match row.cells.get(end) {
                        Some(c) => *c,
                        None => break,
                    };
                    let same = if c2.flags & bzmux_core::cell::INV != 0 {
                        c2.fg == bg && c2.bg == cell.fg
                    } else {
                        c2.bg == bg
                    };
                    if !same {
                        break;
                    }
                    end += 1;
                }
                if bg != Color::Default {
                    let c = self.color(bg, BG);
                    self.push_rect(
                        x as f32 * cw,
                        py,
                        (end - x) as f32 * cw,
                        chh,
                        [c[0], c[1], c[2], 1.0],
                    );
                }
                let _ = flags;
                x = end;
            }

            // 2) glifos (segunda passada: nada de alternar fundo e texto)
            for x in 0..cols.min(row.cells.len()) {
                let cell = row.cells[x];
                if cell.is_cont() {
                    continue;
                }
                let (bg, fg) = if cell.flags & bzmux_core::cell::INV != 0 {
                    (cell.fg, cell.bg)
                } else {
                    (cell.bg, cell.fg)
                };
                let _ = bg;
                let bold = cell.flags & bzmux_core::cell::BOLD != 0;
                if cell.ch != ' ' {
                    if let Some(q) = self.glyph_quad(cell.ch, bold, false) {
                        let c = self.color(fg, FG);
                        let gx = x as f32 * cw + q.left as f32;
                        let gy = py + ascent - q.top as f32;
                        self.instances.push(Instance {
                            rect: [gx, gy, q.w as f32, q.h as f32],
                            color: [c[0], c[1], c[2], 1.0],
                            uv: q.uv,
                        });
                    }
                }
                if cell.flags & bzmux_core::cell::UL != 0 {
                    let c = self.color(fg, FG);
                    self.push_rect(
                        x as f32 * cw,
                        py + ascent + 1.0,
                        cw,
                        1.0,
                        [c[0], c[1], c[2], 1.0],
                    );
                }
            }
        }

        // 3) cursor (por cima; a ordem das instâncias é a ordem de blend)
        if view_off == 0 && term.cursor_visible && term.cy < term.rows && term.cx < cols {
            let cx = term.cx as f32 * cw;
            let cy = term.cy as f32 * chh;
            let style = term.app_cursor_style.map(|(f, _)| f).unwrap_or(0);
            let c = self.color(Color::Default, FG);
            match style {
                1 | 2 => {
                    // sublinhado / barra
                    let h = if style == 1 { 2.0 } else { chh };
                    let w = if style == 1 { cw } else { 2.0 };
                    let y = if style == 1 { cy + chh - 3.0 } else { cy };
                    self.push_rect(cx, y, w, h, [c[0], c[1], c[2], 0.9]);
                }
                _ => {
                    self.push_rect(cx, cy, cw, chh, [c[0], c[1], c[2], 0.85]);
                }
            }
        }
    }

    pub fn render(&mut self, term: &Term, view_off: usize) -> FrameOutcome {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return FrameOutcome::Skipped;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                return FrameOutcome::Reconfigure;
            }
            wgpu::CurrentSurfaceTexture::Validation => return FrameOutcome::Fatal,
        };

        let t0 = std::time::Instant::now();
        self.build(term, view_off);
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
        let (r, g, b) = BG;
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
                            r: r as f64 / 255.0,
                            g: g as f64 / 255.0,
                            b: b as f64 / 255.0,
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
pub enum FrameOutcome {
    Drawn,
    Skipped,
    Reconfigure,
    Fatal,
}
