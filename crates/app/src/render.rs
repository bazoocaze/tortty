//! Renderer: builds instances and draws in a single GPU pass.
//!
//! The **alternation** of background and glyph per run is what used to cost
//! (13.0 ms/frame; 7.9 ms just from grouping runs). Here background and glyph are
//! the same quad instance — there is
//! no alternation to pay for, and a frame costs what building vertices costs
//! (tens of µs), not the drawing.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use swash::scale::ScaleContext;
use tortty_core::cell::{Cell, INV};
use tortty_core::config::{Config, CursorShape, Theme};
use tortty_core::select::Rect;
use tortty_core::term::{sanitize_title, Progress};
use tortty_core::{Color, Span, Term};
use winit::window::Window;

use crate::atlas::{bind_atlas, globals_buffer, globals_layout, Atlas};
use crate::glyph::{self, Fonts};
use crate::prof;

/// Background and foreground used when the configuration says nothing.
pub const BG: (u8, u8, u8) = (0, 0, 0);
pub const FG: (u8, u8, u8) = (214, 214, 214);
/// Width of the scrollbar strip in px (the app hit-tests the same number).
pub const SCROLLBAR_W: f32 = 8.0;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Instance {
    rect: [f32; 4],
    color: [f32; 4],
    /// u0, v0, u1, v1 — all four equal = solid quad (white pixel).
    uv: [f32; 4],
}

/// Is the cell inverted on this row/column? `SGR 7` always inverts; the
/// selection inverts whatever is inside it.
#[inline]
fn is_swap(cell: &Cell, sel: Option<Rect>, y: usize, x: usize) -> bool {
    let inv = cell.flags & INV != 0;
    let inside = sel.is_some_and(|s| {
        // a rectangle cuts the same columns on every row; a flow selection only
        // cuts the first and the last one (`B5`)
        let in_cols = if s.block {
            x >= s.c1 && x <= s.c2
        } else {
            x >= (if y == s.r1 { s.c1 } else { 0 })
                && x <= (if y == s.r2 { s.c2 } else { usize::MAX })
        };
        y >= s.r1 && y <= s.r2 && in_cols
    });
    inv ^ inside
}

/// Does this glyph exist to touch the one in the neighbour cell?
///
/// Box drawing (`─ ┼ │`, what `mc`, `vim` and DEC graphics draw) and block
/// elements (`▀ ▄ █`, the quote bar of `git` and of `vim`) are meant to tile; the
/// rest of the text has no such intention and stays where the advance puts it.
fn tiles(ch: char) -> bool {
    matches!(ch, '\u{2500}'..='\u{257f}' | '\u{2580}'..='\u{259f}')
}

/// Instances of one row: the mask (text, background, decoration) and the colour
/// emoji, which goes in its own draw call because it has an RGBA atlas.
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
    /// Came from the RGBA atlas (colour emoji) instead of the mask.
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
    /// Cache of the same glyph resolved by (face, gid) — this is what shaping uses.
    cache_gid: HashMap<(u8, u16), Option<GlyphQuad>>,
    /// Shaping per row run (ligatures and whatever else the font does).
    shaper: crate::shape::Shaper,
    /// Ligature mode coming from the configuration.
    ligatures: tortty_core::config::Ligatures,
    /// `bold_as_bright`: bold text on the low half of the palette draws the
    /// bright half (`color::bright`).
    bold_as_bright: bool,
    instances: Vec<Instance>,
    pub palette: Vec<(u8, u8, u8)>,
    /// Effective theme (config + defaults); `build` resolves cell by cell.
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub cursor: (u8, u8, u8),
    /// Margin in px around the grid (`window_padding_width`).
    pub pad: f32,
    /// Height of the tab bar in px (0 = a single tab, no bar). It pushes the grid
    /// down: the shader's `origin` is `pad + bar`.
    bar: f32,
    /// Width of the workspace sidebar in px (0 = a single workspace). Pushes the
    /// grid right: `origin` becomes `pad + side`.
    side: f32,
    pub cursor_shape: CursorShape,
    /// Where the toast was drawn in the last frame, in **window** pixels
    /// (`(x, y, w, h)`), or `None` when there was none. The app reads it to
    /// answer a click: the box floats over the grid and swallows the press, so
    /// the geometry cannot live only here (the duplication that `M-Q` exists to
    /// kill).
    pub toast_rect: Option<[f32; 4]>,
    /// Where text ends and images come in (images sit above text and below the
    /// cursor/chrome, as in kitty).
    img_split: usize,
    /// Where the **grid's** colour emoji end and the chrome's begin (`B8`): the
    /// chrome is drawn after the grid, so its colour glyphs need their own span,
    /// drawn last.
    emoji_split: usize,
    img_pipeline: wgpu::RenderPipeline,
    img_buf: wgpu::Buffer,
    img_instances: Vec<Instance>,
    /// Image id of each instance (the texture is bound by id).
    img_ids: Vec<u32>,
    img_layout: wgpu::BindGroupLayout,
    img_sampler: wgpu::Sampler,
    /// RGBA atlas of the colour glyphs (emoji). Same shelf as the mask, but
    /// coloured: emoji does not accept tinting.
    emoji: Atlas,
    emoji_bind: wgpu::BindGroup,
    emoji_instances: Vec<Instance>,
    emoji_buf: wgpu::Buffer,
    emoji_cap: usize,
    /// Emoji quads of the row being built (relative y, like the mask).
    emoji_row: Vec<Instance>,
    /// Texture per image (id → bind group). The bind group keeps the texture alive.
    img_cache: HashMap<u32, ImgTex>,
    /// Grid instances, kept **per row**: only the dirty row is rebuilt
    /// (A17/G17). The final list (`instances`) is their concatenation plus the
    /// images, the chrome and the cursor, in blend order.
    per_line: Vec<RowQuads>,
    /// Signature of the last frame: what changed on the whole screen (see
    /// `damage`).
    signature: crate::damage::Signature,
    /// Theme generation: bumps on every `apply_theme` and forces a full rebuild.
    theme_gen: u64,
    /// `state` counters (prof): proof of the damage without looking at the screen.
    pub draw: u64,
    /// Result of the last frame ("drawn"/"skipped"/"occluded"...): the first
    /// question when the window does not update is whether the frame went out.
    pub last: &'static str,
    /// How many frames the surface refused (`Timeout`/`Occluded`) and how many
    /// went out: separates "the app did not draw" from "the app drew and the
    /// surface did not present" — the symptom of a confined environment (M-AA).
    pub skips: u64,
    pub drawn: u64,
    /// Already warned on stderr that the surface is not presenting.
    avisou_skip: bool,
    /// Damage decision of the last frame (`all`/`rows: N`): answers on the spot
    /// why `rows_built` grows too much.
    pub last_damage: String,
    /// How many frames were "everything" and the largest span ever rebuilt:
    /// separates "the damage did not decide" from "it decided and the core
    /// dirtied everything".
    pub frames_all: u64,
    pub rows_max: usize,
    /// Why the last frame was "everything": `global` (core damage) or
    /// `signature` (something full-screen changed). Without it the counter only
    /// says it happened, not who asked for it.
    pub all_reason: &'static str,
    pub linhas_remontadas: u64,
    pub celulas_remontadas: u64,
}

struct ImgTex {
    version: u64,
    bind: wgpu::BindGroup,
}

