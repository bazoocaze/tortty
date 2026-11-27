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
use tortty_core::config::{Config, Theme};
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
    /// `uv` of the corner tile in the mask atlas (`B57`); `None` = the atlas
    /// refused it, and the boxes fall back to square corners.
    toast_corner: Option<[f32; 4]>,
    /// `bold_as_bright`: bold text on the low half of the palette draws the
    /// bright half (`color::bright`).
    bold_as_bright: bool,
    /// `progress_show`: where the program's progress is drawn (`B47`).
    ///
    /// A field here that comes from the config has **two** readers: this
    /// constructor and `apply_layout`, which is what a reload runs. Missing the
    /// second is invisible — the app's copy of the config already changed, so the
    /// `state` verb reports the new value while the screen keeps the old one.
    progress_show: tortty_core::config::ProgressShow,
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
    /// `M-D`: 0 = ordinary cell, 1 = a match, 2 = the current match. Rebuilt with
    /// the frame, indexed `row * cols + col`.
    marks: Vec<u8>,
    /// `B69`: the plain-text links of the visible window, as `(col, end)` per view
    /// row. Rebuilt with the frame and only while Ctrl is held — the modifier that
    /// both underlines and opens them.
    row_links: Vec<Vec<(u32, u32)>>,
    /// The runs of the row `draw_cell` is painting (taken from `row_links`), so the
    /// per-cell test does not carry a sixth parameter like `cur_mark`.
    link_mask: Vec<(u32, u32)>,
    /// The `cols` the mask was built with (not the signature's: the rows are
    /// painted before the signature is stored).
    marks_cols: usize,
    /// Mark of the cell `draw_cell` is painting (the caller knows the row).
    cur_mark: u8,
    /// How many of `img_instances` are drawn **below** the text (`z < 0`).
    img_below: usize,
    /// Texture per image (id → bind group). The bind group keeps the texture alive.
    img_cache: HashMap<u32, ImgTex>,
    /// Combining marks of the cell `draw_cell` is painting (`M-U`), and how many.
    cur_marks: [char; tortty_core::grid::MAX_MARKS],
    n_marks: usize,
    /// The whole grapheme cluster of the cell being painted (`B66`): the base and
    /// everything the row keeps beside it. Empty for a cell with no tail, which
    /// is almost every cell.
    cur_cluster: String,
    /// How many codepoints the cell has beside its base, so the cluster path is
    /// not even looked for when there are none.
    n_tail: usize,
    /// Composed cluster glyphs already rasterized, by style and cluster. A family
    /// or a flag repeats on screen (a listing of emoji, a redraw) and shaping is
    /// not free.
    cluster_cache: HashMap<(u8, String), Option<GlyphQuad>>,
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
    /// Rows and cells the last frames actually rebuilt (`A17`/`G17`): the
    /// proof that partial drawing works, and the numbers the socket publishes.
    pub rows_built: u64,
    pub cells_built: u64,
}

struct ImgTex {
    version: u64,
    /// The frame the texture was built from (`B2`): an animation changes the
    /// picture without changing the image's `version`.
    frame: u32,
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

        let mut atlas = Atlas::new(&device);
        // the toast's rounded corner (`B57`): one anti-aliased quarter disc in the
        // mask atlas, sampled mirrored by the four corners of every box drawn
        let toast_corner = atlas.add(CORNER, CORNER, &corner_mask(CORNER));
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
            marks: Vec::new(),
            marks_cols: 0,
            cur_mark: 0,
            row_links: Vec::new(),
            link_mask: Vec::new(),
            img_below: 0,
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
            progress_show: cfg.progress_show,
            toast_corner,
            instances: Vec::with_capacity(4096),
            palette: cfg.palette256(),
            fg: theme(cfg, cfg.fg, FG),
            bg: theme(cfg, cfg.bg, BG),
            cursor: theme(cfg, cfg.cursor, FG),
            pad: cfg.padding as f32,
            bar: 0.0,
            side: 0.0,
            img_split: 0,
            emoji_split: 0,
            img_pipeline,
            img_buf,
            img_instances: Vec::new(),
            img_ids: Vec::new(),
            img_layout,
            img_sampler,
            img_cache: HashMap::new(),
            cur_marks: ['\0'; tortty_core::grid::MAX_MARKS],
            n_marks: 0,
            cur_cluster: String::new(),
            n_tail: 0,
            cluster_cache: HashMap::new(),
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
            rows_built: 0,
            cells_built: 0,
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
        self.pad = cfg.padding as f32;
        // `progress_show` is drawn with the chrome, which every frame rebuilds, so
        // it takes effect on the next frame; `bold_as_bright` changes the ink of
        // the cells already cached, and those have to be painted again
        self.progress_show = cfg.progress_show;
        if self.bold_as_bright != cfg.bold_as_bright {
            self.bold_as_bright = cfg.bold_as_bright;
            self.theme_gen = self.theme_gen.wrapping_add(1);
        }
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

