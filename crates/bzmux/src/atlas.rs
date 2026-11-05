//! Glyph atlas: R8 texture with shelf allocation.
//!
//! A glyph is rasterized once and reused across every frame; the frame builds
//! only vertices. The white pixel in the corner serves as the "texture" for the
//! solid quads (background, underline, cursor), which allows a single pipeline.

use wgpu::util::DeviceExt;

pub const ATLAS_SIZE: u32 = 2048;
/// Border around the white block, away from the glyphs.
const WHITE: u32 = 4;

pub struct Atlas {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Bytes per pixel: 1 = mask (glyph), 4 = RGBA (colored emoji).
    bpp: u32,
    size: u32,
    data: Vec<u8>,
    x: u32,
    y: u32,
    row_h: u32,
    pending: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    pub uv_white: [f32; 4],
}

impl Atlas {
    /// RGBA atlas for colored glyphs (emoji): the same shelf layout, with 4
    /// bytes per pixel.
    pub fn new_rgba(device: &wgpu::Device) -> Atlas {
        let mut atlas = Atlas::new(device);
        let size = atlas.size;
        atlas.bpp = 4;
        atlas.data = vec![0u8; (size * size * 4) as usize];
        for y in 0..WHITE {
            for x in 0..WHITE {
                let i = ((y * size + x) * 4) as usize;
                atlas.data[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        // `new` left the white block pending at 1 byte/pixel; here it is RGBA,
        // otherwise the flush `write_texture` does not match the `bpp`.
        atlas.pending = vec![(
            0,
            0,
            WHITE,
            WHITE,
            vec![255u8; (WHITE * WHITE * 4) as usize],
        )];
        atlas.tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas-rgba"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        atlas.view = atlas
            .tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        atlas.uv_white = [
            0.0,
            0.0,
            WHITE as f32 / size as f32,
            WHITE as f32 / size as f32,
        ];
        atlas
    }

    pub fn new(device: &wgpu::Device) -> Atlas {
        let size = ATLAS_SIZE;
        let mut data = vec![0u8; (size * size) as usize];
        // white block (WHITE x WHITE) in the corner
        for y in 0..WHITE {
            for x in 0..WHITE {
                data[(y * size + x) as usize] = 255;
            }
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut atlas = Atlas {
            tex,
            view,
            bpp: 1,
            size,
            data,
            x: WHITE,
            y: 0,
            row_h: WHITE,
            pending: Vec::new(),
            uv_white: [0.0; 4],
        };
        // center of the white block: avoids sampling the border
        let c = (WHITE as f32 / 2.0) / size as f32;
        atlas.uv_white = [c, c, c, c];
        // the white block has to reach the GPU on the first flush
        atlas
            .pending
            .push((0, 0, WHITE, WHITE, vec![255u8; (WHITE * WHITE) as usize]));
        atlas
    }

    /// Reserves space and returns the UV rectangle (u0, v0, u1, v1).
    // w/h/x/y/u/v are the coordinates of a rectangle in the atlas: renaming
    // them only moves the code further from the drawing
    #[allow(clippy::many_single_char_names)]
    pub fn add(&mut self, w: u32, h: u32, data: &[u8]) -> Option<[f32; 4]> {
        if w == 0 || h == 0 || w + 2 > self.size || h + 2 > self.size {
            return None;
        }
        if self.x + w + 1 > self.size {
            self.x = 0;
            self.y += self.row_h + 1;
            self.row_h = 0;
        }
        if self.y + h + 1 > self.size {
            return None; // atlas full: the glyph is dropped from this frame
        }
        let (x, y) = (self.x, self.y);
        let bpp = self.bpp as usize;
        for row in 0..h {
            let src = (row * w) as usize * bpp;
            let dst = ((y + row) * self.size + x) as usize * bpp;
            self.data[dst..dst + w as usize * bpp]
                .copy_from_slice(&data[src..src + w as usize * bpp]);
        }
        self.x += w + 1;
        self.row_h = self.row_h.max(h);
        self.pending.push((x, y, w, h, data.to_vec()));
        let s = self.size as f32;
        Some([
            x as f32 / s,
            y as f32 / s,
            (x + w) as f32 / s,
            (y + h) as f32 / s,
        ])
    }

    /// Uploads the new glyphs to the GPU (only what changed, not the whole
    /// atlas).
    pub fn flush(&mut self, queue: &wgpu::Queue) {
        for (x, y, w, h, data) in self.pending.drain(..) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * self.bpp),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

/// The atlas `bind_group` (texture + nearest sampler).
pub fn bind_atlas(
    device: &wgpu::Device,
    atlas: &Atlas,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("atlas-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("atlas-sampler"),
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atlas-bind"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas.view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (layout, bind)
}

/// Globals buffer (screen size in pixels).
pub fn globals_buffer(device: &wgpu::Device, w: f32, h: f32, pad: f32) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("globals"),
        contents: bytemuck::cast_slice(&[w, h, pad, pad]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

pub fn globals_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Bind group layout for the images of the graphics protocol: same format as
/// the atlas, but with **filterable** sampling and a filterable texture — an
/// image is scaled (the `icat` sends 48x48 and the cell is bigger than that),
/// and with `Nearest` the result comes out jagged. The atlas cannot use the
/// same layout because the glyph coverage mask is `filterable: false`.
pub fn bind_image(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::Sampler) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("img-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("img-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    });
    (layout, sampler)
}