/// Picks the adapter with an explicit rule instead of letting `wgpu` take the
/// first one that shows up.
///
/// Rule: hardware adapter (discrete/integrated/virtual) beats CPU adapter
/// (llvmpipe/lavapipe: VM, remote X, container without `/dev/dri`); on a tie the
/// `wgpu` order wins. The list exists so CPU does not get chosen by accident —
/// and to record what was learned about each backend in DEBUG.md.
///
/// `WGPU_BACKEND` still rules: whoever asked for an explicit backend gets that
/// backend, or the usual error.
fn choose_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, String> {
    let request = std::env::var("WGPU_BACKEND").unwrap_or_default();
    let all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let candidates: Vec<wgpu::Adapter> = all
        .into_iter()
        .filter(|a| a.is_surface_supported(surface))
        .collect();
    let de_cpu = |a: &wgpu::Adapter| a.get_info().device_type == wgpu::DeviceType::Cpu;
    let chosen = if request.is_empty() {
        // hardware first; with no hardware, the natural `wgpu` order (Vulkan
        // before GL). GL is not preferred for CPU: in a VM without `/dev/dri`
        // GL/llvmpipe depends on MIT-SHM to reach the window and, when SHM does
        // not work, the window stays empty, while Vulkan/llvmpipe draws through
        // Present.
        candidates
            .iter()
            .find(|a| !de_cpu(a))
            .or_else(|| candidates.first())
    } else {
        candidates.first()
    };
    let chosen = chosen.ok_or_else(|| {
        if candidates.is_empty() {
            "adapter: no adapter supports the surface".to_string()
        } else {
            "adapter: no usable adapter".to_string()
        }
    })?;
    Ok(chosen.clone())
}

