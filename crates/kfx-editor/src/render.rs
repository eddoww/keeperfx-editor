//! GPU side of the 3D viewport: terrain chunks, texture atlas, overlay lines, offscreen target.
use crate::atlas::{AtlasData, LAYER_H, LAYER_W, MIP_LEVELS};
use crate::camera::Camera;
use crate::mesh::{ChunkMesh, Vertex, BLOCKS_PER_VARIATION, VARIATIONS};
use crate::sprites::{SpriteAtlas, ATLAS_SIZE};
use egui_wgpu::wgpu;
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Default MSAA sample count (Preferences > Graphics can turn it off: 1).
pub const SAMPLES: u32 = 4;
pub const CLEAR: [f64; 3] = [0.035, 0.03, 0.028];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye_dir: [f32; 4],
    params: [u32; 4],
    /// x: exposure of the tone curve (1 = none), y: local light strength, zw unused.
    tone: [f32; 4],
    /// Local lights: xyz position, w range (world units); `light_count` used.
    lights: [[f32; 4]; MAX_LIGHTS],
    /// Local light intensity (0..1) per light, four per vector.
    light_power: [[f32; 4]; MAX_LIGHTS / 4],
}

/// Local lights the terrain shader takes.
pub const MAX_LIGHTS: usize = 16;

/// A point light near the camera (from the map's lights) for close-up views.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalLight {
    pub pos: Vec3,
    pub range: f32,
    /// 0..1
    pub power: f32,
}

/// Tone of the terrain: exposure (1 = the game's colours) and local lights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tone {
    pub exposure: f32,
    pub lights: Vec<LocalLight>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteGlobals {
    view_proj: [[f32; 4]; 4],
    right: [f32; 4],
    up: [f32; 4],
    eye: [f32; 4],
}

/// One billboard (see `sprite.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteInstance {
    pub pos: [f32; 3],
    /// World units per sprite pixel.
    pub scale: f32,
    /// Bitmap rectangle relative to the anchor, sprite pixels (x, y, w, h; y down).
    pub rect: [f32; 4],
    /// Atlas texel rectangle (x, y, w, h).
    pub uv: [f32; 4],
    pub tint: [f32; 4],
    /// Bit 0 mirror, bit 1 drawn over everything, bit 2 translucent.
    pub flags: u32,
}

/// Depth bias of billboards towards the eye, world units.
const SPRITE_DEPTH_BIAS: f32 = 0.75;

/// Overlay line vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [u8; 4],
}

struct GpuChunk {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
}

struct Target {
    size: (u32, u32),
    /// `None` without multisampling (the resolve texture is drawn into directly).
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    _resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
}

pub struct ViewRenderer {
    terrain_pipeline: wgpu::RenderPipeline,
    line_pipeline_top: wgpu::RenderPipeline,
    line_pipeline_depth: wgpu::RenderPipeline,
    tri_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    sprite_blend_pipeline: wgpu::RenderPipeline,
    sprite_globals: wgpu::Buffer,
    sprite_tex: wgpu::Texture,
    sprite_bind_group: wgpu::BindGroup,
    pub flags: ViewFlags,
    bgl: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    chunks: Vec<Option<GpuChunk>>,
    /// Render targets: slot 0 the main view, slot 1 the second pane of the split view.
    targets: [Option<Target>; 2],
    /// The target the next `render` / `output` / `read_back` use.
    pub slot: usize,
    pub tone: Tone,
    /// Clear to transparent (the 2D flat colours are drawn under the view).
    pub clear_alpha: bool,
    pub triangles: usize,
    samples: u32,
}

/// View options passed to the terrain shader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewFlags {
    /// Tint every face with its slab owner's colour.
    pub ownership: bool,
    /// Slab grid on top faces.
    pub grid: bool,
    /// Ambient + directional face lighting (else the old eye-facing shading).
    pub lighting: bool,
    /// Draw the terrain at all.
    pub terrain: bool,
}

impl ViewFlags {
    fn bits(self) -> u32 {
        self.ownership as u32 | (self.grid as u32) << 1 | (self.lighting as u32) << 2
    }
}