    /// A rounded rectangle: a cross of three rects plus the four corner tiles,
    /// sampled with `u`/`v` mirrored (`B57`).
    // x/y/w/h/r are the four edges and the radius of a rectangle: renaming them
    // only moves the code further from the drawing
    #[allow(clippy::many_single_char_names)]
    fn rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: [f32; 4]) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let Some([u0, v0, u1, v1]) = self.toast_corner.filter(|_| r > 0.5) else {
            self.push_rect(x, y, w, h, color);
            return;
        };
        self.push_rect(x + r, y, w - 2.0 * r, h, color);
        self.push_rect(x, y + r, r, h - 2.0 * r, color);
        self.push_rect(x + w - r, y + r, r, h - 2.0 * r, color);
        let tiles = [
            (x, y, [u0, v0, u1, v1]),
            (x + w - r, y, [u1, v0, u0, v1]),
            (x, y + h - r, [u0, v1, u1, v0]),
            (x + w - r, y + h - r, [u1, v1, u0, v0]),
        ];
        for (tx, ty, uv) in tiles {
            self.instances.push(Instance {
                rect: [tx, ty, r, r],
                color,
                uv,
            });
        }
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
    /// Lays out and draws the chrome of this frame (`M-Q`): the bar and the cards
    /// draw from the scene, and `app.rs` hit-tests the same rects — the drawing
    /// formula and the click formula are one.
    fn build_chrome(
        &mut self,
        tabs: Option<&[TabInfo]>,
        sidebar: Option<&[WsCard]>,
        cards_off: usize,
        m: crate::chrome::Metrics,
    ) {
        let specs: Vec<crate::chrome::Tab<'_>> = tabs.map_or_default(|list| {
            list.iter()
                .map(|t| crate::chrome::Tab {
                    title: &t.title,
                    alert: t.alert,
                })
                .collect()
        });
        let ws = sidebar.map_or(0, <[WsCard]>::len);
        let layout = crate::chrome::Layout::build(m, &specs, ws, cards_off);
        if let Some(tabs) = tabs {
            self.tabbar(tabs, &layout);
        }
        if let Some(cards) = sidebar {
            self.sidebar(cards, &layout);
        }
    }

    /// The bar: one slot per tab, taken from the scene (`M-Q`) — the same rect
    /// the app hit-tests, so a click and a label cannot disagree.
    fn tabbar(&mut self, tabs: &[TabInfo], layout: &crate::chrome::Layout) {
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        #[allow(clippy::cast_precision_loss)] // columns, far below 2^23
        let total_w = layout.m.cols as f32 * cw;
        let bg = self.color(Color::Idx(0), self.bg);
        // the selected tab wears its own background with a white label now
        // (`SEL_BG`/`SEL_FG` above), not the theme inversion
        // `origin.y` is `pad + bar`: so the bar lands in the reserved strip
        // (and not over the first grid row) it is drawn at y = -bar
        let y0 = -self.bar;
        self.push_rect(0.0, y0, total_w, chh, [bg[0], bg[1], bg[2], 1.0]);
        for slot in &layout.tabs {
            let Some(t) = tabs.get(slot.i) else {
                continue;
            };
            let [x, y, width, _] = slot.rect;
            // a tab with a notification keeps its colours and carries the bell
            // (`B8`): painting the whole label was louder than the news
            let bell = if t.alert { 2.0 * cw } else { 0.0 };
            let spec = crate::chrome::Tab {
                title: &t.title,
                alert: t.alert,
            };
            let label = crate::chrome::tab_label(slot.i, &spec);
            let (bg, fg) = if t.ativa {
                (chrome_rgb(SEL_BG), chrome_rgb(SEL_FG))
            } else {
                // the button's own body, one step up from the bar, and one more
                // under the pointer: the two steps are the same mix, towards
                // the label's own ink. The bar's face is `color0`; the step is
                // computed on its RGB, not on the slot, so the body does not
                // move when the palette does
                let bar_bg = tortty_core::color::resolve(Color::Idx(0), &self.palette, self.bg);
                let body = mix(bar_bg, INK_INACTIVE, 0.14);
                let body = if t.hover {
                    mix(body, INK_INACTIVE, 0.14)
                } else {
                    body
                };
                (chrome_rgb(body), chrome_rgb(INK_INACTIVE))
            };
            // one rectangle per tab (not one per letter): the label background is
            // a single piece, and the text is drawn on top of it
            self.push_rect(x, y, width, chh, [bg[0], bg[1], bg[2], 1.0]);
            let text_x = x + bell;
            if t.alert {
                self.chrome_bell(x, y, fg);
            }
            self.chrome_text(&label, text_x, y, fg, width - bell);
            // the bar is one of the two faces of `progress_show`: `title` puts
            // the number on the label instead, and `none` draws nothing
            if let Some(p) = t
                .progress
                .filter(|_| self.progress_show == tortty_core::config::ProgressShow::Bar)
            {
                let (c, alpha, frac) = progress_ink(p);
                let rgb = self.color(c, self.fg);
                self.push_rect(
                    x,
                    y + chh - PROGRESS_H,
                    width * frac,
                    PROGRESS_H,
                    [rgb[0], rgb[1], rgb[2], alpha],
                );
            }
        }
    }

    /// Workspace sidebar (I6): a column on the left with one card per workspace.
    /// It occupies the strip the `origin` reserved (x = -side) and starts on row
    /// 1, together with the tab bar (y = -bar).
    ///
    /// It is drawn from card `off` (`B42`): the list scrolls, so the first card
    /// can start above the column and the last one is cut at the bottom of the
    /// grid — the strip ends there, not at the window's edge.
    fn sidebar(&mut self, cards: &[WsCard], layout: &crate::chrome::Layout) {
        if self.side <= 0.0 {
            return;
        }
        let chh = self.fonts.ch;
        // the column's bottom, in this coordinate space (y = 0 is the first grid
        // row): the cards and their text are cut here
        let bottom = layout.m.card_bottom();
        for slot in &layout.cards {
            let Some(c) = cards.get(slot.i) else {
                continue;
            };
            let (i, [x0, y0, width, height]) = (slot.i, slot.rect);
            let y1 = y0 + height;
            let active = c.ativa;
            let bar_bg = tortty_core::color::resolve(Color::Idx(0), &self.palette, self.bg);
            let (bg, fg) = if active {
                (chrome_rgb(SEL_BG), chrome_rgb(SEL_FG))
            } else {
                // the card's body, one step up from the column, and one more
                // under the pointer — the same two steps the tab takes
                let body = mix(bar_bg, INK_INACTIVE, 0.14);
                let body = if c.hover {
                    mix(body, INK_INACTIVE, 0.14)
                } else {
                    body
                };
                (chrome_rgb(body), chrome_rgb(INK_INACTIVE))
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
            if let Some(p) = c
                .progress
                .filter(|_| self.progress_show == tortty_core::config::ProgressShow::Bar)
            {
                let (color, alpha, frac) = progress_ink(p);
                // the bar keeps the state's colour even while the card alerts
                // (it used to turn `over` here, which on a dark card was
                // invisible): the yellow text carries the alert, the bar the
                // measure — and the alert tint wins over the white, as agreed
                let rgb = self.color(color, self.fg);
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
    fn toast(&mut self, t: &ToastText, m: crate::chrome::Metrics) {
        // how far the surface is pulled away from the grid's own colour
        const VEIL: f32 = 0.14;
        // the box's wrap, its width and its place are `chrome::Toast`'s: what is
        // drawn here is what a press tests (`M-Q`)
        let Some(box_) = crate::chrome::Toast::build(m, &t.title, &t.detail) else {
            return;
        };
        let [x1, y1, bw, bh] = box_.rect;
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
        // `B57`: the box has a rounded corner and a shadow made of two veils, the
        // lighter one a step further out — nested rects cannot be soft, so the
        // softness is the sum of two edges
        let r = CORNER as f32;
        self.rounded_rect(
            x1 - 2.0,
            y1 + 3.0,
            bw + 4.0,
            bh + 4.0,
            r + 2.0,
            [0.0, 0.0, 0.0, 0.10],
        );
        self.rounded_rect(
            x1 - 1.0,
            y1 + 1.0,
            bw + 2.0,
            bh + 2.0,
            r + 1.0,
            [0.0, 0.0, 0.0, 0.12],
        );
        // the border is the ring between two rounded rects, not four 1 px lines
        self.rounded_rect(x1, y1, bw, bh, r, edge);
        self.rounded_rect(
            x1 + 1.0,
            y1 + 1.0,
            bw - 2.0,
            bh - 2.0,
            r - 1.0,
            [base[0], base[1], base[2], 1.0],
        );
        self.rounded_rect(
            x1 + 1.0,
            y1 + 1.0,
            bw - 2.0,
            bh - 2.0,
            r - 1.0,
            [ink[0], ink[1], ink[2], VEIL],
        );
        self.push_rect(x1 + 1.0, y1 + 2.0, crate::chrome::TOAST_BAR, bh - 4.0, edge);
        let (tx, ty) = box_.title_at(m);
        self.chrome_text(&box_.title, tx, ty, ink, 0.0);
        for (i, line) in box_.detail.iter().enumerate() {
            let (lx, ly) = box_.line_at(m, i);
            self.chrome_text(line, lx, ly, dim, 0.0);
        }
    }

    /// The glyph colour of a cell. `M-D`: on the bright amber of the current match
    /// the theme's text colour vanishes, so that glyph goes dark.
    fn ink(&self, fg: Color, swap: bool) -> [f32; 3] {
        if self.cur_mark == 2 {
            return [0.1, 0.08, 0.0];
        }
        self.color(fg, if swap { self.bg } else { self.fg })
    }

    /// The mark of one cell: 0 none, 1 a match, 2 the current match (`M-D`).
    fn mark_at(&self, row: usize, col: usize) -> u8 {
        self.marks
            .get(row * self.marks_cols + col)
            .copied()
            .unwrap_or(0)
    }

    /// The grid's context menu (`M-V`): a chip like the search bar's — the same
    /// opaque surface, so the grid behind it does not read through — with one row
    /// per item and the row under the pointer filled in.
    fn menu(&mut self, mv: &MenuView) {
        let cw = self.fonts.cw;
        let [x0, y0, w, _] = mv.layout.rect;
        let base = self.color(Color::Default, self.bg);
        let ink = self.color(Color::Default, self.fg);
        let accent = self.color(Color::Idx(4), self.fg);
        let dim = [
            ink[0] * 0.45 + base[0] * 0.55,
            ink[1] * 0.45 + base[1] * 0.55,
            ink[2] * 0.45 + base[2] * 0.55,
        ];
        let corner = CORNER as f32;
        let height = mv.layout.rect[3];
        // shadow, body and edge: the same three layers as `chip`, drawn at once
        // because a menu is one box and not a row of boxes
        self.rounded_rect(
            x0 - 1.0,
            y0 + 2.0,
            w + 2.0,
            height + 1.0,
            corner + 1.0,
            [0.0, 0.0, 0.0, 0.12],
        );
        self.rounded_rect(x0, y0, w, height, corner, [0.6, 0.6, 0.6, 1.0]);
        let surface = [
            base[0] + (ink[0] - base[0]) * 0.14,
            base[1] + (ink[1] - base[1]) * 0.14,
            base[2] + (ink[2] - base[2]) * 0.14,
        ];
        self.rounded_rect(
            x0 + 1.0,
            y0 + 1.0,
            w - 2.0,
            height - 2.0,
            corner - 1.0,
            [surface[0], surface[1], surface[2], 1.0],
        );
        for slot in &mv.layout.rows {
            let [x, y, width, h] = slot.rect;
            let Some(label) = mv.layout.items.get(slot.i).map(|i| i.label()) else {
                continue;
            };
            let enabled = mv.enabled.get(slot.i).copied().unwrap_or(false);
            if mv.hover == Some(slot.i) && enabled {
                self.push_rect(x, y, width, h, [accent[0], accent[1], accent[2], 0.22]);
            }
            let fg = if enabled { ink } else { dim };
            // `+1.0`: a cell is a fractional number of pixels, and `ceil` is what
            // keeps the last letter of a label from falling off a rounded-down
            // limit — the gutter has the pixel to spare
            self.chrome_text(label, x + cw, y, fg, width - 2.0 * cw + 1.0);
        }
    }

    /// The search bar (`M-D`): two chips on the grid's bottom row, `[find:
    /// query_ ]` and `[ 1/3 ]`. The brackets are text, so the shape reads the same
    /// at any cell width; the surface is opaque and one step off the grid's, which
    /// is what keeps it from blending into the background behind it.
    fn search_bar(&mut self, sv: &SearchView, m: crate::chrome::Metrics) {
        let (cols, rows) = (m.cols, m.rows);
        let cw = self.fonts.cw;
        let chh = self.fonts.ch;
        let base = self.color(Color::Default, self.bg);
        let ink = self.color(Color::Default, self.fg);
        // the label recedes, the query is the ink, and the cursor is the amber of
        // the current match: the two ends of the search read as one thing
        let dim = [
            ink[0] * 0.78 + base[0] * 0.22,
            ink[1] * 0.78 + base[1] * 0.22,
            ink[2] * 0.78 + base[2] * 0.22,
        ];
        let amber = self.color(Color::Rgb(0xd7, 0xa5, 0x00), self.fg);
        let grey = self.color(Color::Idx(8), self.fg);
        let red = self.color(Color::Idx(1), self.fg);
        // `[find: ` + query + `_ ]` is `q + 10` cells; `[ 1/3 ]` is `n + 4`. The
        // chip is exactly as wide as its text: the brackets are the shape, so any
        // padding would double the edge
        let count = sv.count.clone();
        let count_cells = count.as_ref().map_or(0, |c| c.chars().count() + 4);
        let room = cols.saturating_sub(count_cells);
        // a long query keeps its tail, where the cursor is: the head is what a
        // reader can spare
        let mut query: String = sv.query.clone();
        let fits = room.saturating_sub(10);
        if query.chars().count() > fits {
            query = query.chars().skip(query.chars().count() - fits).collect();
        }
        let q = query.chars().count();
        let find_cells = (q + 10).min(room).max(1);
        #[allow(clippy::cast_precision_loss)]
        let y = rows.saturating_sub(1) as f32 * chh;
        #[allow(clippy::cast_precision_loss)]
        let (find_w, count_w) = (find_cells as f32 * cw, count_cells as f32 * cw);
        self.chip((0.0, y), (find_w, chh), grey);
        #[allow(clippy::cast_precision_loss)]
        let x = find_cells as f32 * cw;
        if count.is_some() {
            // a search that found nothing says so in red, not in grey
            let edge = if sv.count.as_deref() == Some("no match") {
                red
            } else {
                grey
            };
            self.chip((x, y), (count_w, chh), edge);
        }
        #[allow(clippy::cast_precision_loss)]
        let cell = |n: usize| n as f32 * cw;
        self.chrome_text("[", cell(0), y, dim, 0.0);
        self.chrome_text("find: ", cell(1), y, dim, 0.0);
        self.chrome_text(&query, cell(7), y, ink, 0.0);
        self.chrome_text("_", cell(7 + q), y, amber, 0.0);
        self.chrome_text(" ]", cell(8 + q), y, dim, 0.0);
        if let Some(c) = count.as_ref() {
            let edge = if c == "no match" { red } else { ink };
            self.chrome_text("[ ", x, y, grey, 0.0);
            self.chrome_text(c, x + cell(2), y, edge, 0.0);
            self.chrome_text(" ]", x + cell(2) + cell(c.chars().count()), y, grey, 0.0);
        }
    }

    /// The bottom row of the grid carries the search bar (`M-D`), the one
    /// overlay that lives there.
    fn bottom_row(&mut self, search: Option<&SearchView>, m: crate::chrome::Metrics) {
        if let Some(sv) = search {
            self.search_bar(sv, m);
        }
    }

    /// A chip: the search bar's surface — a hair of ink over the grid's
    /// background, so it is opaque (nothing behind shows through) and still reads
    /// as chrome, with a 1 px edge to keep the corner crisp.
    fn chip(&mut self, at: (f32, f32), size: (f32, f32), edge: [f32; 3]) {
        /// How much of the text colour is mixed into the grid's background.
        const VEIL: f32 = 0.14;
        let base = self.color(Color::Default, self.bg);
        let ink = self.color(Color::Default, self.fg);
        let surface = [
            base[0] + (ink[0] - base[0]) * VEIL,
            base[1] + (ink[1] - base[1]) * VEIL,
            base[2] + (ink[2] - base[2]) * VEIL,
        ];
        let corner = CORNER as f32;
        let (left, top) = at;
        let (width, height) = size;
        self.rounded_rect(
            left - 1.0,
            top + 2.0,
            width + 2.0,
            height + 1.0,
            corner + 1.0,
            [0.0, 0.0, 0.0, 0.12],
        );
        self.rounded_rect(
            left,
            top,
            width,
            height,
            corner,
            [edge[0], edge[1], edge[2], 1.0],
        );
        self.rounded_rect(
            left + 1.0,
            top + 1.0,
            width - 2.0,
            height - 2.0,
            corner - 1.0,
            [surface[0], surface[1], surface[2], 1.0],
        );
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

    /// `B69`: the plain-text links of the visible window, into `row_links`.
    ///
    /// One scan per frame, over the rows in sight, and only while Ctrl is held.
    /// Nothing is kept across frames because the text underneath changes without
    /// telling the renderer — a stale underline over the wrong cells is worse than
    /// a scan of fifty rows.
    fn scan_links(&mut self, rows: &[&tortty_core::grid::Row]) {
        if self.row_links.len() == rows.len() {
            for slot in &mut self.row_links {
                slot.clear();
            }
        } else {
            self.row_links.clear();
            self.row_links.resize(rows.len(), Vec::new());
        }
        for link in tortty_core::links::scan(rows) {
            for run in link.runs {
                if let Some(slot) = self.row_links.get_mut(run.row) {
                    let start = u32::try_from(run.col).unwrap_or(u32::MAX);
                    let end = u32::try_from(run.col.saturating_add(run.len)).unwrap_or(u32::MAX);
                    slot.push((start, end));
                }
            }
        }
    }

    /// `true` when column `x` of the row being painted is inside a plain-text
    /// link (`B69`). The runs of a row are few, so the walk beats a search.
    fn in_link(&self, x: usize) -> bool {
        let Ok(x) = u32::try_from(x) else {
            return false;
        };
        self.link_mask
            .iter()
            .any(|&(start, end)| start <= x && x < end)
    }

    /// Glyph + decoration of one cell. The order here is the blend order:
    /// everything of the text comes before the cursor.
    /// `glyph` = glyph already resolved by shaping (`face`, `gid`, offset in px).
    /// `None` means resolve the character through the chain — the single-cell run
    /// path; a shaping glyph that does not draw also falls back to the chain.
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
        let mut c = self.ink(fg, swap);
        // faint: the same tone with less opacity (Pango used alpha 0.55)
        let alpha = if cell.flags & FAINT != 0 { 0.55 } else { 1.0 };

        self.paint_glyph(cell, x, py, glyph, [c[0], c[1], c[2], alpha]);

        // `M-U`: a combining mark has no cell of its own — it is drawn over the
        // one it follows, at the same origin, which is where the font put it
        if self.n_marks > 0 {
            let bold = cell.flags & BOLD != 0;
            let italic = cell.flags & ITALIC != 0;
            self.paint_marks(x, py, [c[0], c[1], c[2], alpha], bold, italic);
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
        // `OSC 8` link, or the URL the text spells (`B69`): the underline is the
        // only thing that says "this text is a link", and it is drawn while Ctrl
        // is held — the modifier that opens it. A cell with an underline of its
        // own keeps its style.
        if links && cell.ul == tortty_core::cell::UL_NONE && (cell.link != 0 || self.in_link(x)) {
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
        // `M-L`: `z` orders the placements, and the negative ones belong **behind**
        // the text — `img_below` counts them, and `paint` draws that span first
        let mut placed: Vec<&tortty_core::graphics::Placement> =
            term.graphics.placements().iter().collect();
        placed.sort_by_key(|p| p.z);
        self.img_below = placed.iter().take_while(|p| p.z < 0).count();
        let mut quads: Vec<(u32, Instance)> = Vec::new();
        for pl in placed {
            // `U=1` placements are prototypes: the grid's `U+10EEEE` cells below
            // decide where the picture goes
            if pl.placeholder {
                continue;
            }
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
        self.placeholder_cells(f, (cw, chh), &mut quads);
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

    /// The glyph of one cell, with the two special cases it has: the half-pixel
    /// overlap that makes box-drawing neighbours meet, and the placeholder cell,
    /// whose private-use glyph is not drawn at all because an image goes there.
    fn paint_glyph(
        &mut self,
        cell: &Cell,
        x: usize,
        py: f32,
        glyph: Option<(usize, u16, f32)>,
        colour: [f32; 4],
    ) {
        let cw = self.fonts.cw;
        let ascent = self.fonts.ascent;
        let alpha = colour[3];
        let bold = cell.flags & tortty_core::cell::BOLD != 0;
        let italic = cell.flags & tortty_core::cell::ITALIC != 0;
        // the placeholder cell is a pixel of an image (`M-L`), which the image
        // pass draws over it: the glyph itself would be a box
        if cell.ch == ' ' || cell.ch == tortty_core::placeholder::PLACEHOLDER {
            return;
        }
        let quad = self
            .cluster_quad(bold, italic)
            .or_else(|| glyph.and_then(|(face, gid, _)| self.quad_gid(face, gid)))
            .or_else(|| self.glyph_quad(cell.ch, bold, italic));
        let desloc = glyph.map_or(0.0, |(_, _, dx)| dx);
        let Some(q) = quad else {
            return;
        };
        let mut gx = x as f32 * cw + desloc + q.left as f32;
        let mut gw = q.w as f32;
        // Box drawing and block elements are the glyphs that touch the
        // neighbour's ink on purpose: at a fractional cell width the antialias of
        // two neighbours meets with less than full ink, and a dim column shows at
        // the seam. The quad goes half a pixel wider on each side, so the two
        // coverages add up instead. A stroke (`│`) does not span the cell and
        // stays where the advance puts it.
        if tiles(cell.ch) && gw >= cw - 1.0 {
            gx -= 0.5;
            gw += 1.0;
        }
        let gy = py + ascent - q.top as f32;
        // colour emoji gets no tinting: only the cell alpha
        let (cor, target) = if q.color {
            ([1.0, 1.0, 1.0, alpha], &mut self.emoji_row)
        } else {
            (colour, &mut self.instances)
        };
        target.push(Instance {
            rect: [gx, gy, gw, q.h as f32],
            color: cor,
            uv: q.uv,
        });
    }

    /// Fills what drawing needs from a cell's tail (`B66`): the cluster as text,
    /// and how many of its codepoints are plain combining marks. A joiner, a
    /// selector, a keycap, a skin tone or a joined member is **not** a mark —
    /// painting those over the base is what drew a square beside the hand.
    fn set_tail(&mut self, cell: Cell, tail: &[char]) {
        self.n_tail = tail.len();
        self.cur_cluster.clear();
        if tail.is_empty() {
            self.n_marks = 0;
            return;
        }
        self.cur_cluster.push(cell.ch);
        self.cur_cluster.extend(tail.iter());
        let mut n = 0;
        for ch in tail.iter().filter(|c| tortty_core::grid::is_mark(**c)) {
            if n == tortty_core::grid::MAX_MARKS {
                break;
            }
            self.cur_marks[n] = *ch;
            n += 1;
        }
        self.n_marks = n;
    }

    /// The glyph of a cell's **whole cluster** (`B66`), when a face composes it
    /// into one: a family of four is one picture, not four. `None` when no face
    /// has the ligature, and then the cell is drawn as it was — the base and its
    /// combining marks.
    fn cluster_quad(&mut self, bold: bool, italic: bool) -> Option<GlyphQuad> {
        if self.n_tail == 0 {
            return None;
        }
        let key = (
            u8::from(bold) | (u8::from(italic) << 1),
            self.cur_cluster.clone(),
        );
        if let Some(hit) = self.cluster_cache.get(&key) {
            return *hit;
        }
        let bitmap =
            self.fonts
                .cluster_bitmap(&mut self.scale_ctx, &self.cur_cluster, bold, italic);
        let quad = bitmap.and_then(|(b, color)| self.quad_of(&b, color));
        self.cluster_cache.insert(key, quad);
        quad
    }

    /// The combining marks of the cell being drawn (`M-U`), in `cur_marks`.
    fn paint_marks(&mut self, x: usize, py: f32, colour: [f32; 4], bold: bool, italic: bool) {
        let cw = self.fonts.cw;
        let ascent = self.fonts.ascent;
        for i in 0..self.n_marks {
            let Some(q) = self.glyph_quad(self.cur_marks[i], bold, italic) else {
                continue;
            };
            self.instances.push(Instance {
                rect: [
                    x as f32 * cw + q.left as f32,
                    py + ascent - q.top as f32,
                    q.w as f32,
                    q.h as f32,
                ],
                color: colour,
                uv: q.uv,
            });
        }
    }

    /// `U=1` (`M-L`): a cell holding `U+10EEEE` is one pixel of an image whose
    /// prototype placement says how many cells it spans. The cell's foreground
    /// colour carries the image id, its underline colour the placement id, and the
    /// diacritics the row and column — with the spec's inheritance from the cell
    /// to the left, which is why this walks each row in order.
    fn placeholder_cells(
        &mut self,
        f: &Frame<'_>,
        cell: (f32, f32),
        quads: &mut Vec<(u32, Instance)>,
    ) {
        use tortty_core::placeholder::{decode, uv, Cell as Where, PLACEHOLDER};
        let (cw, chh) = cell;
        let view = f.view_off;
        for (row_index, row) in f.term.visible_rows(view).iter().enumerate() {
            // the inheritance is only allowed from a neighbour with the same
            // colours, so the colours are part of what is carried
            let mut prev: Option<(Where, u32, u32)> = None;
            for (col, c) in row.cells.iter().enumerate() {
                if c.ch != PLACEHOLDER {
                    prev = None;
                    continue;
                }
                let id = ink_id(c.fg);
                let pid = ink_id(c.ul_color);
                let inherited = prev
                    .filter(|(_, fg, ul)| *fg == id && *ul == pid)
                    .map(|(cell, _, _)| cell);
                let where_ = decode(row.marks_at(col), inherited);
                prev = Some((where_, id, pid));
                let Some(vpl) = f.term.graphics.placeholder(where_.image(id), pid) else {
                    continue;
                };
                let Some(img) = f.term.graphics.images().get(&vpl.image_id) else {
                    continue;
                };
                if !self.ensure_texture(img) {
                    continue;
                }
                #[allow(clippy::cast_precision_loss)]
                let rect = [col as f32 * cw, row_index as f32 * chh, cw, chh];
                quads.push((
                    vpl.image_id,
                    Instance {
                        rect,
                        color: [1.0, 1.0, 1.0, 1.0],
                        uv: uv(where_, vpl.cols, vpl.rows),
                    },
                ));
            }
        }
    }

    /// Uploads the image texture if it is not in the cache yet (or if the app
    /// resent it with the same id and the version changed).
    fn ensure_texture(&mut self, img: &tortty_core::graphics::Image) -> bool {
        if self
            .img_cache
            .get(&img.id)
            .is_some_and(|t| t.version == img.version && t.frame == img.current())
        {
            return true;
        }
        let (br, bgc, bb) = self.bg;
        let Some((w, h, rgba)) = compose(img, [br, bgc, bb]) else {
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
                frame: img.current(),
                bind,
            },
        );
        true
    }

    /// Cursor (block, underline or beam) — on top of the text and the images.
    ///
    /// One cursor, two owners: whoever owns it (the program, or copy-mode
    /// while the mode lasts) resolved the cell into `chrome::Cursor`, and this
    /// only turns it into ink. Which owner it is does not reach here.
    fn build_cursor(&mut self, frame: &Frame<'_>) {
        let Some(cursor) = frame.cursor else { return };
        if !frame.blink.cursor {
            return;
        }
        let ink = self.color(Color::Default, self.cursor);
        // a line or a beam is thinner ink and reads better a shade stronger
        let alpha = match cursor.shape {
            crate::chrome::CursorShape::Block => 0.85,
            crate::chrome::CursorShape::Underline | crate::chrome::CursorShape::Beam => 0.9,
        };
        let rect = cursor.rect;
        self.push_rect(
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            [ink[0], ink[1], ink[2], alpha],
        );
    }

    /// The scrollbar: an 8 px strip on the grid's right edge, drawn only when
    /// there is history to scroll. The thumb is the share of the buffer the view
    /// shows and it sits at the bottom while the view follows the output
    /// (`view_off == 0`). No track frame and a translucent thumb: the strip is
    /// **over** the last column, and a solid one would eat the text under it.
    /// Width lives in `SCROLLBAR_W` because the app hit-tests the same strip.
    fn scrollbar(&mut self, f: &Frame<'_>) {
        // the strip and the thumb are `chrome::Scrollbar`'s numbers, the same ones
        // the press that drags it reads (`M-Q`)
        let Some(sb) =
            crate::chrome::Scrollbar::new(f.term.scrollback.len(), f.term.rows, f.view_off)
        else {
            return;
        };
        let (track, thumb) = sb.parts(f.metrics);
        let ink = self.color(Color::Default, self.fg);
        self.push_rect(
            track[0],
            track[1],
            track[2],
            track[3],
            [ink[0], ink[1], ink[2], 0.10],
        );
        self.push_rect(
            thumb[0],
            thumb[1],
            thumb[2],
            thumb[3],
            [ink[0], ink[1], ink[2], 0.55],
        );
    }

    pub fn build(&mut self, f: &Frame<'_>) {
        let Frame {
            tab,
            term,
            view_off,
            sel,
            // read whole in `build_cursor`, which needs nothing else
            cursor: _,
            blink,
            toast,
            tabs,
            sidebar,
            cards_off,
            links,
            search,
            menu,
            metrics,
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
        // `M-D`: the matches are a background override on the grid cells, so the
        // mask has to exist before the rows are painted (see `paint_line`)
        self.build_marks(search, &rows, cols, ctx);
        // everything after this point belongs to the chrome, which is drawn on
        // top: the colour glyphs of the chrome are the second half of the list
        self.emoji_split = self.emoji_instances.len();

        // here the grid text ends: the images (H1–H13) come on top of it and
        // below the chrome/cursor, which is kitty's order
        self.img_split = self.instances.len();
        self.build_images(f);

        // 2b) the chrome of this frame, laid out once (`M-Q`)
        self.build_chrome(tabs, sidebar, cards_off, metrics);

        // 2d) scrollbar: over the grid's right edge, under the cursor/toast
        self.scrollbar(f);

        // 3) cursor (on top; instance order is blend order)
        self.build_cursor(f);

        // 3b) the grid's bottom row, shared by two overlays (`M-D`, `B47`)
        self.bottom_row(search, metrics);

        // 4) history indicator (D8): "↑N" — without it, scrolling back is
        // indistinguishable from the app having stopped writing
        if view_off > 0 {
            let txt = format!("↑{view_off}");
            self.badge(&txt, cols);
        }

        // 5) toast (I12/K13): notice from the terminal itself, over the chrome and
        // the grid and under the menu. It goes away on its own, or with a click;
        // the app controls the timing and derives the same box from the same
        // notice, so a click never lands on a notice that is gone.
        if let Some(t) = toast {
            self.toast(t, metrics);
        }

        // 6) the grid's context menu (`M-V`): it is opened *by* a click on what
        // it covers, so it is drawn last
        if let Some(mv) = menu {
            self.menu(mv);
        }
    }

    /// The search mask plus, when there is one, the rows rebuilt with it: the
    /// per-row cache was painted without the highlight (`M-D`).
    fn build_marks(
        &mut self,
        search: Option<&SearchView>,
        rows: &[&tortty_core::grid::Row],
        cols: usize,
        ctx: RowCtx,
    ) {
        self.marks = marks(search, cols, rows.len());
        self.marks_cols = cols;
        if self.marks.iter().any(|m| *m != 0) {
            self.per_line = vec![RowQuads::default(); rows.len()];
            for (y, row) in rows.iter().enumerate() {
                self.per_line[y] = self.line(row, y, ctx);
            }
            self.concat_lines();
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
        // `B69`: the plain-text links are a property of the window, not of a row
        if ctx.links {
            self.scan_links(rows);
        }
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
        self.link_mask.clear();
        if let Some(spans) = self.row_links.get(y) {
            self.link_mask.extend_from_slice(spans);
        }
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
                self.cur_mark = self.mark_at(y, cx);
                let swap = self.cur_mark == 0 && is_swap(&c, sel, y, cx);
                // the tail of this cell, for `draw_cell` (fields instead of a
                // seventh parameter, the way `cur_mark` already works): the whole
                // cluster for shaping (`B66`), and — of it — only the plain
                // combining marks to paint one over the other (`M-U`)
                self.set_tail(c, row.tail_at(cx));
                self.draw_cell(&c, cx, py, swap, glyph, links);
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
            // `M-D`: a match overrides the cell's own background, and the current
            // one overrides it with the bright shade
            let mark = self.mark_at(y, x);
            let swap = mark == 0 && is_swap(&cell, sel, y, x);
            let bg = if swap { cell.fg } else { cell.bg };
            let mut end = x + 1;
            while end < cols {
                let Some(c2) = row.cells.get(end) else { break };
                if self.mark_at(y, end) != mark {
                    break;
                }
                if mark == 0 && is_swap(c2, sel, y, end) != swap {
                    break;
                }
                let bg2 = if swap { c2.fg } else { c2.bg };
                if mark == 0 && bg2 != bg {
                    break;
                }
                end += 1;
            }
            let hit = match mark {
                2 => Some(CUR_BG),
                1 => Some(HIT_BG),
                _ => None,
            };
            if let Some(h) = hit {
                // 0..=255 to 0.0..=1.0: the same conversion `Renderer::color` does
                let s = |v: u8| f32::from(v) / 255.0;
                self.push_rect(
                    x as f32 * cw,
                    py,
                    (end - x) as f32 * cw,
                    chh,
                    [s(h.0), s(h.1), s(h.2), 1.0],
                );
            } else if swap || bg != Color::Default {
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
        self.rows_built = self.rows_built.saturating_add(1);
        self.cells_built = self.cells_built.saturating_add(cols as u64);
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
                 GPU usually looks like this: check /dev/dri, and bind the GPU device into a \
                 container; see docs/DEBUG.md"
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
    /// One image span of `img_buf`: one draw call per image, since each has its
    /// own texture.
    fn img_draw(&self, rp: &mut wgpu::RenderPass<'_>, span: std::ops::Range<usize>) {
        rp.set_pipeline(&self.img_pipeline);
        rp.set_vertex_buffer(0, self.img_buf.slice(..));
        for i in span {
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

    fn paint(&self, rp: &mut wgpu::RenderPass<'_>) {
        rp.set_pipeline(&self.pipeline);
        rp.set_bind_group(0, &self.globals_bind, &[]);
        rp.set_bind_group(1, &self.atlas_bind, &[]);
        rp.set_vertex_buffer(0, self.inst_buf.slice(..));
        // grid text → images → chrome and cursor. Two spans of the same buffer
        // because the image has its own pipeline (samples RGB).
        let split = self.img_split.min(self.instances.len()) as u32;
        // `M-L`: the placements with `z < 0` go **under** the text, so their span
        // of the image buffer is drawn before the grid's first span
        let below = self.img_below.min(self.img_instances.len());
        if below > 0 {
            self.img_draw(rp, 0..below);
        }
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
        if self.img_instances.len() > below {
            self.img_draw(rp, below..self.img_instances.len());
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

/// Side of the toast's corner tile, in px (`B57`): the radius and the sample
/// size are the same, so the tile is drawn 1:1 and its antialiasing is not
/// rescaled.
const CORNER: u32 = 8;

/// Alpha of a quarter disc filling the top-left of a `CORNER` square, sampled
/// 4×4 per pixel — the corner of every toast box, mirrored to the other three.
fn corner_mask(side: u32) -> Vec<u8> {
    #[allow(clippy::cast_precision_loss)] // 8 px, far below 2^23
    let radius = side as f32;
    let mut out = Vec::with_capacity((side * side) as usize);
    for py in 0..side {
        for px in 0..side {
            let mut inside = 0.0_f32;
            for sy in 0..4 {
                for sx in 0..4 {
                    #[allow(clippy::cast_precision_loss)]
                    let x = px as f32 + (sx as f32 + 0.5) / 4.0;
                    #[allow(clippy::cast_precision_loss)]
                    let y = py as f32 + (sy as f32 + 0.5) / 4.0;
                    let inside_corner = if x <= radius && y <= radius {
                        let (dx, dy) = (x - radius, y - radius);
                        dx * dx + dy * dy <= radius * radius
                    } else {
                        true
                    };
                    if inside_corner {
                        inside += 1.0;
                    }
                }
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            out.push((inside / 16.0 * 255.0).round() as u8);
        }
    }
    out
}

/// A cell colour read as a number, which is how a placeholder cell names its
/// image (the foreground) and its placement (the underline): true colour carries
/// 24 bits, the palette one byte, and the default colour names nothing.
fn ink_id(colour: tortty_core::color::Color) -> u32 {
    use tortty_core::color::Color;
    match colour {
        Color::Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        Color::Idx(i) => u32::from(i),
        Color::Default => 0,
    }
}

/// The picture to upload for an image: its current animation frame (`B2`),
/// composed from what the client sent, as (width, height, RGBA8).
///
/// The core keeps every transmission as a recipe — rectangle, base frame, blend
/// mode, background — because composing is pixel work and the PNG decoder lives
/// here. Frame 1 is the image's own data; the rest start from the frame their
/// `c=` names (or from their background colour) and have their rectangles
/// painted over it.
fn compose(img: &tortty_core::graphics::Image, background: [u8; 3]) -> Option<(u32, u32, Vec<u8>)> {
    let (width, height) = (img.width, img.height);
    if width == 0 || height == 0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let mut canvas = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut seen = vec![false; img.frames.len().max(1)];
    if paint(img, img.current(), &mut canvas, &mut seen, background) {
        Some((width, height, canvas))
    } else {
        None
    }
}

fn paint(
    img: &tortty_core::graphics::Image,
    k: u32,
    canvas: &mut [u8],
    seen: &mut [bool],
    background: [u8; 3],
) -> bool {
    let index = usize::try_from(k.saturating_sub(1)).unwrap_or(0);
    if index >= seen.len() || std::mem::replace(&mut seen[index], true) {
        return false; // no such frame, or a `c=` chain that bites its own tail
    }
    let full = (img.width, img.height);
    if index == 0 {
        let Some((w, h, px)) = decode_pixels(img.format, &img.data, full.0, full.1, background)
        else {
            return false;
        };
        blit(canvas, full, (0, 0), true, (w, h), &px);
        return true;
    }
    let frame = &img.frames[index];
    let Some(first) = frame.steps.first() else {
        return false;
    };
    if let Some(base) = first.base {
        if !paint(img, base, canvas, seen, background) {
            return false;
        }
    } else {
        // a canvas nobody provides starts as one colour: black and transparent,
        // or whatever `Y=` asked for
        let rgba = first.background.to_be_bytes();
        for px in canvas.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&rgba);
        }
    }
    for step in &frame.steps {
        let Some((w, h, px)) =
            decode_pixels(step.format, &step.data, step.width, step.height, background)
        else {
            return false;
        };
        blit(canvas, full, step.at, step.replace, (w, h), &px);
    }
    true
}

/// One rectangle's bytes → RGBA8. `f=100` carries its own size in the PNG header,
/// so `width`/`height` are only a hint for it.
fn decode_pixels(
    format: tortty_core::graphics::Format,
    data: &[u8],
    width: u32,
    height: u32,
    background: [u8; 3],
) -> Option<(u32, u32, Vec<u8>)> {
    use tortty_core::graphics::Format;
    if format == Format::Png {
        return decode_png(data);
    }
    if width == 0 || height == 0 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    match format {
        Format::Rgba => {
            let need = w * h * 4;
            (data.len() >= need).then(|| (width, height, data[..need].to_vec()))
        },
        Format::Rgb => {
            let need = w * h * 3;
            if data.len() < need {
                return None;
            }
            let mut out = Vec::with_capacity(w * h * 4);
            for px in data[..need].as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Some((width, height, out))
        },
        Format::Png => decode_png(data),
        // `P2=0` draws the terminal's own background behind the picture;
        // `P2=1` leaves it transparent, so the text behind shows through
        Format::Sixel => sixel(data, Some(background), width, height),
        Format::SixelAlpha => sixel(data, None, width, height),
    }
}

/// A sixel stream → RGBA8 (`B3`). The picture's own raster is what counts; the
/// `width`/`height` the core measured are the same numbers, so a stream whose
/// walk disagrees with them is not drawn at all.
fn sixel(
    data: &[u8],
    under: Option<[u8; 3]>,
    width: u32,
    height: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let raster = tortty_core::sixel::decode(data, under, (width, height), u32::MAX)?;
    if (raster.width, raster.height) != (width, height) {
        return None;
    }
    Some((raster.width, raster.height, raster.rgba))
}

/// PNG → RGBA8 (the app owns the decoder; the core does not carry one).
fn decode_png(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(data))
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
}

/// Paints one rectangle into the canvas at `at`, clipped to the image. `replace`
/// overwrites; otherwise it is source-over, in the same 8-bit space as the rest.
fn blit(
    canvas: &mut [u8],
    image: (u32, u32),
    at: (u32, u32),
    replace: bool,
    part: (u32, u32),
    px: &[u8],
) {
    let (iw, ih) = (image.0 as usize, image.1 as usize);
    let (pw, ph) = (part.0 as usize, part.1 as usize);
    let (x0, y0) = (at.0 as usize, at.1 as usize);
    for row in 0..ph {
        let y = y0 + row;
        if y >= ih {
            break;
        }
        for col in 0..pw {
            let x = x0 + col;
            if x >= iw {
                break;
            }
            let Some(src) = px.get((row * pw + col) * 4..(row * pw + col) * 4 + 4) else {
                return;
            };
            let dst = (y * iw + x) * 4;
            if replace || src[3] == 255 {
                canvas[dst..dst + 4].copy_from_slice(src);
                continue;
            }
            let sa = u32::from(src[3]);
            for c in 0..3 {
                let s = u32::from(src[c]);
                let d = u32::from(canvas[dst + c]);
                canvas[dst + c] = u8::try_from((s * sa + d * (255 - sa)) / 255).unwrap_or(255);
            }
            let da = u32::from(canvas[dst + 3]);
            canvas[dst + 3] = u8::try_from(sa + da * (255 - sa) / 255).unwrap_or(255);
        }
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

/// The search highlight (`M-D`): the match background and the current one's. The
/// bright amber is the current hit, so `Enter` is visible without reading the bar.
const HIT_BG: (u8, u8, u8) = (0x4a, 0x3d, 0x00);
const CUR_BG: (u8, u8, u8) = (0xd7, 0xa5, 0x00);

/// `row * cols + col` → 0 (none), 1 (a match) or 2 (the current match).
fn marks(search: Option<&SearchView>, cols: usize, rows: usize) -> Vec<u8> {
    let mut m = vec![0u8; cols * rows];
    let Some(sv) = search else {
        return m;
    };
    for &(row, col, len, current) in &sv.hits {
        if row >= rows {
            continue;
        }
        let kind = if current { 2 } else { 1 };
        for c in col..(col + len).min(cols) {
            m[row * cols + c] = kind;
        }
    }
    m
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
    /// bottom edge of the tab, under `progress_show = bar`.
    pub progress: Option<Progress>,
    /// The pointer is standing on this tab: the one shade the chrome reacts to
    /// without a press.
    pub hover: bool,
}

/// Height of that bar. Two pixels on the bottom edge: thin enough not to eat the
/// label, and the width is in pixels (37% of a 20-cell tab is 7.4 cells of ink).
const PROGRESS_H: f32 = 2.0;

/// The inactive tab and card wear their own ink now, not the palette's slot 8:
/// the chrome does not follow the terminal palette, so a theme or
/// `bold_as_bright` changing `color8` must not move the labels with it.
const INK_INACTIVE: (u8, u8, u8) = (0x7f, 0x7f, 0x7f);

/// The selected tab and card wear their own background now (`#0f3d22`, picked
/// by the owner in the colour round), not the theme inversion: the label is
/// pure white on it, whatever the palette says.
const SEL_BG: (u8, u8, u8) = (0x0f, 0x3d, 0x22);
const SEL_FG: (u8, u8, u8) = (0xff, 0xff, 0xff);

/// A fixed chrome tone for the shader (the chrome does not follow the
/// terminal palette, and `Color::Default` would resolve to the theme).
fn chrome_rgb(c: (u8, u8, u8)) -> [f32; 3] {
    [
        f32::from(c.0) / 255.0,
        f32::from(c.1) / 255.0,
        f32::from(c.2) / 255.0,
    ]
}

/// The chrome's one step: `base` pulled `t` of the way towards `ink`. The
/// toast's surface and the menu's use the same formula inline; the buttons
/// build on it twice (a step up from the bar, another for the hover), so the
/// arithmetic has one home. `t` in `0.0..=1.0`.
fn mix(base: (u8, u8, u8), ink: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    // a channel, rounded; the result is between both inputs
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn step(b: u8, i: u8, t: f32) -> u8 {
        (f32::from(b) + (f32::from(i) - f32::from(b)) * t).round() as u8
    }
    (
        step(base.0, ink.0, t),
        step(base.1, ink.1, t),
        step(base.2, ink.2, t),
    )
}

/// Colour and fill of a progress report: one light tone per state, all opaque
/// (picks of the colour round) — the bar says how far, the bell and the card
/// tint say whether it wants you. A state with no measure fills the tab.
fn progress_ink(p: Progress) -> (Color, f32, f32) {
    let frac = p.pct.map_or(1.0, |pct| f32::from(pct) / 100.0);
    match p.state {
        2 => (Color::Rgb(0xf8, 0x71, 0x71), 1.0, frac),
        4 => (Color::Rgb(0xfc, 0xd3, 0x4d), 1.0, frac),
        3 => (Color::Rgb(0x22, 0xd3, 0xee), 1.0, frac),
        _ => (Color::Rgb(0x4a, 0xde, 0x80), 1.0, frac),
    }
}

/// What a notice is telling the owner: the accent (bar and border) and, on the
/// app's side, how long it stays. Four shades, not a spectrum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// A receipt: something worked.
    Ok,
    /// Neither good nor bad: the app saying what it just did (a mode entered, a
    /// view opened). It is not a receipt — nothing was accomplished — and it is
    /// not a warning either.
    Info,
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
            // blue carries no judgement: 2, 3 and 1 already mean good,
            // attention and bad, and this shade must not read as any of them
            Severity::Info => Color::Idx(4),
            Severity::Warn => Color::Idx(3),
            Severity::Err => Color::Idx(1),
        }
    }

    /// The same shade in the event log: a warning on screen is a warning in the
    /// file, and a receipt is not one.
    pub fn level(self) -> crate::log::Level {
        match self {
            Severity::Ok | Severity::Info => crate::log::Level::Info,
            Severity::Warn => crate::log::Level::Warn,
            Severity::Err => crate::log::Level::Err,
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

/// Everything a frame needs, in one package: the signature of `build`/`render`
/// grew one parameter per slice (tabs, sidebar, toast...).
pub struct Frame<'a> {
    /// Identity of the drawn tab (see `damage`).
    pub tab: u64,
    pub term: &'a Term,
    /// Where the chrome lands, built by the app — the only place that knows the
    /// bar and the sidebar height of this frame (`M-Q`).
    pub metrics: crate::chrome::Metrics,
    pub view_off: usize,
    pub sel: Option<Span>,
    /// The cursor, already resolved to a cell and a shape by whoever owns it:
    /// the program, or copy-mode while the mode lasts (`B76`). `None` = not
    /// shown, and the painter draws nothing.
    pub cursor: Option<crate::chrome::Cursor>,
    pub blink: Blink,
    pub toast: Option<&'a ToastText>,
    pub tabs: Option<&'a [TabInfo]>,
    pub sidebar: Option<&'a [WsCard]>,
    /// First card of the sidebar list to draw (`B42`).
    pub cards_off: usize,
    /// Ctrl is held: the `OSC 8` links underline themselves.
    pub links: bool,
    /// The scrollback search (`M-D`): the matches in **view rows**, plus the bar.
    pub search: Option<&'a SearchView>,
    /// The grid's context menu (`M-V`), when it is open.
    pub menu: Option<&'a MenuView>,
}

/// The context menu as the renderer needs it: its geometry, which rows are
/// usable and the one under the pointer.
pub struct MenuView {
    pub layout: crate::chrome::Menu,
    pub enabled: Vec<bool>,
    pub hover: Option<usize>,
}

/// What the renderer needs from the search: the visible matches and the query.
/// `current` is the one `Enter` last landed on.
pub struct SearchView {
    /// `(row in the view, column, length, current)`.
    pub hits: Vec<(usize, usize, usize, bool)>,
    /// What is typed, without the label or the cursor.
    pub query: String,
    /// `1/17` or `no match`; `None` while the query is empty, and then the count
    /// chip is not drawn at all.
    pub count: Option<String>,
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
    /// The pointer is standing on this card: the one shade the chrome reacts to
    /// without a press.
    pub hover: bool,
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
    use super::{card_lines, is_swap, progress_ink};
    use tortty_core::cell::Cell;
    use tortty_core::select::Rect;
    use tortty_core::term::Progress;
    use tortty_core::Color;

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

    /// The chrome's one step, twice: the inactive button's body is the bar's
    /// black pulled 14% towards the label ink, and the hover is that body
    /// pulled 14% more. Computed, not written: a change in either input moves
    /// both steps with it.
    #[test]
    fn the_button_body_is_two_steps_of_the_same_mix() {
        use super::{mix, INK_INACTIVE};
        let bar = (0, 0, 0); // `color0` as the defaults have it
        let body = mix(bar, INK_INACTIVE, 0.14);
        // one step: 14% of the way on every channel (ink is 0x7f over black)
        assert_eq!(body, (18, 18, 18));
        // two steps: the body pulled 14% again — up, and never past the ink
        let hovered = mix(body, INK_INACTIVE, 0.14);
        assert_eq!(hovered, (33, 33, 33));
        assert!(hovered.0 > body.0 && hovered.0 < INK_INACTIVE.0);
        // the ends hold: no pull is the base, the whole pull is the ink
        assert_eq!(mix(bar, INK_INACTIVE, 0.0), bar);
        assert_eq!(mix(bar, INK_INACTIVE, 1.0), INK_INACTIVE);
    }

    /// `OSC 9;4`: one light tone per state, all opaque, filled by the fraction.
    #[test]
    fn progress_ink_follows_the_state() {
        // the colour round's picks: light green, light red, light yellow
        assert_eq!(
            progress_ink(Progress {
                state: 1,
                pct: Some(37)
            }),
            (Color::Rgb(0x4a, 0xde, 0x80), 1.0, 37.0 / 100.0)
        );
        assert_eq!(
            progress_ink(Progress {
                state: 2,
                pct: Some(50)
            })
            .0,
            Color::Rgb(0xf8, 0x71, 0x71)
        );
        assert_eq!(
            progress_ink(Progress {
                state: 4,
                pct: None
            }),
            (Color::Rgb(0xfc, 0xd3, 0x4d), 1.0, 1.0)
        );
        // indeterminate is opaque too and fills the tab: it does not move,
        // because moving costs a clock and a frame per step
        assert_eq!(
            progress_ink(Progress {
                state: 3,
                pct: None
            }),
            (Color::Rgb(0x22, 0xd3, 0xee), 1.0, 1.0)
        );
        // a state with no fraction still gets a band
        assert_eq!(
            progress_ink(Progress {
                state: 1,
                pct: None
            }),
            (Color::Rgb(0x4a, 0xde, 0x80), 1.0, 1.0)
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

    /// The info shade is blue, and it is blue *because* it judges nothing: 2, 3
    /// and 1 already mean good, attention and bad, so a mode announcing itself
    /// must not borrow any of them. In the log it is an info line, like a receipt.
    #[test]
    fn the_info_shade_is_blue() {
        use super::Severity;
        assert_eq!(Severity::Info.color(), Color::Idx(4));
        assert_eq!(Severity::Info.level(), crate::log::Level::Info);
        for (shade, slot) in [(Severity::Ok, 2), (Severity::Warn, 3), (Severity::Err, 1)] {
            assert_ne!(
                Severity::Info.color(),
                shade.color(),
                "slot {slot} is taken"
            );
        }
    }
}
