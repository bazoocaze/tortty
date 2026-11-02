//! Atlas de glifos: textura R8 com alocação por prateleira.
//!
//! Um glifo é rasterizado uma vez e reusado em todo frame; o frame monta
//! somente vértices. O pixel branco no canto serve de "textura" para os quad
//! sólidos (fundo, sublinhado, cursor), o que permite um pipeline só.

use wgpu::util::DeviceExt;

pub const ATLAS_SIZE: u32 = 2048;
/// Borda em volta do bloco branco, longe dos glifos.
const WHITE: u32 = 4;

pub struct Atlas {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    size: u32,
    data: Vec<u8>,
    x: u32,
    y: u32,
    row_h: u32,
    pending: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    pub uv_white: [f32; 4],
}

impl Atlas {
    pub fn new(device: &wgpu::Device) -> Atlas {
        let size = ATLAS_SIZE;
        let mut data = vec![0u8; (size * size) as usize];
        // bloco branco (WHITE x WHITE) no canto
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
            size,
            data,
            x: WHITE,
            y: 0,
            row_h: WHITE,
            pending: Vec::new(),
            uv_white: [0.0; 4],
        };
        // centro do bloco branco: evita amostrar a borda
        let c = (WHITE as f32 / 2.0) / size as f32;
        atlas.uv_white = [c, c, c, c];
        // o bloco branco precisa ir para a GPU no primeiro flush
        atlas
            .pending
            .push((0, 0, WHITE, WHITE, vec![255u8; (WHITE * WHITE) as usize]));
        atlas
    }

    /// Reserva espaço e devolve o retângulo UV (u0, v0, u1, v1).
    // w/h/x/y/u/v são as coordenadas de um retângulo no atlas: renomear só
    // afasta o código do desenho
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
            return None; // atlas cheio: o glifo cai fora deste frame
        }
        let (x, y) = (self.x, self.y);
        for row in 0..h {
            let src = (row * w) as usize;
            let dst = ((y + row) * self.size + x) as usize;
            self.data[dst..dst + w as usize].copy_from_slice(&data[src..src + w as usize]);
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

    /// Sobe os glifos novos para a GPU (só o que mudou, não o atlas inteiro).
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
                    bytes_per_row: Some(w),
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

/// O `bind_group` do atlas (textura + sampler nearest).
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

/// Buffer de globals (tamanho da tela em pixels).
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

/// Layout de bind group das imagens do protocolo de gráficos: mesmo formato do
/// atlas, mas com amostragem **filtrável** e textura filtrável — imagem é
/// escalada (o `icat` manda 48x48 e a célula ocupa mais que isso), e com
/// `Nearest` o resultado fica serrilhado. O atlas não pode usar o mesmo layout
/// porque a máscara de cobertura de glifo é `filterable: false`.
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