impl Renderer {
    #[allow(clippy::too_many_lines)] // GPU setup is one single sequence; splitting does not help
    pub fn new(
        window: &Arc<Window>,
        fonts: Fonts,
        vsync: bool,
        cfg: &Config,
    ) -> Result<Renderer, String> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        // the display handle is not optional for every backend: EGL (GL backend)
        // needs it to match the surface with the display, and without this
        // `WGPU_BACKEND=gl` dies with "gl not compatible with provided
        // surface". Vulkan does not complain, so the defect only shows up on the
        // machine that has no Vulkan — exactly the one that needed GL.
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("surface: {e}"))?;
        let adapter = choose_adapter(&instance, &surface)?;
        // which backend/board was chosen: it is the first question when the
        // window does not draw on another machine (see DEBUG.md)
        let info = adapter.get_info();
        eprintln!(
            "tortty: GPU: {:?} {} (driver {})",
            info.backend, info.name, info.driver
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tortty"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        // non-srgb format: the cell colours are already sRGB and must not be
        // linearized again by the target
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
            // explicit `Opaque`: taking the first of the list gave a translucent
            // window on the GL backend (ARGB visual with the background leaking
            // what is behind it). A terminal is opaque; transparency is not
            // parity.
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
            label: Some("tortty"),
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
            label: Some("tortty-layout"),
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
            label: Some("tortty-pipeline"),
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

        // image pipeline: same globals/atlas layout and the same instance
        // format, only the shader changes (samples RGB instead of the coverage
        // mask)
        let img_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tortty-image"),
            source: wgpu::ShaderSource::Wgsl(include_str!("image.wgsl").into()),
        });
        let (img_layout, img_sampler) = crate::atlas::bind_image(&device);
        // emoji uses the image pipeline (filterable texture), so its bind group
        // has to follow the same layout, not the mask atlas one
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
            label: Some("tortty-image-layout"),
            bind_group_layouts: &[Some(&gl), Some(&img_layout)],
            immediate_size: 0,
        });
        let img_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tortty-image-pipeline"),
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
            ligatures: tortty_core::config::Ligatures::default(),
            bold_as_bright: cfg.bold_as_bright,
            instances: Vec::with_capacity(4096),
            palette: cfg.palette256(),
            fg: theme(cfg, cfg.fg, FG),
            bg: theme(cfg, cfg.bg, BG),
            cursor: theme(cfg, cfg.cursor, FG),
            pad: cfg.padding as f32,
            bar: 0.0,
            side: 0.0,
            cursor_shape: cfg.cursor_shape,
            toast_rect: None,
            img_split: 0,
            emoji_split: 0,
            img_pipeline,
            img_buf,
            img_instances: Vec::new(),
            img_ids: Vec::new(),
            img_layout,
            img_sampler,
            img_cache: HashMap::new(),
            per_line: Vec::new(),
            signature: crate::damage::Signature::default(),
            theme_gen: 0,
            draw: 0,
            last: "none",
            skips: 0,
            drawn: 0,
            avisou_skip: false,
            last_damage: String::new(),
            frames_all: 0,
            rows_max: 0,
            all_reason: "",
            linhas_remontadas: 0,
            celulas_remontadas: 0,
        })
    }

    /// New theme without recreating the GPU (auto-reload and `OSC 4/10/11/12`).
    pub fn apply_theme(&mut self, t: &Theme) {
        // anything that changes cell colour invalidates the damage: the clean
        // rows would keep the old palette
        self.theme_gen = self.theme_gen.wrapping_add(1);
        self.palette = t.palette256();
        self.fg = t.rgb(t.fg, FG);
        self.bg = t.rgb(t.bg, BG);
        self.cursor = t.rgb(t.cursor, FG);
        self.write_globals();
    }

    /// What is not colour: cursor shape, margin and ligatures (only changes per
    /// file).
    pub fn apply_layout(&mut self, cfg: &Config) {
        self.cursor_shape = cfg.cursor_shape;
        self.pad = cfg.padding as f32;
        if self.ligatures != cfg.ligatures {
            // a shaped run is per mode: changing the mode invalidates the cache
            self.ligatures = cfg.ligatures;
            self.shaper = crate::shape::Shaper::new();
            // the damage compares `theme_gen`: touching it forces the grid rebuild
            self.theme_gen = self.theme_gen.wrapping_add(1);
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

    /// Reserves the tab bar strip (0 = no bar). Call whenever the number of tabs
    /// changes; the grid already arrives with the height discounted.
    pub fn set_bar(&mut self, h: f32) {
        if (h - self.bar).abs() < 0.01 {
            return;
        }
        self.bar = h;
        self.write_globals();
    }

    /// Reserves the workspace sidebar column (0 = a single workspace).
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
        let (r, g, b) = tortty_core::color::resolve(c, &self.palette, default);
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ]
    }

    /// Quad from the bitmap, in the right atlas (RGBA for emoji, mask for the
    /// rest). It is the only place that writes to both atlases.
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

    /// Quad of a glyph **already resolved** by shaping. Cache by (face, gid): the
    /// same glyph can come from different characters and different runs.
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

    /// Quad resolving the character through the font chain (path without
    /// shaping: single-cell run, or a glyph that shaping did not draw).
    fn glyph_quad(&mut self, ch: char, bold: bool, italic: bool) -> Option<GlyphQuad> {
        // the key is the requested style, not the chosen face: the font chain
        // decides the face, and the result already comes resolved
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

    /// Badge for a row in the top-right corner (history indicator).
    /// Drawn last: it has to sit on top of whatever is there.
    /// Chrome text: **the only place** that draws text outside the grid
    /// (tab bar, sidebar card, history badge, toast).
    ///
    /// Sanitization lives here, not in the caller: that way a new chrome element
    /// inherits the guarantee of not drawing control, zero-width or bidi marks
    /// in the label — that is how the tab title used to show garbage before the
    /// text (B4, 2026-11-04). Advance is always one cell per character;
    /// `width` = 0 means no cap.
    fn chrome_text(&mut self, text: &str, x0: f32, y: f32, fg: [f32; 3], width: f32) {
        let cw = self.fonts.cw;
        let ascent = self.fonts.ascent;
        let limit = if width > 0.0 {
            width.floor()
        } else {
            f32::INFINITY
        };
        for (i, ch) in sanitize_title(text).chars().enumerate() {
            let px = x0 + i as f32 * cw;
            if px + cw > x0 + limit {
                break;
            }
            if ch == ' ' {
                continue;
            }
            if let Some(q) = self.glyph_quad(ch, false, false) {
                self.instances.push(Instance {
                    rect: [
                        px + q.left as f32,
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

    /// The bell of a tab with a notification (`B8`), in a two-cell box.
    ///
    /// It comes from the emoji atlas when the font has a colour one, and from the
    /// mask atlas (tinted with `fg`) when it does not — the two targets are what
    /// the frame splits, and nothing here needs to know which font answered.
    fn chrome_bell(&mut self, x: f32, y: f32, fg: [f32; 3]) {
        let cw = self.fonts.cw;
        let ascent = self.fonts.ascent;
        let Some(q) = self.glyph_quad('\u{1F514}', false, false) else {
            return;
        };
        // centred in the two cells the label reserved
        let slack = (2.0 * cw - q.w as f32) / 2.0;
        let rect = [
            x + slack.max(0.0) + q.left as f32,
            y + ascent - q.top as f32,
            q.w as f32,
            q.h as f32,
        ];
        if q.color {
            self.emoji_instances.push(Instance {
                rect,
                color: [1.0, 1.0, 1.0, 1.0],
                uv: q.uv,
            });
        } else {
            self.instances.push(Instance {
                rect,
                color: [fg[0], fg[1], fg[2], 1.0],
                uv: q.uv,
            });
        }
    }

    /// Tab bar: ` 1 zsh │ 2 vim  ` in the strip above the grid.
    ///
    /// The active tab is inverted (background = text colour). A tab with a recent
    /// BELL swaps the highlight colour — that is what makes "that terminal
    /// beeped" visible without looking at it.
    fn tabbar(&mut self, tabs: &[TabInfo], cols: usize) {
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        // the bar starts to the right of the sidebar: the two share row 1, and
        // the sidebar does not get a hole on top (the card is 2.4 rows high, so
        // it passes under the bar anyway)
        let x0 = 0.0;
        let total_w = cols as f32 * cw;
        let bg = self.color(Color::Idx(0), self.bg);
        // text over the active tab uses the grid background colour (inversion)
        let over = self.color(Color::Default, self.bg);
        // `origin.y` is `pad + bar`: so the bar lands in the reserved strip
        // (and not over the first grid row) it is drawn at y = -bar
        let y0 = -self.bar;
        self.push_rect(x0, y0, total_w, chh, [bg[0], bg[1], bg[2], 1.0]);
        let mut x = x0;
        for (i, t) in tabs.iter().enumerate() {
            let label = format!(" {} {} ", i + 1, t.title);
            // a tab with a notification keeps its colours and carries the bell
            // (`B8`): painting the whole label was louder than the news
            let bell = if t.alert { 2 } else { 0 };
            let n = label.chars().count() + bell;
            let width = (n as f32 * cw).min(total_w - x);
            if width <= 0.0 {
                break;
            }
            let (bg, fg) = if t.ativa {
                (self.color(Color::Default, self.fg), over)
            } else {
                (bg, self.color(Color::Idx(8), self.fg))
            };
            // one rectangle per tab (not one per letter): the label background is
            // a single piece, and the text is drawn on top of it
            self.push_rect(x, y0, width, chh, [bg[0], bg[1], bg[2], 1.0]);
            let text_x = x + bell as f32 * cw;
            if t.alert {
                self.chrome_bell(x, y0, fg);
            }
            self.chrome_text(&label, text_x, y0, fg, width - bell as f32 * cw);
            if let Some(p) = t.progress {
                let (c, alpha, frac) = progress_ink(p);
                let rgb = self.color(c, self.fg);
                self.push_rect(
                    x,
                    y0 + chh - PROGRESS_H,
                    width * frac,
                    PROGRESS_H,
                    [rgb[0], rgb[1], rgb[2], alpha],
                );
            }
            x += width;
        }
    }

    /// Workspace sidebar (I6): a column on the left with one card per workspace.
    /// It occupies the strip the `origin` reserved (x = -side) and starts on row
    /// 1, together with the tab bar (y = -bar).
    ///
    /// It is drawn from card `off` (`B42`): the list scrolls, so the first card
    /// can start above the column and the last one is cut at the bottom of the
    /// grid — the strip ends there, not at the window's edge.
    fn sidebar(&mut self, cards: &[WsCard], off: usize, visible: usize) {
        if self.side <= 0.0 {
            return;
        }
        let chh = self.fonts.ch;
        let card_h = chh * 2.4;
        let x0 = -self.side;
        let width = self.side - self.pad;
        let bg = self.color(Color::Idx(0), self.bg);
        let over = self.color(Color::Default, self.bg);
        let top = -self.bar;
        // the bottom of the grid, in this coordinate space (y = 0 is the first
        // grid row): the cards are cut here
        let bottom = self.config.height as f32 - self.pad - self.bar;
        for (i, c) in cards.iter().enumerate().skip(off).take(visible.max(1)) {
            let y0 = top + (i - off) as f32 * card_h;
            if y0 >= bottom {
                break;
            }
            let y1 = (y0 + card_h).min(bottom);
            let active = c.ativa;
            let (bg, fg) = if active {
                (self.color(Color::Default, self.fg), over)
            } else {
                (bg, self.color(Color::Idx(8), self.fg))
            };
            self.push_rect(x0, y0, width, y1 - y0, [bg[0], bg[1], bg[2], 1.0]);
            // a text row is drawn only when the whole cell fits: half a glyph
            // at the cut is worse than no glyph
            let fits = |y: f32| y + chh <= bottom;
            // the card is tinted by the alert colour while a notification
            // waits (`M-K`), like a tab label does
            let fg = if c.alert {
                self.color(Color::Idx(3), self.fg)
            } else {
                fg
            };
            let (first, second) = card_lines(i + 1, &c.name, &c.title);
            if fits(y0 + 3.0) {
                self.text_line(x0, y0 + 3.0, &first, fg, width);
            }
            // the second row only exists when the first took the name (`B43`):
            // an empty row is not drawn, the card keeps its 2.4-row height
            if !second.is_empty() && fits(y0 + chh + 3.0) {
                self.text_line(x0, y0 + chh + 3.0, &second, fg, width);
            }
            if let Some(p) = c.progress {
                let (color, alpha, frac) = progress_ink(p);
                let rgb = if c.alert {
                    over
                } else {
                    self.color(color, self.fg)
                };
                let h = PROGRESS_H.min(y1 - y0);
                self.push_rect(x0, y1 - h, width * frac, h, [rgb[0], rgb[1], rgb[2], alpha]);
            }
        }
    }

    /// One sidebar text row, clipped to the card width.
    fn text_line(&mut self, x0: f32, y: f32, text: &str, fg: [f32; 3], width: f32) {
        let fit = (width / self.fonts.cw).floor().max(1.0) as usize;
        // ellipsis instead of a hard cut: a narrow card does not eat the end of
        // the title without warning
        let text: String = if text.chars().count() > fit {
            let mut t: String = text.chars().take(fit.saturating_sub(1)).collect();
            t.push('…');
            t
        } else {
            text.to_string()
        };
        self.chrome_text(&text, x0 + self.pad, y, fg, 0.0);
    }

    /// Toast (I12/K13): a box floating over the bottom of the grid, in the
    /// middle, with a title, an optional detail that wraps and an accent bar
    /// telling the three shades apart (receipt, warning, error).
    ///
    /// It sits at the bottom because the top holds the tab bar (the header is
    /// its own); the notice belongs to the terminal, not to the program. It
    /// used to be a strip across the last row with the colours inverted — that
    /// erased the line and did not read as an aviso. The box erases only what
    /// it occupies.
    ///
    /// The surface is the theme background pulled 14% towards the foreground,
    /// so it works in a light theme too (it darkens instead of lightening) and
    /// needs no key. Two rects, not one: the opaque background goes first, or
    /// the text underneath would show through the veil.
    ///
    /// No rounded corner and no shadow for now: the pipeline paints everything
    /// through one texture, so a corner would need a mask of its own baked into
    /// the atlas (nine patch) — an idea kept for later, not a limit.
    fn toast(&mut self, t: &ToastText, cols: usize, rows: usize) {
        // gutter on both sides of the text, in cells, so the box lines up with
        // the grid; the vertical respiro is in px, independent of the line
        // height of the theme
        const PAD_X: f32 = 1.5;
        const PAD_Y: f32 = 8.0;
        const MARGIN_X: f32 = 1.0;
        const MARGIN_Y: f32 = 28.0;
        // the accent bar on the left: wide enough to read as a strip and not as
        // a crack in the border
        const BAR: f32 = 4.0;
        const VEIL: f32 = 0.14;
        // Detail lines beyond this are cut: a notification carrying a whole page
        // must not cover the grid it is telling you about.
        const DETAIL_LINES: usize = 4;
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        // title and detail wrap at the same column, so the box has straight
        // sides (the old one cut every notice at the grid width)
        let n = (cols as f32 - 2.0 * MARGIN_X - 2.0 * PAD_X)
            .floor()
            .max(1.0) as usize;
        let title: String = t.title.chars().take(n).collect();
        let room = (rows.saturating_sub(2)).max(1);
        let detail = wrap(&t.detail, n, DETAIL_LINES.min(room));
        let widest = detail
            .iter()
            .map(|l| l.chars().count())
            .chain(std::iter::once(title.chars().count()))
            .max()
            .unwrap_or(0);
        if widest == 0 {
            return;
        }
        let lines = 1 + detail.len();
        let bw = widest as f32 * cw + 2.0 * PAD_X * cw + BAR;
        let bh = lines as f32 * chh + 2.0 * PAD_Y;
        // centred: a notice is read where the eye lands, not in a corner. The
        // text was cut at `n` above, so the box always fits the grid.
        let x1 = ((cols as f32 * cw - bw) / 2.0).max(0.0);
        let y1 = (rows as f32 * chh - MARGIN_Y - bh).max(0.0);
        let base = self.color(Color::Default, self.bg);
        let ink = self.color(Color::Default, self.fg);
        let accent = self.color(t.severity.color(), self.fg);
        // the detail is the same ink pulled 22% towards the surface: it recedes
        // in a dark theme and in a light one alike
        let dim = [
            ink[0] * 0.78 + base[0] * 0.22,
            ink[1] * 0.78 + base[1] * 0.22,
            ink[2] * 0.78 + base[2] * 0.22,
        ];
        let edge = [accent[0], accent[1], accent[2], 1.0];
        self.push_rect(x1, y1, bw, bh, [base[0], base[1], base[2], 1.0]);
        self.push_rect(x1, y1, bw, bh, [ink[0], ink[1], ink[2], VEIL]);
        // the quad has no stroke: the border is four 1 px rects
        self.push_rect(x1, y1, bw, 1.0, edge);
        self.push_rect(x1, y1 + bh - 1.0, bw, 1.0, edge);
        self.push_rect(x1, y1, 1.0, bh, edge);
        self.push_rect(x1 + bw - 1.0, y1, 1.0, bh, edge);
        self.push_rect(x1 + 1.0, y1 + 1.0, BAR, bh - 2.0, edge);
        let tx = x1 + BAR + PAD_X * cw;
        self.chrome_text(&title, tx, y1 + PAD_Y, ink, 0.0);
        for (i, line) in detail.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)] // one line index, far below 2^23
            let y = y1 + PAD_Y + (i + 1) as f32 * chh;
            self.chrome_text(line, tx, y, dim, 0.0);
        }
        // window pixels, so the app answers a click without repeating the
        // arithmetic that built the box
        self.toast_rect = Some([x1 + self.pad + self.side, y1 + self.pad + self.bar, bw, bh]);
    }

    fn badge(&mut self, text: &str, cols: usize) {
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
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
        self.chrome_text(text, x0 as f32 * cw, 0.0, fg, 0.0);
    }

    /// Glyph + decoration of one cell. The order here is the blend order:
    /// everything of the text comes before the cursor.
    /// `glyph` = glyph already resolved by shaping (`face`, `gid`, offset in
    /// px). `None` means resolve the character through the chain — that is the
    /// single-cell run path. A shaping glyph that does not draw (a face that maps
    /// and has no bitmap) also falls back to the chain.
    fn draw_cell(
        &mut self,
        cell: &Cell,
        x: usize,
        py: f32,
        swap: bool,
        glyph: Option<(usize, u16, f32)>,
        links: bool,
    ) {
        use tortty_core::cell::{
            BOLD, FAINT, ITALIC, OVERLINE, STRIKE, UL_CURLY, UL_DASHED, UL_DOTTED, UL_DOUBLE,
        };
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let ascent = self.fonts.ascent;
        // `bold_as_bright` (opt-in): the glyph of a bold cell in the low half of
        // the palette takes the bright half — the swapped (reverse video) cell
        // keeps the palette as it is, because there the glyph colour is the
        // background
        let mut fg = cell.fg;
        if self.bold_as_bright && cell.flags & BOLD != 0 {
            fg = tortty_core::color::bright(fg);
        }
        let fg = if swap { cell.bg } else { fg };
        let mut c = self.color(fg, if swap { self.bg } else { self.fg });
        // faint: the same tone with less opacity (Pango used alpha 0.55)
        let alpha = if cell.flags & FAINT != 0 { 0.55 } else { 1.0 };

        if cell.ch != ' ' {
            let bold = cell.flags & BOLD != 0;
            let italic = cell.flags & ITALIC != 0;
            let quad = glyph
                .and_then(|(face, gid, _)| self.quad_gid(face, gid))
                .or_else(|| self.glyph_quad(cell.ch, bold, italic));
            let desloc = glyph.map_or(0.0, |(_, _, dx)| dx);
            if let Some(q) = quad {
                let mut gx = x as f32 * cw + desloc + q.left as f32;
                let mut gw = q.w as f32;
                // Box drawing and block elements are the glyphs that touch the
                // neighbour's ink on purpose: at a fractional cell width the
                // antialias of two neighbours meets with less than full ink, and
                // a dim column shows at the seam. The quad goes half a pixel
                // wider on each side, so the two coverages add up instead. A
                // stroke (`│`) does not span the cell and stays where the advance
                // puts it.
                if tiles(cell.ch) && gw >= cw - 1.0 {
                    gx = x as f32 * cw + desloc + q.left as f32 - 0.5;
                    gw += 1.0;
                }
                let gy = py + ascent - q.top as f32;
                // colour emoji gets no tinting: only the cell alpha
                let (cor, target) = if q.color {
                    ([1.0, 1.0, 1.0, alpha], &mut self.emoji_row)
                } else {
                    ([c[0], c[1], c[2], alpha], &mut self.instances)
                };
                target.push(Instance {
                    rect: [gx, gy, gw, q.h as f32],
                    color: cor,
                    uv: q.uv,
                });
            }
        }

        // decoration colour: the text one, except SGR 58 (which does not invert)
        if cell.ul_color != Color::Default {
            c = self.color(cell.ul_color, self.fg);
        }
        if cell.ul != tortty_core::cell::UL_NONE {
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
                    // sinusoidal approximation in 4 pieces; there is no curved
                    // geometry in the rectangle pipeline
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

        // strike at the middle of the x height (not the middle of the cell: the
        // empty descender would leave the line too high)
        if cell.flags & STRIKE != 0 {
            let y = py + ascent - chh * 0.18;
            self.push_rect(x as f32 * cw, y, cw, 1.0, [c[0], c[1], c[2], 1.0]);
        }
        if cell.flags & OVERLINE != 0 {
            self.push_rect(x as f32 * cw, py + 1.0, cw, 1.0, [c[0], c[1], c[2], 1.0]);
        }
        // `OSC 8` link: the underline is the only thing that says "this text is a
        // link", and it is drawn while Ctrl is held — the modifier that opens it.
        // A cell with an underline of its own keeps its style.
        if links && cell.link != 0 && cell.ul == tortty_core::cell::UL_NONE {
            self.push_rect(
                x as f32 * cw,
                py + ascent + 1.0,
                cw,
                1.0,
                [c[0], c[1], c[2], 1.0],
            );
        }
    }

    /// Builds the frame instances. Public for the format sanity test.
    #[allow(clippy::too_many_lines)] // one loop per category, in blend order
    #[allow(clippy::many_single_char_names)] // x/y/cw/ch/k: cell geometry
    /// Builds the instances of the visible images and guarantees each texture.
    ///
    /// H12: placement lives in **grid** coordinates, so the row on screen is
    /// `row + view_off` — without that the image stood still while the text
    /// scrolled underneath it.
    fn build_images(&mut self, f: &Frame<'_>) {
        let term = f.term;
        let (cw, chh) = (self.fonts.cw, self.fonts.ch);
        let view = i32::try_from(f.view_off).unwrap_or(0);
        let lines = i32::try_from(term.rows).unwrap_or(0);
        let cols = term.cols;
        let mut quads: Vec<(u32, Instance)> = Vec::new();
        for pl in term.graphics.placements() {
            let y = pl.line + view;
            // off screen (above or below) does not become an instance
            if y >= lines || y + i32::from(pl.rows) <= 0 || usize::from(pl.col) >= cols {
                continue;
            }
            let Some(img) = term.graphics.images().get(&pl.image_id).cloned() else {
                continue;
            };
            if !self.ensure_texture(&img) {
                continue; // unreadable image (bad PNG) does not bring the frame down
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
                    // `x`/`y`/`w`/`h` of the placement crop the source: the uv
                    // is that rectangle as a fraction of the picture
                    uv: pl.uv((img.width, img.height)),
                },
            ));
        }
        // images the app deleted do not need a live texture
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

    /// Uploads the image texture if it is not in the cache yet (or if the app
    /// resent it with the same id and the version changed).
    fn ensure_texture(&mut self, img: &tortty_core::graphics::Image) -> bool {
        if self
            .img_cache
            .get(&img.id)
            .is_some_and(|t| t.version == img.version)
        {
            return true;
        }
        let Some((w, h, rgba)) = decode(img) else {
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
                version: img.version,
                bind,
            },
        );
        true
    }

    /// Cursor (block, underline or beam) — on top of the text and the images.
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
                    // underline / beam
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

    /// The scrollbar: an 8 px strip on the grid's right edge, drawn only when
    /// there is history to scroll. The thumb is the share of the buffer the view
    /// shows and it sits at the bottom while the view follows the output
    /// (`view_off == 0`). No track frame and a translucent thumb: the strip is
    /// **over** the last column, and a solid one would eat the text under it.
    /// Width lives in `SCROLLBAR_W` because the app hit-tests the same strip.
    fn scrollbar(&mut self, f: &Frame<'_>) {
        let hist = f.term.scrollback.len();
        let rows = f.term.rows;
        if hist == 0 || rows == 0 {
            return;
        }
        let cell_w = self.fonts.cw;
        let cell_h = self.fonts.ch;
        let strip = SCROLLBAR_W.min(cell_w);
        let x = f.term.cols as f32 * cell_w - strip;
        let track_h = rows as f32 * cell_h;
        let thumb_h = (track_h * rows as f32 / (rows + hist) as f32)
            .max(12.0)
            .min(track_h);
        let off = f.view_off.min(hist) as f32 / hist as f32;
        let y = (track_h - thumb_h) * (1.0 - off);
        let ink = self.color(Color::Default, self.fg);
        self.push_rect(x, 0.0, strip, track_h, [ink[0], ink[1], ink[2], 0.10]);
        self.push_rect(x, y, strip, thumb_h, [ink[0], ink[1], ink[2], 0.55]);
    }

    pub fn build(&mut self, f: &Frame<'_>) {
        let Frame {
            tab,
            term,
            view_off,
            sel,
            blink,
            toast,
            tabs,
            sidebar,
            cards_off,
            cards_visible,
            links,
        } = *f;
        let cols = term.cols;
        // selection rectangle in the view (stuck to the screen); outside it
        // nothing changes
        let sel = sel.and_then(|s| s.visible(term, view_off));
        let rows = term.visible_rows(view_off);
        let ctx = RowCtx {
            cols,
            sel,
            blink,
            cursor: break_no_cursor(term, view_off, rows.len()),
            links,
        };

        // A17/G17: which rows to rebuild. `blink` only enters the signature if
        // there is a cell with SGR 5 in sight — the cursor blink does not live
        // here.
        let signature = crate::damage::Signature {
            tab,
            cols,
            rows: rows.len(),
            view_off,
            sel,
            links,
            blink: blink.phase && Self::has_blink(&rows),
            theme: self.theme_gen,
            fg: self.fg,
            bg: self.bg,
        };
        let changed = self.signature != signature;
        let damage = crate::damage::decide(
            &signature,
            &mut self.signature,
            // the scroll conflict (went up and down in the same frame) cannot
            // rotate the cache: the row that left does not come back, so only the
            // whole screen solves it (B6)
            term.dirty_all || term.scroll_conflict,
            &term.dirty,
            rows.len(),
        );
        if matches!(damage, crate::damage::Damage::All) {
            self.frames_all = self.frames_all.saturating_add(1);
            self.all_reason = if term.dirty_all {
                "global"
            } else if changed {
                "signature"
            } else {
                "?"
            };
        }
        self.last_damage = match &damage {
            crate::damage::Damage::All => "all".to_string(),
            crate::damage::Damage::Rows(ls) if ls.is_empty() => "rows: 0".to_string(),
            #[allow(clippy::cast_precision_loss)]
            crate::damage::Damage::Rows(ls) if ls.len() > self.rows_max => {
                self.rows_max = ls.len();
                format!("rows: {}", ls.len())
            },
            crate::damage::Damage::Rows(ls) => format!("rows: {}", ls.len()),
        };
        if self.per_line.len().ne(&rows.len()) {
            // grid changed size: no reusing the old span
            self.signature = signature;
            self.per_line = vec![RowQuads::default(); rows.len()];
            for (y, row) in rows.iter().enumerate() {
                self.per_line[y] = self.line(row, y, ctx);
            }
        } else {
            let shift = if term.scroll_conflict {
                0
            } else {
                term.scroll_shift
            };
            self.rebuild_rows(&damage, &rows, ctx, shift);
        }

        self.concat_lines();
        // everything after this point belongs to the chrome, which is drawn on
        // top: the colour glyphs of the chrome are the second half of the list
        self.emoji_split = self.emoji_instances.len();

        // here the grid text ends: the images (H1–H13) come on top of it and
        // below the chrome/cursor, which is kitty's order
        self.img_split = self.instances.len();
        self.build_images(f);

        // 2b) tab bar: it sits in the strip the `origin` reserved above the grid,
        // so it needs no clipping math
        if let Some(tabs) = tabs {
            self.tabbar(tabs, cols);
        }
        // 2c) workspace sidebar: column on the left, in the reserved strip
        if let Some(cards) = sidebar {
            self.sidebar(cards, cards_off, cards_visible);
        }

        // 2d) scrollbar: over the grid's right edge, under the cursor/toast
        self.scrollbar(f);

        // 3) cursor (on top; instance order is blend order)
        self.build_cursor(f);

        // 4) history indicator (D8): "↑N" — without it, scrolling back is
        // indistinguishable from the app having stopped writing
        if view_off > 0 {
            let txt = format!("↑{view_off}");
            self.badge(&txt, cols);
        }

        // 5) toast (I12/K13): notice from the terminal itself, on top of
        // everything. It goes away on its own, or with a click; the app controls
        // the timing, and the rect of the last frame dies with the box — a click
        // must not land on a notice that is gone.
        self.toast_rect = None;
        if let Some(t) = toast {
            self.toast(t, cols, rows.len());
        }
    }

    /// Final list in blend order: grid (per row) → images → chrome → cursor →
    /// badge → toast. The finished rows are at y = 0; the vertical position comes
    /// in here, and that is what lets scrolling only shift rows.
    fn concat_lines(&mut self) {
        self.instances.clear();
        self.emoji_instances.clear();
        let chh = self.fonts.ch;
        for (y, line) in self.per_line.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let dy = y as f32 * chh;
            for inst in &line.mask {
                let mut copies = *inst;
                copies.rect[1] += dy;
                self.instances.push(copies);
            }
            for inst in &line.emoji {
                let mut copies = *inst;
                copies.rect[1] += dy;
                self.emoji_instances.push(copies);
            }
        }
    }

    /// `true` if any visible cell has SGR 5 (only then does the blink phase force
    /// the grid to be rebuilt).
    fn has_blink(rows: &[&tortty_core::grid::Row]) -> bool {
        rows.iter().any(|r| {
            r.cells
                .iter()
                .any(|c| c.flags & tortty_core::cell::BLINK != 0)
        })
    }

    /// Builds the instances of one row without touching the frame list.
    /// Rebuilds what the damage asked for. A full-screen scroll shifts the
    /// finished rows: only the ones coming in (top or bottom) need a new glyph.
    fn rebuild_rows(
        &mut self,
        damage: &crate::damage::Damage,
        rows: &[&tortty_core::grid::Row],
        ctx: RowCtx,
        shift: isize,
    ) {
        let crate::damage::Damage::Rows(lines) = damage else {
            for (y, row) in rows.iter().enumerate() {
                self.per_line[y] = self.line(row, y, ctx);
            }
            return;
        };
        let mut rebuild = lines.clone();
        if shift != 0 {
            let n = shift.unsigned_abs().min(self.per_line.len());
            for _ in 0..n {
                if shift > 0 {
                    self.per_line.remove(0);
                    self.per_line.push(RowQuads::default());
                } else {
                    self.per_line.pop();
                    self.per_line.insert(0, RowQuads::default());
                }
            }
            // after rotating by `n` rows, the `n` that came in need a new glyph.
            // Before, it was `rebuild.push(len - 1)` **inside** the loop: the
            // index was always the same and only the last row was rebuilt — the
            // others stayed empty (blank spans in the middle of the screen)
            let end = self.per_line.len();
            if shift > 0 {
                rebuild.extend(end - n..end);
            } else {
                rebuild.extend(0..n);
            }
        }
        rebuild.sort_unstable();
        rebuild.dedup();
        for y in rebuild {
            if let Some(row) = rows.get(y) {
                self.per_line[y] = self.line(row, y, ctx);
            }
        }
    }

    /// Builds one row **relative to the top of the grid** (y = 0). The vertical
    /// position comes in the concatenation, and that is what allows scrolling the
    /// screen by shifting the finished row instead of rebuilding the glyphs.
    fn line(&mut self, row: &tortty_core::grid::Row, y: usize, ctx: RowCtx) -> RowQuads {
        let stored = std::mem::take(&mut self.instances);
        let stored_color = std::mem::take(&mut self.emoji_row);
        self.instances.clear();
        self.emoji_row.clear();
        self.paint_line(row, y, ctx);
        RowQuads {
            mask: std::mem::replace(&mut self.instances, stored),
            emoji: std::mem::replace(&mut self.emoji_row, stored_color),
        }
    }

    /// Glyphs and decoration of one row, grouped into **runs** of the same face
    /// and style: it is the run context that makes the font swap each cell's
    /// glyph for the ligature piece (`calt`). A single-cell run has no context
    /// and goes straight through the font chain — that is the case of whoever
    /// paints cell by cell (doom-fire: 2400 runs of 1).
    fn paint_glyphs(&mut self, row: &tortty_core::grid::Row, y: usize, ctx: RowCtx) {
        let RowCtx {
            cols,
            sel,
            blink,
            cursor,
            links,
        } = ctx;
        // `y` here is the grid index (selection and cursor), **not** pixels: the
        // vertical position comes in `concat_linhas`. Adding `y * ch` in both
        // places painted row `r` at `2r` (A17 after per-row cache)
        let py = 0.0;
        let limit = cols.min(row.cells.len());
        let mut x = 0usize;
        while x < limit {
            let cell = row.cells[x];
            if cell.is_cont() || (cell.flags & tortty_core::cell::BLINK != 0 && !blink.phase) {
                x += 1;
                continue;
            }
            let (face, bold, italic, swap) = self.style(&cell, sel, y, x);
            let mut cells = vec![x];
            let mut text = String::new();
            text.push(cell.ch);
            let mut end = x + 1;
            while end < limit {
                let c2 = row.cells[end];
                if c2.is_cont() {
                    // half of a wide character belongs to the character
                    end += 1;
                    continue;
                }
                if c2.flags & tortty_core::cell::BLINK != 0 && !blink.phase {
                    break;
                }
                // the cursor breaks the run (see `cursor` in build)
                if cursor == Some((end, y)) || cursor == Some((end.wrapping_sub(1), y)) {
                    break;
                }
                if self.style(&c2, sel, y, end) != (face, bold, italic, swap) {
                    break;
                }
                cells.push(end);
                text.push(c2.ch);
                end += 1;
            }
            let glyphs = match face {
                Some(face) if cells.len() > 1 => {
                    let cw = self.fonts.cw;
                    let px = self.fonts.px;
                    let plan = self.shaper.plan(
                        self.fonts.face(face),
                        &text,
                        &cells,
                        self.ligatures,
                        cw,
                        px,
                    );
                    plan.map(|p| (face, p))
                },
                _ => None,
            };
            for (i, &cx) in cells.iter().enumerate() {
                let c = row.cells[cx];
                let glyph = match &glyphs {
                    Some((face, plan)) => plan.piece_at(i).map(|p| (*face, p.gid, p.x)),
                    None => None,
                };
                self.draw_cell(&c, cx, py, is_swap(&c, sel, y, cx), glyph, links);
            }
            x = end;
        }
    }

    /// Draws **one** grid row (A17/G17: only runs for dirty rows).
    fn paint_line(&mut self, row: &tortty_core::grid::Row, y: usize, ctx: RowCtx) {
        let RowCtx { cols, sel, .. } = ctx;
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        // relative to the top of the grid: whoever adds `y * ch` is `concat_linhas`
        let py = 0.0;

        // 1) backgrounds, grouped by colour (one instance per run). Inversion
        // (SGR 7) and the selection use the same rule: swap fg/bg and resolve the
        // default through the *other* extreme (otherwise a selection over a cell
        // without colour would not show up).
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

        self.paint_glyphs(row, y, ctx);
        self.linhas_remontadas = self.linhas_remontadas.saturating_add(1);
        self.celulas_remontadas = self.celulas_remontadas.saturating_add(cols as u64);
    }

    /// Face and attributes that define a shaping run, plus the inversion
    /// (selection/SGR 7), which swaps the colour and therefore splits runs.
    #[allow(clippy::type_complexity)]
    fn style(
        &self,
        cell: &tortty_core::cell::Cell,
        sel: Option<Rect>,
        y: usize,
        x: usize,
    ) -> (Option<usize>, bool, bool, bool) {
        let bold = cell.flags & tortty_core::cell::BOLD != 0;
        let italic = cell.flags & tortty_core::cell::ITALIC != 0;
        (
            self.fonts.face_of(cell.ch, bold, italic),
            bold,
            italic,
            is_swap(cell, sel, y, x),
        )
    }

    /// Uploads both instance buffers (mask and emoji). They grow by doubling:
    /// resizing in the middle of drawing cannot reallocate every frame.
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

    /// The surface did not hand out a texture: counts it in `skips` and warns
    /// **once** on stderr. It is the only signal the app can give that the
    /// problem is not the drawing (the grid stays alive in the pty and the
    /// socket) but the presentation — the symptom of a confined environment /
    /// no compositor (`M-AA`).
    fn refuse(&mut self) -> FrameOutcome {
        self.last = "skipped (timeout/occluded)";
        self.skips = self.skips.saturating_add(1);
        if !self.avisou_skip {
            self.avisou_skip = true;
            eprintln!(
                "tortty: the surface is not presenting (timeout/occluded) — the app draws, but \
                 the frame does not reach the screen. A confined environment without a visible \
                 GPU usually looks like this: check /dev/dri and, on sandbox, use `sandbox @gpu`; see \
                 docs/DEBUG.md"
            );
        }
        FrameOutcome::Skipped
    }

    /// Paints one frame, in blend order: grid → images → chrome and cursor → the
    /// colour glyphs of the chrome (`B8`).
    ///
    /// The mask and the colour glyphs travel in two buffers with two pipelines,
    /// and the cut in each list is what keeps the order (grid emoji under the
    /// images, the chrome's bell over the chrome).
    fn paint(&self, rp: &mut wgpu::RenderPass<'_>) {
        rp.set_pipeline(&self.pipeline);
        rp.set_bind_group(0, &self.globals_bind, &[]);
        rp.set_bind_group(1, &self.atlas_bind, &[]);
        rp.set_vertex_buffer(0, self.inst_buf.slice(..));
        // grid text → images → chrome and cursor. Two spans of the same buffer
        // because the image has its own pipeline (samples RGB).
        let split = self.img_split.min(self.instances.len()) as u32;
        if split > 0 {
            rp.draw(0..6, 0..split);
        }
        // colour emoji: same image pipeline, the texture is the RGBA atlas.
        // The grid's span comes here; the chrome's (the bell, `B8`) is drawn
        // after the chrome, at the end — the list is one buffer, cut in two
        let emoji_split = self.emoji_split.min(self.emoji_instances.len()) as u32;
        if emoji_split > 0 {
            rp.set_pipeline(&self.img_pipeline);
            rp.set_bind_group(1, &self.emoji_bind, &[]);
            rp.set_vertex_buffer(0, self.emoji_buf.slice(..));
            rp.draw(0..6, 0..emoji_split);
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(1, &self.atlas_bind, &[]);
            rp.set_vertex_buffer(0, self.inst_buf.slice(..));
        }
        if !self.img_instances.is_empty() {
            rp.set_pipeline(&self.img_pipeline);
            rp.set_vertex_buffer(0, self.img_buf.slice(..));
            // one draw call per image: each has its own texture
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
        // the colour glyphs of the chrome (the tab's bell, `B8`): they belong
        // above it, so they come last
        let emoji_all = self.emoji_instances.len() as u32;
        if emoji_all > emoji_split {
            rp.set_pipeline(&self.img_pipeline);
            rp.set_bind_group(1, &self.emoji_bind, &[]);
            rp.set_vertex_buffer(0, self.emoji_buf.slice(..));
            rp.draw(0..6, emoji_split..emoji_all);
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(1, &self.atlas_bind, &[]);
            rp.set_vertex_buffer(0, self.inst_buf.slice(..));
        }
    }

    pub fn render(&mut self, f: &Frame<'_>) -> FrameOutcome {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return self.refuse();
            },
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.last = "reconfigure";
                return FrameOutcome::Reconfigure;
            },
            wgpu::CurrentSurfaceTexture::Validation => {
                self.last = "fatal";
                return FrameOutcome::Fatal;
            },
        };

        let t0 = std::time::Instant::now();
        self.draw = self.draw.saturating_add(1);
        self.drawn = self.drawn.saturating_add(1);
        self.last = "drawn";
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
                label: Some("tortty-pass"),
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
            self.paint(&mut rp);
        }
        self.queue.submit(Some(enc.finish()));
        self.queue.present(frame);

        if prof::enabled() {
            prof::add_draw(t0.elapsed().as_nanos() as u64);
        }
        FrameOutcome::Drawn
    }
}