fn line_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    on_top: bool,
    tris: bool,
    samples: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(match (on_top, tris) {
            (_, true) => "overlay-tris",
            (true, false) => "lines-top",
            (false, false) => "lines-depth",
        }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_line"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: if tris {
                wgpu::PrimitiveTopology::TriangleList
            } else {
                wgpu::PrimitiveTopology::LineList
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(if on_top {
                wgpu::CompareFunction::Always
            } else {
                wgpu::CompareFunction::LessEqual
            }),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_line"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl ViewRenderer {
    /// A renderer with `samples` MSAA samples (1 = no multisampling).
    pub fn with_samples(device: &wgpu::Device, samples: u32) -> Self {
        let samples = if samples > 1 { SAMPLES } else { 1 };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("view"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("view"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Uint16x2, 1 => Uint8x4, 2 => Uint32],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let line_pipeline_top = line_pipeline(device, &layout, &shader, true, false, samples);
        let line_pipeline_depth = line_pipeline(device, &layout, &shader, false, false, samples);
        let tri_pipeline = line_pipeline(device, &layout, &shader, false, true, samples);
        let (sprite_pipeline, sprite_blend_pipeline, sprite_globals, sprite_tex, sprite_bind_group) =
            Self::sprite_setup(device, samples);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ViewRenderer {
            terrain_pipeline,
            line_pipeline_top,
            line_pipeline_depth,
            tri_pipeline,
            sprite_pipeline,
            sprite_blend_pipeline,
            sprite_globals,
            sprite_tex,
            sprite_bind_group,
            flags: ViewFlags {
                lighting: true,
                terrain: true,
                ..Default::default()
            },
            bgl,
            globals,
            bind_group: None,
            chunks: vec![],
            targets: [None, None],
            slot: 0,
            tone: Tone {
                exposure: 1.0,
                lights: vec![],
            },
            clear_alpha: false,
            triangles: 0,
            samples,
        }
    }

    fn sprite_setup(
        device: &wgpu::Device,
        samples: u32,
    ) -> (
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::Buffer,
        wgpu::Texture,
        wgpu::BindGroup,
    ) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let mk = |blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if blend { "sprite-blend" } else { "sprite" }),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_sprite"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SpriteInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32,
                            2 => Float32x4,
                            3 => Float32x4,
                            4 => Float32x4,
                            5 => Uint32
                        ],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!blend),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if blend {
                        "fs_sprite_blend"
                    } else {
                        "fs_sprite"
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = mk(false);
        let blend_pipeline = mk(true);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite-globals"),
            size: std::mem::size_of::<SpriteGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        (pipeline, blend_pipeline, globals, tex, bg)
    }

    /// Upload the frames the sprite atlas added since the last call.
    pub fn upload_sprites(&mut self, queue: &wgpu::Queue, atlas: &mut SpriteAtlas) {
        for (s, px) in atlas.pending.drain(..) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.sprite_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: s.x,
                        y: s.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &px,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(s.w * 4),
                    rows_per_image: Some(s.h),
                },
                wgpu::Extent3d {
                    width: s.w,
                    height: s.h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Upload a level's atlas.
    pub fn set_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, atlas: &AtlasData) {
        // At least 2 layers: the GL backend creates a single-layer texture as a plain 2D
        // texture, which a `texture_2d_array` binding then cannot read (all black).
        let layers = atlas.layers.len().max(2) as u32;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: LAYER_W,
                height: LAYER_H,
                depth_or_array_layers: layers,
            },
            mip_level_count: MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (l, mips) in atlas.layers.iter().enumerate() {
            for (m, px) in mips.iter().enumerate() {
                let (w, h) = (LAYER_W >> m, LAYER_H >> m);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex,
                        mip_level: m as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: l as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    px,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w * 4),
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
        let lut_h = (atlas.lookup.len() as u32) / BLOCKS_PER_VARIATION;
        debug_assert_eq!(lut_h, VARIATIONS * 8);
        let lut = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("lut"),
                size: wgpu::Extent3d {
                    width: BLOCKS_PER_VARIATION,
                    height: lut_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&atlas.lookup),
        );
        let atlas_view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let lut_view = lut.create_view(&Default::default());
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
            ],
        }));
    }

    /// Replace all chunk meshes.
    pub fn set_chunks(&mut self, device: &wgpu::Device, meshes: &[ChunkMesh]) {
        self.chunks.clear();
        self.triangles = 0;
        for i in 0..meshes.len() {
            self.chunks.push(None);
            self.update_chunk(device, i, &meshes[i]);
        }
    }

    /// Replace one chunk mesh (local re-mesh after an edit).
    pub fn update_chunk(&mut self, device: &wgpu::Device, i: usize, m: &ChunkMesh) {
        if let Some(Some(old)) = self.chunks.get(i) {
            self.triangles -= old.count as usize / 3;
        }
        let gc = (!m.indices.is_empty()).then(|| GpuChunk {
            vb: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk-vb"),
                contents: bytemuck::cast_slice(&m.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            ib: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk-ib"),
                contents: bytemuck::cast_slice(&m.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: m.indices.len() as u32,
        });
        if let Some(g) = &gc {
            self.triangles += g.count as usize / 3;
        }
        if i >= self.chunks.len() {
            self.chunks.resize_with(i + 1, || None);
        }
        self.chunks[i] = gc;
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let slot = self.slot.min(1);
        if self.targets[slot].as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let mk = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = (self.samples > 1).then(|| {
            mk(
                "view-msaa",
                COLOR_FORMAT,
                self.samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let depth = mk(
            "view-depth",
            DEPTH_FORMAT,
            self.samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolve = mk(
            "view-color",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        self.targets[slot] = Some(Target {
            size,
            msaa: msaa.map(|m| m.create_view(&Default::default())),
            depth: depth.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            _resolve: resolve,
        });
    }

    /// The resolved colour texture of the last render.
    pub fn output(&self) -> Option<(&wgpu::Texture, &wgpu::TextureView)> {
        self.targets[self.slot.min(1)]
            .as_ref()
            .map(|t| (&t._resolve, &t.resolve_view))
    }

    /// Render into the offscreen target of `size` pixels. `top_lines` are drawn over everything,
    /// `depth_lines` are depth tested, `tris` are translucent depth-tested triangles (previews).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cam: &Camera,
        size: (u32, u32),
        frame: u32,
        depth_lines: &[LineVertex],
        top_lines: &[LineVertex],
        tris: &[LineVertex],
        sprites: &[SpriteInstance],
    ) -> wgpu::SubmissionIndex {
        let size = (size.0.max(1), size.1.max(1));
        self.ensure_target(device, size);
        let vp = Vec2::new(size.0 as f32, size.1 as f32);
        let mut lights = [[0.0f32; 4]; MAX_LIGHTS];
        let mut power = [[0.0f32; 4]; MAX_LIGHTS / 4];
        let n = self.tone.lights.len().min(MAX_LIGHTS);
        for (i, l) in self.tone.lights.iter().take(MAX_LIGHTS).enumerate() {
            lights[i] = [l.pos.x, l.pos.y, l.pos.z, l.range.max(0.1)];
            power[i / 4][i % 4] = l.power.clamp(0.0, 1.0);
        }
        let g = Globals {
            view_proj: cam.view_proj(vp).to_cols_array_2d(),
            eye_dir: cam.back().extend(0.0).to_array(),
            params: [frame % 8, self.flags.bits(), n as u32, 0],
            tone: [self.tone.exposure.max(0.1), 1.0, 0.0, 0.0],
            lights,
            light_power: power,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let back = cam.back();
        let (right, up) = cam.screen_axes();
        let sg = SpriteGlobals {
            view_proj: g.view_proj,
            right: right.extend(0.0).to_array(),
            up: up.extend(0.0).to_array(),
            eye: back.extend(SPRITE_DEPTH_BIAS).to_array(),
        };
        queue.write_buffer(&self.sprite_globals, 0, bytemuck::bytes_of(&sg));
        let mk_lines = |v: &[LineVertex]| {
            (!v.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lines"),
                    contents: bytemuck::cast_slice(v),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        let dl = mk_lines(depth_lines);
        let tl = mk_lines(top_lines);
        let tb = mk_lines(tris);
        // opaque first, translucent after
        let mut ordered: Vec<SpriteInstance> = sprites
            .iter()
            .filter(|s| s.flags & 4 == 0)
            .copied()
            .collect();
        let n_opaque = ordered.len() as u32;
        ordered.extend(sprites.iter().filter(|s| s.flags & 4 != 0).copied());
        let sb = (!ordered.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sprites"),
                contents: bytemuck::cast_slice(&ordered),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let t = self.targets[self.slot.min(1)].as_ref().unwrap();
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("view"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: t.msaa.as_ref().unwrap_or(&t.resolve_view),
                    depth_slice: None,
                    resolve_target: t.msaa.as_ref().map(|_| &t.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if self.clear_alpha {
                            wgpu::Color::TRANSPARENT
                        } else {
                            wgpu::Color {
                                r: CLEAR[0],
                                g: CLEAR[1],
                                b: CLEAR[2],
                                a: 1.0,
                            }
                        }),
                        store: if t.msaa.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(bg) = &self.bind_group {
                pass.set_bind_group(0, bg, &[]);
                pass.set_pipeline(&self.terrain_pipeline);
                for c in self.chunks.iter().flatten().filter(|_| self.flags.terrain) {
                    pass.set_vertex_buffer(0, c.vb.slice(..));
                    pass.set_index_buffer(c.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..c.count, 0, 0..1);
                }
                if let Some(b) = &sb {
                    pass.set_pipeline(&self.sprite_pipeline);
                    pass.set_bind_group(0, &self.sprite_bind_group, &[]);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..6, 0..n_opaque);
                    if (n_opaque as usize) < ordered.len() {
                        pass.set_pipeline(&self.sprite_blend_pipeline);
                        pass.draw(0..6, n_opaque..ordered.len() as u32);
                    }
                    pass.set_bind_group(0, bg, &[]);
                }
                if let Some(b) = &tb {
                    pass.set_pipeline(&self.tri_pipeline);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..tris.len() as u32, 0..1);
                }
                if let Some(b) = &dl {
                    pass.set_pipeline(&self.line_pipeline_depth);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..depth_lines.len() as u32, 0..1);
                }
                if let Some(b) = &tl {
                    pass.set_pipeline(&self.line_pipeline_top);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..top_lines.len() as u32, 0..1);
                }
            }
        }
        queue.submit([enc.finish()])
    }

    /// Copy the last render to the CPU as tightly packed RGBA8 (blocking).
    pub fn read_back(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.targets[self.slot.min(1)].as_ref()?;
        read_texture(device, queue, &t._resolve, t.size)
    }
}

/// Blocking read-back of an RGBA8 texture.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    size: (u32, u32),
) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = size;
    let row = (w * 4).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
    rx.recv().ok()?.ok()?;
    let data = slice.get_mapped_range().ok()?;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let o = (y * row) as usize;
        out.extend_from_slice(&data[o..o + (w * 4) as usize]);
    }
    drop(data);
    buf.unmap();
    Some((w, h, out))
}