/// Protocol image → (width, height, RGBA8) for the texture.
///
/// `f=24` becomes RGBA with alpha 255; `f=32` passes through; `f=100` is PNG,
/// which the app decodes here (the core does not carry a PNG decoder).
fn decode(img: &tortty_core::graphics::Image) -> Option<(u32, u32, Vec<u8>)> {
    use tortty_core::graphics::Format;
    let (width, height) = (img.width, img.height);
    if width == 0 || height == 0 {
        return None;
    }
    match img.format {
        Format::Rgba => {
            let need = (width as usize) * (height as usize) * 4;
            (img.data.len() >= need).then(|| (width, height, img.data[..need].to_vec()))
        },
        Format::Rgb => {
            let need = (width as usize) * (height as usize) * 3;
            if img.data.len() < need {
                return None;
            }
            let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
            for px in img.data[..need].as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Some((width, height, out))
        },
        Format::Png => {
            let mut reader = png::Decoder::new(std::io::Cursor::new(&img.data))
                .read_info()
                .ok()?;
            let mut buf = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buf).ok()?;
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
                png::ColorType::Indexed => return None, // rare: icat sends RGBA
            }
            Some((pw, ph, out))
        },
    }
}

/// What to do after a frame (wgpu 30's `get_current_texture` returns an enum
/// instead of a `Result`).
/// Blink state in this frame (G12).
///
/// What the drawing of **one row** needs to know from the frame. It exists so as
/// not to drag five parameters through `build` → `rebuild_rows` → `line`.
#[derive(Clone, Copy)]
struct RowCtx {
    cols: usize,
    sel: Option<Rect>,
    blink: Blink,
    /// Cursor cell, when it is in this view (breaks the ligature).
    cursor: Option<(usize, usize)>,
    /// Ctrl is held: every `OSC 8` link underlines itself, so the reader sees
    /// what a click would open before clicking it.
    links: bool,
}

/// The cursor breaks the ligature: its cell is shaped on its own, otherwise it
/// paints in the middle of a glyph that covers more than one cell — it is kitty's
/// `disable_ligatures=cursor`. Outside the view (scrolled) there is no cursor.
fn break_no_cursor(term: &Term, view_off: usize, rows: usize) -> Option<(usize, usize)> {
    (view_off == 0 && term.cursor_visible && term.cy < rows).then_some((term.cx, term.cy))
}

/// `phase` toggles every 500 ms and applies to text with SGR 5; `cursor` already
/// comes resolved by the app (config `cursor_blink` + the program's `CSI 6 q`).
#[derive(Clone, Copy, Default)]
pub struct Blink {
    pub phase: bool,
    pub cursor: bool,
}

/// What the bar needs to know about each tab (the renderer does not know `Tab`).
pub struct TabInfo {
    pub title: String,
    pub ativa: bool,
    /// Recent BELL: the label gets a highlight for ~800 ms (I13).
    pub alert: bool,
    /// Progress the program reports about itself (`OSC 9;4`): a bar on the
    /// bottom edge of the tab.
    pub progress: Option<Progress>,
}

/// Height of that bar. Two pixels on the bottom edge: thin enough not to eat the
/// label, and the width is in pixels (37% of a 20-cell tab is 7.4 cells of ink).
const PROGRESS_H: f32 = 2.0;

/// Colour and fill of a progress report: the shade follows the state, in the
/// same three tones the toast uses, and a state with no measure fills the tab
/// (there is a band, the program just cannot say how wide). Indeterminate is the
/// only translucent one, and it does not move — moving it would need a clock and
/// a frame per step, for a fact the state already tells.
fn progress_ink(p: Progress) -> (Color, f32, f32) {
    let frac = p.pct.map_or(1.0, |pct| f32::from(pct) / 100.0);
    match p.state {
        2 => (Color::Idx(1), 1.0, frac),
        4 => (Color::Idx(3), 1.0, frac),
        3 => (Color::Idx(6), 0.35, 1.0),
        _ => (Color::Idx(2), 1.0, frac),
    }
}

/// What a notice is telling the owner: the accent (bar and border) and, on the
/// app's side, how long it stays. Three shades, not a spectrum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// A receipt: something worked.
    Ok,
    /// Not the owner's fault, but it costs attention.
    Warn,
    /// Something did not work and the owner may have to act.
    Err,
}

impl Severity {
    /// Palette slot the accent comes from, so a theme with its own palette moves
    /// the toast with it and no key is added for that.
    fn color(self) -> Color {
        match self {
            Severity::Ok => Color::Idx(2),
            Severity::Warn => Color::Idx(3),
            Severity::Err => Color::Idx(1),
        }
    }
}

/// The notice as the renderer needs it: own text, like `TabInfo`/`WsCard`, so the
/// app can build it and drop it while the renderer is `&mut`.
pub struct ToastText {
    pub title: String,
    /// Second level, wrapped: what happened, where, which key. Empty is fine.
    pub detail: String,
    pub severity: Severity,
}

/// Breaks `text` into lines of at most `n` characters, preferring the space
/// (a word is not cut while it fits) and cutting a word longer than the whole
/// line, which has nowhere to break — a path or a URL must still fit the box.
/// At most `max` lines: the rest is dropped, never drawn under the margin.
fn wrap(text: &str, n: usize, max: usize) -> Vec<String> {
    let n = n.max(1);
    let mut words: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        let mut rest = word;
        while rest.chars().count() > n {
            let cut = rest.char_indices().nth(n).map_or(rest.len(), |(i, _)| i);
            words.push(&rest[..cut]);
            rest = &rest[cut..];
        }
        if !rest.is_empty() {
            words.push(rest);
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in words {
        let sep = usize::from(!line.is_empty());
        if !line.is_empty() && line.chars().count() + sep + word.chars().count() > n {
            out.push(std::mem::take(&mut line));
            if out.len() == max {
                return out;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() && out.len() < max {
        out.push(line);
    }
    out
}

/// Everything a frame needs, in one package: the signature of `build`/`render`
/// grew one parameter per slice (tabs, sidebar, toast...).
pub struct Frame<'a> {
    /// Identity of the drawn tab (see `damage`).
    pub tab: u64,
    pub term: &'a Term,
    pub view_off: usize,
    pub sel: Option<Span>,
    pub blink: Blink,
    pub toast: Option<&'a ToastText>,
    pub tabs: Option<&'a [TabInfo]>,
    pub sidebar: Option<&'a [WsCard]>,
    /// First card of the sidebar list to draw (`B42`).
    pub cards_off: usize,
    /// How many whole cards fit: the rest of the column is the strip a drag
    /// walks in (`B55`), so a card is never drawn there.
    pub cards_visible: usize,
    /// Ctrl is held: the `OSC 8` links underline themselves.
    pub links: bool,
}

/// What the sidebar needs from each workspace (I6).
pub struct WsCard {
    /// The name the owner typed (`I8`); empty means never renamed (`B43`).
    pub name: String,
    /// Title of the selected tab — the automatic half of the card.
    pub title: String,
    pub ativa: bool,
    /// A tab of this workspace has an unacknowledged notification (`M-K`).
    pub alert: bool,
    /// Progress of the selected tab (`OSC 9;4`), on the bottom edge of the card:
    /// the same report the tab label shows, so a parked workspace is not blind.
    pub progress: Option<Progress>,
}

pub enum FrameOutcome {
    Drawn,
    Skipped,
    Reconfigure,
    Fatal,
}

/// Theme colour: `Default` falls back to the program default, the rest resolves
/// through the palette (the first 16 can come from the configuration file).
fn theme(cfg: &Config, c: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    match c {
        Color::Default => default,
        other => tortty_core::color::resolve(other, &cfg.palette256(), default),
    }
}

/// Returns a new buffer only when the capacity does not fit; `None` keeps the
/// current one (avoids reallocating every frame).
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

/// The two rows of one card (`B43`): the number never stands alone, and the
/// tab title drops to the second row only when an explicit name took the
/// first one. Without a name, the second row is empty — the card keeps its
/// 2.4 rows and the title does not move down for nothing.
fn card_lines(n: usize, name: &str, title: &str) -> (String, String) {
    // a name of spaces is no name: `finish_rename` trims, but the state file
    // is hand-editable
    let name = name.trim();
    if name.is_empty() {
        (format!("{n} {title}"), String::new())
    } else {
        (format!("{n} {name}"), title.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{card_lines, is_swap, progress_ink, wrap};
    use tortty_core::cell::Cell;
    use tortty_core::select::Rect;
    use tortty_core::term::Progress;
    use tortty_core::Color;

    /// The detail of a notice fills the box up to the margin, breaks at the space
    /// and never leaves a line wider than the box.
    #[test]
    fn a_notice_detail_wraps_at_the_space() {
        assert_eq!(
            wrap("alpha beta gamma delta", 11, 4),
            vec!["alpha beta", "gamma delta"]
        );
    }

    /// A word with no space in it — a path, a URL — is cut where it reaches the
    /// margin instead of overflowing the box.
    #[test]
    fn a_word_longer_than_the_line_is_cut() {
        assert_eq!(
            wrap("/usr/local/share/very/long/path", 10, 4),
            vec!["/usr/local", "/share/ver", "y/long/pat", "h"]
        );
    }

    /// The cap is lines: a notice that would be taller than the grid keeps the
    /// first lines and drops the rest instead of covering the whole screen.
    #[test]
    fn the_detail_stops_at_the_line_cap() {
        assert_eq!(wrap("a b c d e f g", 1, 2), vec!["a", "b"]);
    }

    /// `B5`: the rectangle inverts the same columns on every one of its rows,
    /// while a flow selection only cuts the first and the last.
    #[test]
    fn a_block_inverts_a_column_and_a_flow_inverts_lines() {
        let cell = Cell::default();
        let block = Rect {
            r1: 1,
            c1: 3,
            r2: 3,
            c2: 5,
            block: true,
        };
        assert!(is_swap(&cell, Some(block), 1, 3));
        assert!(is_swap(&cell, Some(block), 2, 4), "the middle row too");
        assert!(!is_swap(&cell, Some(block), 2, 6), "outside the column");
        assert!(!is_swap(&cell, Some(block), 2, 2), "left of the column");
        assert!(!is_swap(&cell, Some(block), 4, 4), "below the rectangle");
        let flow = Rect {
            block: false,
            ..block
        };
        assert!(
            is_swap(&cell, Some(flow), 2, 4),
            "a flow selection covers the whole middle row"
        );
        assert!(
            is_swap(&cell, Some(flow), 2, 0),
            "even where the block does not reach"
        );
        assert!(
            is_swap(&cell, Some(flow), 2, 6),
            "the middle row of a flow selection is whole"
        );
        assert!(
            !is_swap(&cell, Some(flow), 3, 6),
            "and its last row stops at the end column"
        );
    }

    /// `OSC 9;4`: the shade follows the state and the fill follows the fraction.
    #[test]
    fn progress_ink_follows_the_state() {
        // the three shades of the toast, opaque, filled by the fraction
        assert_eq!(
            progress_ink(Progress {
                state: 1,
                pct: Some(37)
            }),
            (Color::Idx(2), 1.0, 37.0 / 100.0)
        );
        assert_eq!(
            progress_ink(Progress {
                state: 2,
                pct: Some(50)
            })
            .0,
            Color::Idx(1)
        );
        assert_eq!(
            progress_ink(Progress {
                state: 4,
                pct: None
            }),
            (Color::Idx(3), 1.0, 1.0)
        );
        // indeterminate is the only translucent one, and it fills the tab: it
        // does not move, because moving costs a clock and a frame per step
        assert_eq!(
            progress_ink(Progress {
                state: 3,
                pct: None
            }),
            (Color::Idx(6), 0.35, 1.0)
        );
        // a state with no fraction still gets a band
        assert_eq!(
            progress_ink(Progress {
                state: 1,
                pct: None
            }),
            (Color::Idx(2), 1.0, 1.0)
        );
    }

    #[test]
    fn the_card_never_stands_on_the_number_alone() {
        // renamed: the name takes the first row and the title the second
        assert_eq!(
            card_lines(2, "build", "make"),
            ("2 build".to_string(), "make".to_string())
        );
        // never renamed: the title climbs to the first row and nothing is drawn
        // underneath — no `(3 tabs)`, no placeholder
        assert_eq!(
            card_lines(1, "", "vim ~/.zshrc"),
            ("1 vim ~/.zshrc".to_string(), String::new())
        );
        // the title is whatever `Tab::title` answers, fallback included
        assert_eq!(
            card_lines(3, "", "tortty"),
            ("3 tortty".to_string(), String::new())
        );
        // a name cleared in the rename field is the same as never renamed
        assert_eq!(
            card_lines(4, "   ", "htop"),
            ("4 htop".to_string(), String::new())
        );
        // and the word the column already is does not come back
        for (line, _) in [card_lines(1, "", "tortty"), card_lines(2, "build", "make")] {
            assert!(!line.contains("workspace"), "{line}");
        }
    }
}
