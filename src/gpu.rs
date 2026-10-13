//! Hardware rasterizer for the display list through wgpu: Vulkan on Linux (NVIDIA, AMD, Intel),
//! Metal on Apple GPUs, DX12 on Windows. It renders colour, picking and depth like the CPU
//! rasterizer and reads colour and picking back for the terminal and mouse hit-testing.

use std::mem::{offset_of, size_of};
use std::ops::Range;
use std::sync::{Arc, Mutex, mpsc};

use bytemuck::{Pod, Zeroable};

use crate::render::field::StorageLayout;
use crate::render::shared::{self, LINE_DEPTH_BIAS, NONE};
use crate::render::{Backdrop, Color, Frame, Item};

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const PICK: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    size: [f32; 2],
    depth: [f32; 2],
    haze: [f32; 4],
    sky: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    depth: f32,
    color: [u8; 4],
    pick: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Ball {
    center: [f32; 3],
    radius: f32,
    world: f32,
    color: [u8; 4],
    pick: u32,
    selected: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Glow {
    center: [f32; 2],
    radius: f32,
    strength: f32,
    color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Glyph {
    origin: [f32; 2],
    scale: f32,
    color: [u8; 4],
    bits: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Splat {
    center: [f32; 2],
    radius: f32,
    depth: f32,
    pick: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FieldVertex {
    position: [f32; 2],
    depth: f32,
    uv: [f32; 2],
    /// The word offset of the field's slot in the storage buffer.
    slot: u32,
}

// Layout assertions. Each struct is uploaded as raw bytes and read through a vertex layout made
// of tightly packed attributes (see `vertex_attr_array!` below), so its size and field offsets
// must match the WGSL struct of the same name in shaders.wgsl.

// Globals: uniform buffer; WGSL `Globals` (vec2, vec2, vec4, vec4), 48 bytes.
const _: () = {
    assert!(size_of::<Globals>() == 48);
    assert!(offset_of!(Globals, size) == 0);
    assert!(offset_of!(Globals, depth) == 8);
    assert!(offset_of!(Globals, haze) == 16);
    assert!(offset_of!(Globals, sky) == 32);
};

// Vertex: WGSL `Vertex`, attributes Float32x2, Float32, Unorm8x4, Uint32.
const _: () = {
    assert!(size_of::<Vertex>() == 20);
    assert!(offset_of!(Vertex, position) == 0);
    assert!(offset_of!(Vertex, depth) == 8);
    assert!(offset_of!(Vertex, color) == 12);
    assert!(offset_of!(Vertex, pick) == 16);
};

// Ball: WGSL `Ball`, attributes Float32x3, Float32, Float32, Unorm8x4, Uint32, Uint32.
const _: () = {
    assert!(size_of::<Ball>() == 32);
    assert!(offset_of!(Ball, center) == 0);
    assert!(offset_of!(Ball, radius) == 12);
    assert!(offset_of!(Ball, world) == 16);
    assert!(offset_of!(Ball, color) == 20);
    assert!(offset_of!(Ball, pick) == 24);
    assert!(offset_of!(Ball, selected) == 28);
};

// Glow: WGSL `Glow`, attributes Float32x2, Float32, Float32, Unorm8x4.
const _: () = {
    assert!(size_of::<Glow>() == 20);
    assert!(offset_of!(Glow, center) == 0);
    assert!(offset_of!(Glow, radius) == 8);
    assert!(offset_of!(Glow, strength) == 12);
    assert!(offset_of!(Glow, color) == 16);
};

// Glyph: WGSL `Glyph`, attributes Float32x2, Float32, Unorm8x4, Uint32x4.
const _: () = {
    assert!(size_of::<Glyph>() == 32);
    assert!(offset_of!(Glyph, origin) == 0);
    assert!(offset_of!(Glyph, scale) == 8);
    assert!(offset_of!(Glyph, color) == 12);
    assert!(offset_of!(Glyph, bits) == 16);
};

// Splat: WGSL `Splat`, attributes Float32x2, Float32, Float32, Uint32.
const _: () = {
    assert!(size_of::<Splat>() == 20);
    assert!(offset_of!(Splat, center) == 0);
    assert!(offset_of!(Splat, radius) == 8);
    assert!(offset_of!(Splat, depth) == 12);
    assert!(offset_of!(Splat, pick) == 16);
};

// FieldVertex: WGSL `FieldVertex`, attributes Float32x2, Float32, Float32x2, Uint32.
const _: () = {
    assert!(size_of::<FieldVertex>() == 24);
    assert!(offset_of!(FieldVertex, position) == 0);
    assert!(offset_of!(FieldVertex, depth) == 8);
    assert!(offset_of!(FieldVertex, uv) == 12);
    assert!(offset_of!(FieldVertex, slot) == 20);
};

/// The shader source: the generated constants from `render::shared`, then `shaders.wgsl`.
fn shader_source() -> String {
    format!("{}{}", shared::prelude(), include_str!("shaders.wgsl"))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pass {
    Triangles,
    Lines,
    Beams,
    Stars,
    Balls,
    Glows,
    Glyphs,
    Splats,
    Fields,
}

struct Targets {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    pick: wgpu::Texture,
    color_view: wgpu::TextureView,
    pick_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    /// Bytes per colour row in the readback buffer, padded to wgpu's copy alignment.
    row: u32,
}

/// Picking is read back only in a window around the pointer: hover and clicks never look further.
const PICK_WINDOW: u32 = 160;
const PICK_ROW: u32 = (PICK_WINDOW * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// A vertex or instance buffer reused across frames and grown when a frame needs more room.
#[derive(Default)]
struct Stream {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
}

impl Stream {
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
    ) -> Option<wgpu::Buffer> {
        if data.is_empty() {
            return None;
        }
        if self.capacity < data.len() as u64 {
            self.capacity = (data.len() as u64).next_power_of_two().max(4096);
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("stream"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.buffer.clone()?;
        queue.write_buffer(&buffer, 0, data);
        Some(buffer)
    }
}

/// Words the storage buffer starts with, before any field needs more: a quarter of a mebibyte
/// holds a 180 x 180 field with owners.
const FIELD_STORAGE_WORDS: usize = 1 << 16;

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    fields: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: fields.as_entire_binding(),
            },
        ],
    })
}

/// The storage buffer the field pass samples: a header, node colours and owners for each field of
/// a frame (see `render::field::StorageLayout`), rewritten only where a field changed.
struct FieldStorage {
    buffer: wgpu::Buffer,
    /// What the buffer holds, in words.
    capacity: usize,
    layout: StorageLayout,
}

impl FieldStorage {
    fn new(device: &wgpu::Device, words: usize) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fields"),
                size: words as u64 * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity: words,
            layout: StorageLayout::default(),
        }
    }
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub name: String,
    globals: wgpu::Buffer,
    bind_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    fields: FieldStorage,
    sky: wgpu::RenderPipeline,
    twilight: wgpu::RenderPipeline,
    pipelines: [(Pass, wgpu::RenderPipeline); 9],
    targets: Option<Targets>,
    streams: [Stream; 6],
    scratch: Vec<u8>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Gpu {
    /// Opens the preferred hardware adapter. Software adapters are refused: the CPU rasterizer
    /// is faster than an emulated GPU.
    #[expect(clippy::too_many_lines, reason = "split in #87 (A28)")]
    pub fn new(power: wgpu::PowerPreference) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: power,
            ..Default::default()
        }))
        .map_err(|error| format!("no GPU adapter: {error}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(format!("{} is a software adapter", info.name));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("isotop"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|error| format!("GPU device: {error}"))?;
        let failure = Arc::new(Mutex::new(None));
        let report = Arc::clone(&failure);
        device.on_uncaptured_error(Arc::new(move |error| {
            if let Ok(mut slot) = report.lock() {
                slot.get_or_insert_with(|| error.to_string());
            }
        }));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("isotop shaders"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fields = FieldStorage::new(&device, FIELD_STORAGE_WORDS);
        let bind_group = bind_group(&device, &layout, &globals, &fields.buffer);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("isotop"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Unorm8x4, 3 => Uint32],
        };
        let ball = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Ball>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32, 3 => Unorm8x4, 4 => Uint32, 5 => Uint32],
        };
        let glow = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Glow>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Unorm8x4],
        };
        let glyph = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Glyph>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Unorm8x4, 3 => Uint32x4],
        };
        let splat = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Splat>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Uint32],
        };
        let field = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FieldVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x2, 3 => Uint32],
        };
        let make = |entry: (&str, &str),
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    topology: wgpu::PrimitiveTopology,
                    style: Style| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry.0),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry.0),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry.1),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: COLOR,
                            blend: style.blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: if style.pick_only {
                                wgpu::ColorWrites::empty()
                            } else {
                                wgpu::ColorWrites::ALL
                            },
                        }),
                        Some(wgpu::ColorTargetState {
                            format: PICK,
                            blend: None,
                            write_mask: if style.solid || style.pick_only {
                                wgpu::ColorWrites::ALL
                            } else {
                                wgpu::ColorWrites::empty()
                            },
                        }),
                    ],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(style.solid),
                    depth_compare: Some(if style.solid || style.pick_only {
                        wgpu::CompareFunction::GreaterEqual
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid = Style {
            solid: true,
            blend: false,
            pick_only: false,
        };
        let backdrop = Style {
            solid: false,
            blend: false,
            pick_only: false,
        };
        let translucent = Style {
            solid: false,
            blend: true,
            pick_only: false,
        };
        let pick_only = Style {
            solid: false,
            blend: false,
            pick_only: true,
        };
        use wgpu::PrimitiveTopology::{LineList, PointList, TriangleList};
        let geometry = ("geometry_vertex", "geometry_fragment");
        let pipelines = [
            (
                Pass::Triangles,
                make(geometry, &[Some(vertex.clone())], TriangleList, solid),
            ),
            (
                Pass::Lines,
                make(geometry, &[Some(vertex.clone())], LineList, solid),
            ),
            (
                Pass::Beams,
                make(geometry, &[Some(vertex.clone())], LineList, translucent),
            ),
            (
                Pass::Stars,
                make(geometry, &[Some(vertex)], PointList, backdrop),
            ),
            (
                Pass::Balls,
                make(
                    ("ball_vertex", "ball_fragment"),
                    &[Some(ball)],
                    TriangleList,
                    solid,
                ),
            ),
            (
                Pass::Glows,
                make(
                    ("glow_vertex", "glow_fragment"),
                    &[Some(glow)],
                    TriangleList,
                    translucent,
                ),
            ),
            (
                Pass::Glyphs,
                make(
                    ("glyph_vertex", "glyph_fragment"),
                    &[Some(glyph)],
                    TriangleList,
                    backdrop,
                ),
            ),
            (
                Pass::Splats,
                make(
                    ("splat_vertex", "splat_fragment"),
                    &[Some(splat)],
                    TriangleList,
                    pick_only,
                ),
            ),
            (
                Pass::Fields,
                make(
                    ("field_vertex", "field_fragment"),
                    &[Some(field)],
                    TriangleList,
                    solid,
                ),
            ),
        ];
        let sky = make(("sky_vertex", "sky_fragment"), &[], TriangleList, backdrop);
        let twilight = make(
            ("sky_vertex", "twilight_fragment"),
            &[],
            TriangleList,
            backdrop,
        );
        if let Some(error) = failure.lock().ok().and_then(|mut slot| slot.take()) {
            return Err(error);
        }
        Ok(Self {
            name: format!("{} ({:?})", info.name, info.backend),
            device,
            queue,
            globals,
            bind_layout: layout,
            bind_group,
            fields,
            sky,
            twilight,
            pipelines,
            targets: None,
            streams: Default::default(),
            scratch: Vec::new(),
            failure,
        })
    }

    /// Writes the frame's fields into the storage buffer where they differ from what it holds,
    /// growing it first if they need more room, and returns where each one sits.
    fn store_fields(&mut self, frame: &Frame) -> Result<crate::render::field::StoragePlan, String> {
        let plan = self.fields.layout.plan(&frame.fields, self.fields.capacity);
        if let Some(words) = plan.grow {
            let limit = self.device.limits().max_storage_buffer_binding_size;
            if words as u64 * 4 > limit {
                self.fields.layout.forget();
                return Err(format!(
                    "the frame's fields need {} MiB of GPU storage, over the {} MiB the GPU binds",
                    (words as u64 * 4) >> 20,
                    limit >> 20
                ));
            }
            let layout = std::mem::take(&mut self.fields.layout);
            self.fields = FieldStorage {
                layout,
                ..FieldStorage::new(&self.device, words)
            };
            self.bind_group = bind_group(
                &self.device,
                &self.bind_layout,
                &self.globals,
                &self.fields.buffer,
            );
        }
        for &index in &plan.rewrite {
            let field = &frame.fields[index];
            let slot = plan.offsets[index] as u64 * 4;
            let header = StorageLayout::header(field);
            self.queue
                .write_buffer(&self.fields.buffer, slot, bytemuck::cast_slice(&header));
            let texels = slot + header.len() as u64 * 4;
            self.queue.write_buffer(
                &self.fields.buffer,
                texels,
                bytemuck::cast_slice(&field.texels),
            );
            if let Some(owners) = &field.owners {
                self.queue.write_buffer(
                    &self.fields.buffer,
                    texels + field.texels.len() as u64 * 4,
                    bytemuck::cast_slice(owners),
                );
            }
        }
        Ok(plan)
    }

    fn targets(&mut self, width: u32, height: u32) -> &Targets {
        if self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let texture = |label, format, usage| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let readable = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
            let color = texture("color", COLOR, readable);
            let pick = texture("pick", PICK, readable);
            let depth = texture("depth", DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: row as u64 * height as u64 + PICK_ROW as u64 * PICK_WINDOW as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.targets = Some(Targets {
                width,
                height,
                color_view: color.create_view(&Default::default()),
                pick_view: pick.create_view(&Default::default()),
                depth_view: depth.create_view(&Default::default()),
                color,
                pick,
                readback,
                row,
            });
        }
        self.targets.as_ref().expect("targets were just created")
    }

    /// Rasterizes the frame's display list into its pixels, and its picking buffer around `pointer`.
    #[expect(clippy::too_many_lines, reason = "split in #87 (A28)")]
    pub fn render(&mut self, frame: &mut Frame, pointer: Option<[f32; 2]>) -> Result<(), String> {
        let (width, height) = (frame.width, frame.height);
        let mut vertices = Vec::with_capacity(frame.items.len() * 2);
        let mut balls = Vec::new();
        let mut glows = Vec::new();
        let mut glyphs = Vec::new();
        let mut splats = Vec::new();
        let mut field_vertices = Vec::new();
        let plan = self.store_fields(frame)?;
        let mut batches: Vec<(Pass, Range<u32>)> = Vec::new();
        let mut depth = [f32::INFINITY, f32::NEG_INFINITY];
        let mut track = |d: f32| {
            depth[0] = depth[0].min(d);
            depth[1] = depth[1].max(d);
        };
        let rgba = |color: Color, alpha: u8| [color[0], color[1], color[2], alpha];
        for item in &frame.items {
            let (pass, start) = match *item {
                Item::Sphere { .. } => (Pass::Balls, balls.len()),
                Item::Glow { .. } | Item::Puff { .. } => (Pass::Glows, glows.len()),
                Item::Glyph { .. } => (Pass::Glyphs, glyphs.len()),
                Item::Triangle(..) => (Pass::Triangles, vertices.len()),
                Item::Line(..) => (Pass::Lines, vertices.len()),
                Item::Beam(..) => (Pass::Beams, vertices.len()),
                Item::Star(..) => (Pass::Stars, vertices.len()),
                Item::Pick { .. } => (Pass::Splats, splats.len()),
                Item::Field { .. } => (Pass::Fields, field_vertices.len()),
            };
            match *item {
                Item::Triangle(points, color, pick) => {
                    for p in points {
                        track(p[2]);
                        vertices.push(Vertex {
                            position: [p[0], p[1]],
                            depth: p[2],
                            color: rgba(color, 255),
                            pick,
                        });
                    }
                }
                Item::Line(a, b, color) => {
                    for p in [a, b] {
                        track(p[2] + LINE_DEPTH_BIAS);
                        vertices.push(Vertex {
                            position: [p[0], p[1]],
                            depth: p[2] + LINE_DEPTH_BIAS,
                            color: rgba(color, 255),
                            pick: NONE,
                        });
                    }
                }
                Item::Beam(a, b, color, weight) => {
                    for p in [a, b] {
                        vertices.push(Vertex {
                            position: p,
                            depth: 0.0,
                            color: rgba(color, (weight.clamp(0.0, 1.0) * 255.0) as u8),
                            pick: NONE,
                        });
                    }
                }
                Item::Star(p, color) => vertices.push(Vertex {
                    position: [p[0] + 0.5, p[1] + 0.5],
                    depth: 0.0,
                    color: rgba(color, 255),
                    pick: NONE,
                }),
                Item::Sphere {
                    center,
                    radius,
                    depth: world,
                    color,
                    pick,
                    selected,
                } => {
                    track(center[2] - world);
                    track(center[2] + world);
                    balls.push(Ball {
                        center,
                        radius,
                        world,
                        color: rgba(color, 255),
                        pick,
                        selected: selected as u32,
                    });
                }
                // The blend unit rounds, which is what a `Puff` does on the CPU; a `Glow` truncates.
                Item::Glow {
                    center,
                    radius,
                    color,
                    strength,
                }
                | Item::Puff {
                    center,
                    radius,
                    color,
                    strength,
                } => glows.push(Glow {
                    center,
                    radius,
                    strength,
                    color: rgba(color, 255),
                }),
                Item::Glyph {
                    origin,
                    scale,
                    bits,
                    color,
                } => glyphs.push(Glyph {
                    origin,
                    scale: scale as f32,
                    color: rgba(color, 255),
                    bits: [0, 32, 64, 96].map(|shift| (bits >> shift) as u32),
                }),
                Item::Pick {
                    center,
                    radius,
                    depth,
                    pick,
                } => {
                    track(depth);
                    splats.push(Splat {
                        center,
                        radius,
                        depth,
                        pick,
                    });
                }
                Item::Field { points, uv, field } => {
                    for (p, uv) in points.into_iter().zip(uv) {
                        track(p[2]);
                        field_vertices.push(FieldVertex {
                            position: [p[0], p[1]],
                            depth: p[2],
                            uv,
                            slot: plan.offsets[field as usize],
                        });
                    }
                }
            }
            let end = match pass {
                Pass::Balls => balls.len(),
                Pass::Glows => glows.len(),
                Pass::Glyphs => glyphs.len(),
                Pass::Splats => splats.len(),
                Pass::Fields => field_vertices.len(),
                _ => vertices.len(),
            };
            match batches.last_mut() {
                Some((last, range)) if *last == pass && range.end == start as u32 => {
                    range.end = end as u32
                }
                _ => batches.push((pass, start as u32..end as u32)),
            }
        }
        if depth[0] > depth[1] {
            depth = [0.0, 1.0];
        }
        let sky = frame.sky;
        let globals = Globals {
            size: [width as f32, height as f32],
            depth: [depth[0], 1.0 / (depth[1] - depth[0]).max(1e-3)],
            haze: [sky.haze[0], sky.haze[1], sky.haze[2], sky.time],
            sky: [sky.backdrop as u32 as f32, 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let [
            vertex_stream,
            ball_stream,
            glow_stream,
            glyph_stream,
            splat_stream,
            field_stream,
        ] = &mut self.streams;
        let vertex_buffer =
            vertex_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&vertices));
        let ball_buffer =
            ball_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&balls));
        let glow_buffer =
            glow_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&glows));
        let glyph_buffer =
            glyph_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&glyphs));
        let splat_buffer =
            splat_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&splats));
        let field_buffer = field_stream.upload(
            &self.device,
            &self.queue,
            bytemuck::cast_slice(&field_vertices),
        );
        self.targets(width, height);
        let targets = self.targets.as_ref().expect("targets exist");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let attachment = |view, clear| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })
            };
            let none = wgpu::Color {
                r: NONE as f64,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[
                    attachment(&targets.color_view, wgpu::Color::BLACK),
                    attachment(&targets.pick_view, none),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(if frame.sky.backdrop == Backdrop::Twilight {
                &self.twilight
            } else {
                &self.sky
            });
            pass.draw(0..3, 0..1);
            for (kind, range) in batches {
                let pipeline = &self
                    .pipelines
                    .iter()
                    .find(|(pass, _)| *pass == kind)
                    .expect("every pass has a pipeline")
                    .1;
                pass.set_pipeline(pipeline);
                match kind {
                    Pass::Balls | Pass::Glows | Pass::Glyphs | Pass::Splats => {
                        let buffer = match kind {
                            Pass::Balls => &ball_buffer,
                            Pass::Glows => &glow_buffer,
                            Pass::Splats => &splat_buffer,
                            _ => &glyph_buffer,
                        };
                        if let Some(buffer) = buffer {
                            pass.set_vertex_buffer(0, buffer.slice(..));
                            pass.draw(0..6, range);
                        }
                    }
                    Pass::Fields => {
                        if let Some(buffer) = &field_buffer {
                            pass.set_vertex_buffer(0, buffer.slice(..));
                            pass.draw(range, 0..1);
                        }
                    }
                    _ => {
                        if let Some(buffer) = &vertex_buffer {
                            pass.set_vertex_buffer(0, buffer.slice(..));
                            pass.draw(range, 0..1);
                        }
                    }
                }
            }
        }
        let window = pointer.map(|[x, y]| {
            let size = [PICK_WINDOW.min(width), PICK_WINDOW.min(height)];
            let corner = |at: f32, extent: u32, limit: u32| {
                (at as i64 - extent as i64 / 2).clamp(0, (limit - extent) as i64) as u32
            };
            (
                [corner(x, size[0], width), corner(y, size[1], height)],
                size,
            )
        });
        let mut copies = vec![(&targets.color, [0, 0], [width, height], 0, targets.row)];
        if let Some((origin, size)) = window {
            let offset = targets.row as u64 * height as u64;
            copies.push((&targets.pick, origin, size, offset, PICK_ROW));
        }
        for (texture, origin, size, offset, row) in copies {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: origin[0],
                        y: origin[1],
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &targets.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(size[1]),
                    },
                },
                wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit(Some(encoder.finish()));
        let slice = targets.readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("GPU poll: {error}"))?;
        receiver
            .recv()
            .map_err(|error| format!("GPU readback: {error}"))?
            .map_err(|error| format!("GPU readback: {error}"))?;
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|error| format!("GPU readback: {error}"))?;
            // Mapped readback memory can be uncached (Intel); one bulk copy into cached memory is
            // hundreds of times faster than reading it piecemeal during the conversion below.
            copy_mapped(&mapped, &mut self.scratch);
            drop(mapped);
            let data = &self.scratch;
            let row = targets.row as usize;
            let (w, h) = (width as usize, height as usize);
            frame.pixels.resize(w * h * 3, 0);
            frame.picks.clear();
            frame.picks.resize(w * h, NONE);
            for y in 0..h {
                let colors = &data[y * row..y * row + w * 4];
                for (pixel, rgba) in frame.pixels[y * w * 3..(y + 1) * w * 3]
                    .as_chunks_mut::<3>()
                    .0
                    .iter_mut()
                    .zip(colors.as_chunks::<4>().0)
                {
                    pixel.copy_from_slice(&rgba[..3]);
                }
            }
            if let Some((origin, size)) = window {
                let base = row * h;
                for wy in 0..size[1] as usize {
                    let source = &data[base + wy * PICK_ROW as usize..][..size[0] as usize * 4];
                    let start = (origin[1] as usize + wy) * w + origin[0] as usize;
                    for (pick, bytes) in frame.picks[start..start + size[0] as usize]
                        .iter_mut()
                        .zip(source.as_chunks::<4>().0)
                    {
                        *pick = u32::from_le_bytes(*bytes);
                    }
                }
            }
        }
        targets.readback.unmap();
        if let Some(error) = self.failure.lock().ok().and_then(|mut slot| slot.take()) {
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Style {
    /// Depth-tested, depth-writing and pickable; otherwise drawn over the scene without depth.
    solid: bool,
    blend: bool,
    /// Depth-tested like solid geometry, but writes only picks: no colour and no depth.
    pick_only: bool,
}

/// Copies GPU readback memory into `out`. Mapped readback memory may be write-combined (Intel
/// integrated GPUs), where ordinary loads are uncached and slow; SSE4.1 streaming loads read it at
/// close to normal memory speed.
fn copy_mapped(source: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(source.len());
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("sse4.1")
        && (source.as_ptr() as usize).is_multiple_of(16)
    {
        let whole = source.len() / 16 * 16;
        // SAFETY: SSE4.1 is available, the source is 16-byte aligned and every load stays within
        // `whole` bytes of it; the destination has capacity for `source.len()` bytes and its length
        // is set only after those bytes are written.
        unsafe {
            stream_copy(source.as_ptr(), out.as_mut_ptr(), whole);
            out.set_len(whole);
        }
        out.extend_from_slice(&source[whole..]);
        return;
    }
    out.extend_from_slice(source);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn stream_copy(source: *const u8, target: *mut u8, length: usize) {
    use std::arch::x86_64::{__m128i, _mm_storeu_si128, _mm_stream_load_si128};
    for offset in (0..length).step_by(16) {
        // SAFETY: the caller guarantees alignment, SSE4.1 support and in-bounds offsets.
        unsafe {
            let chunk = _mm_stream_load_si128(source.add(offset) as *const __m128i);
            _mm_storeu_si128(target.add(offset) as *mut __m128i, chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::medium::Grid;
    use crate::model::Identity;
    use crate::render::{
        Backdrop, Buffers, Camera, Field, Placement, Relief, Shading, Sky, Target, Thing,
    };

    /// A frame small enough for the GPU to read back every pick: a process's triangle over the
    /// left half at depth 5, a thing's sphere, a thing's pick splat behind the triangle and
    /// another in front of it, and a process splat over the sky.
    fn picks_frame() -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Dusk,
            haze: [0.0; 3],
            time: 0.0,
        };
        let mut frame = Frame::new(120, 80, sky, Buffers::default());
        frame.identities = vec![Identity { pid: 7, start: 1 }];
        let [ball, behind, front] =
            [1, 2, 3].map(|key| frame.thing_pick(Thing { kind: "grave", key }));
        frame.items = vec![
            Item::Triangle(
                [[0.0, 0.0, 5.0], [60.0, 0.0, 5.0], [0.0, 80.0, 5.0]],
                [90; 3],
                0,
            ),
            Item::Sphere {
                center: [90.0, 20.0, 3.0],
                radius: 6.0,
                depth: 1.0,
                color: [200; 3],
                pick: ball,
                selected: false,
            },
            Item::Pick {
                center: [15.0, 15.0],
                radius: 8.0,
                depth: 2.0,
                pick: behind,
            },
            Item::Pick {
                center: [15.0, 40.0],
                radius: 8.0,
                depth: 9.0,
                pick: front,
            },
            Item::Pick {
                center: [90.0, 60.0],
                radius: 8.0,
                depth: 0.0,
                pick: 0,
            },
        ];
        frame
    }

    /// A dense clump of `count` dark puffs (or glows) over the twilight sky, as a flock draws.
    fn clump(count: usize, spread: [f32; 2], radius: f32, puffs: bool) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Twilight,
            haze: [0.0; 3],
            time: 0.0,
        };
        let mut frame = Frame::new(960, 540, sky, Buffers::default());
        let mut random = crate::simulation::Rng::new(7);
        let strength = 1.0 - (-1.1_f32).exp();
        for _ in 0..count {
            let center = [
                480.0 + random.normal() * spread[0],
                250.0 + random.normal() * spread[1],
            ];
            let color = [12, 10, 16];
            frame.items.push(if puffs {
                Item::Puff {
                    center,
                    radius,
                    color,
                    strength,
                }
            } else {
                Item::Glow {
                    center,
                    radius,
                    color,
                    strength,
                }
            });
        }
        frame
    }

    /// The share of pixels with a channel more than two levels apart, and the largest difference.
    fn disagreement(a: &Frame, b: &Frame) -> (f64, u8) {
        let mut over = 0;
        let mut largest = 0;
        for (x, y) in a.pixels.chunks(3).zip(b.pixels.chunks(3)) {
            let gaps = [0, 1, 2].map(|k| x[k].abs_diff(y[k]));
            over += usize::from(gaps.iter().any(|&gap| gap > 2));
            largest = largest.max(gaps.into_iter().max().unwrap_or(0));
        }
        (over as f64 / (a.pixels.len() / 3) as f64 * 100.0, largest)
    }

    #[test]
    #[ignore = "needs a hardware GPU; run with --ignored on one"]
    fn stacked_puffs_match_the_gpu_to_a_level_however_densely_they_stack() {
        let mut gpu = Gpu::new(wgpu::PowerPreference::LowPower).expect("a hardware GPU");
        for (count, spread, radius) in [
            (1800, [50.0, 35.0], 2.25),
            (8000, [80.0, 50.0], 2.25),
            (8000, [80.0, 50.0], 4.0),
        ] {
            let mut cpu = clump(count, spread, radius, true);
            cpu.rasterize();
            let mut hardware = clump(count, spread, radius, true);
            gpu.render(&mut hardware, None).expect("the GPU renders");
            let (share, largest) = disagreement(&cpu, &hardware);
            assert!(
                share == 0.0 && largest <= 1,
                "{count} puffs of radius {radius}: {share} % of pixels over 2 levels, largest {largest}"
            );
        }
    }

    #[test]
    #[ignore = "needs a hardware GPU; run with --ignored on one"]
    fn pick_splats_and_thing_picks_survive_the_gpu_readback() {
        let mut gpu = Gpu::new(wgpu::PowerPreference::LowPower).expect("a hardware GPU");
        let mut expected = picks_frame();
        expected.rasterize();
        let mut frame = picks_frame();
        gpu.render(&mut frame, Some([60.0, 40.0]))
            .expect("the GPU renders");
        let target = |frame: &Frame, x: usize, y: usize| frame.target(frame.picks[y * 120 + x]);
        let grave = |key| Some(Target::Thing(Thing { kind: "grave", key }));
        let process = Some(Target::Process(Identity { pid: 7, start: 1 }));
        for (x, y, wanted) in [
            (90, 20, grave(1)),
            (15, 15, process),
            (15, 40, grave(3)),
            (90, 60, process),
            (110, 75, None),
        ] {
            assert_eq!(target(&expected, x, y), wanted, "the CPU at ({x}, {y})");
            assert_eq!(target(&frame, x, y), wanted, "the GPU at ({x}, {y})");
        }
        let differing = (0..expected.picks.len())
            .filter(|&i| expected.picks[i] != frame.picks[i])
            .count();
        assert_eq!(differing, 0, "pick buffers differ in {differing} pixels");
        assert_eq!(
            frame.pixels, expected.pixels,
            "the GPU draws the CPU's pixels"
        );
    }

    #[test]
    fn the_shader_source_is_valid_wgsl() {
        use wgpu::naga::front::wgsl;
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};
        let module = wgsl::parse_str(&shader_source()).expect("the shader source parses");
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .expect("the shader source validates");
    }

    /// A field of `columns` by `rows` nodes: rolling colours, a smooth height and an owner per
    /// node, lit with a highlight. `phase` shifts every pattern, so two revisions differ.
    fn lumpy(key: u64, columns: usize, rows: usize, phase: f32) -> Field {
        let mut field = Field::new(key, columns, rows);
        let mut heights = Vec::with_capacity(columns * rows);
        for j in 0..rows {
            for i in 0..columns {
                let (x, y) = (i as f32 / columns as f32, j as f32 / rows as f32);
                let wave = |k: f32| 0.5 + 0.5 * ((x * 9.0 + y * 4.0) * k + phase).sin();
                field.colors[j * columns + i] = [
                    (wave(1.0) * 255.0) as u8,
                    (wave(1.7) * 200.0) as u8,
                    ((1.0 - y) * 220.0) as u8,
                ];
                heights.push(((x * 11.0 + phase).sin() * (y * 7.0).cos()) * 3.0);
            }
        }
        field.heights = Some(heights);
        field.owners = Some(
            (0..(columns * rows) as u32)
                .map(|node| node % 191)
                .collect(),
        );
        field.shading = Some(Shading {
            ambient: 0.45,
            diffuse: 0.75,
            specular: 0.35,
            shininess: 20.0,
        });
        field
    }

    /// The three ways to draw a field, each in a frame of its own.
    fn field_frame(scene: usize, field: &Field) -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Dusk,
            haze: [0.0; 3],
            time: 0.0,
        };
        let mut frame = Frame::new(640, 360, sky, Buffers::default());
        let camera = Camera {
            center: [0.0, 0.0],
            zoom: 5.0,
            rotation: 0.35,
            pitch: 0.8,
            ..Camera::default()
        };
        let (columns, rows) = (field.columns, field.rows);
        let grid = Grid {
            origin: [-(columns as f32) / 2.0 * 0.75, -(rows as f32) / 2.0 * 0.75],
            cell: 0.75,
            columns,
            rows,
        };
        match scene {
            0 => frame.field(
                &camera,
                Placement::Grid { grid, z: 0.0 },
                field,
                Relief::Flat,
            ),
            1 => frame.field(
                &camera,
                Placement::Grid { grid, z: 0.0 },
                field,
                Relief::displaced(1.5),
            ),
            _ => {
                // A panel that faces the camera, tilted off the vertical: its normal is the
                // viewer's direction, its columns run along the horizontal and its rows up it.
                let n = camera.viewer();
                let norm = |v: [f32; 3]| {
                    let length = v.iter().map(|c| c * c).sum::<f32>().sqrt();
                    v.map(|c| c / length)
                };
                let u = norm([n[1], -n[0], 0.0]);
                let v = [
                    n[1] * u[2] - n[2] * u[1],
                    n[2] * u[0] - n[0] * u[2],
                    n[0] * u[1] - n[1] * u[0],
                ];
                let corner = |a: f32, b: f32| [0, 1, 2].map(|k| u[k] * a * 34.0 + v[k] * b * 20.0);
                frame.field(
                    &camera,
                    Placement::Corners([
                        corner(-1.0, -1.0),
                        corner(1.0, -1.0),
                        corner(1.0, 1.0),
                        corner(-1.0, 1.0),
                    ]),
                    field,
                    Relief::displaced(1.0),
                );
            }
        }
        frame
    }

    /// Runs `scripts/compare_png.py` on two frames' PNGs, which stay in `ISOTOP_PARITY_DIR` if it
    /// is set, and returns its one line of output; asserts that the frames are within tolerance.
    fn compared(name: &str, cpu: &Frame, hardware: &Frame) -> String {
        let directory = std::env::var_os("ISOTOP_PARITY_DIR").map_or_else(
            || std::env::temp_dir().join(format!("isotop-parity-{}", std::process::id())),
            std::path::PathBuf::from,
        );
        std::fs::create_dir_all(&directory).expect("the parity directory is created");
        let [cpu_png, gpu_png] =
            ["cpu", "gpu"].map(|backend| directory.join(format!("{name}-{backend}.png")));
        cpu.write_png(&cpu_png).expect("the CPU frame is written");
        hardware
            .write_png(&gpu_png)
            .expect("the GPU frame is written");
        let output = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/compare_png.py"
            ))
            .args([&cpu_png, &gpu_png])
            .output()
            .expect("python3 runs scripts/compare_png.py");
        let line = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if std::env::var_os("ISOTOP_PARITY_DIR").is_none() {
            std::fs::remove_dir_all(&directory).ok();
        }
        assert!(
            output.status.success(),
            "{name}: the backends differ beyond the tolerance: {line} {}",
            String::from_utf8_lossy(&output.stderr)
        );
        line
    }

    /// How many pixels of the pick window around `pointer` the two frames pick differently.
    fn differing_picks(cpu: &Frame, hardware: &Frame, pointer: [f32; 2]) -> (usize, usize) {
        let (w, h) = (cpu.width as usize, cpu.height as usize);
        let half = (PICK_WINDOW / 2) as usize;
        let (x0, y0) = (pointer[0] as usize - half, pointer[1] as usize - half);
        let mut differing = 0;
        let mut inside = 0;
        for y in y0..(y0 + 2 * half).min(h) {
            for x in x0..(x0 + 2 * half).min(w) {
                inside += 1;
                differing += usize::from(cpu.picks[y * w + x] != hardware.picks[y * w + x]);
            }
        }
        (differing, inside)
    }

    #[test]
    #[ignore = "needs a hardware GPU; run with --ignored on one"]
    fn field_draws_the_same_frame_on_cpu_and_gpu() {
        let mut gpu = Gpu::new(wgpu::PowerPreference::LowPower).expect("a hardware GPU");
        let pointer = [320.0, 190.0];
        let mut field = lumpy(11, 97, 65, 0.0);
        for (scene, name) in [(0, "flat"), (1, "relief"), (2, "corners")] {
            let mut cpu = field_frame(scene, &field);
            cpu.rasterize();
            let mut hardware = field_frame(scene, &field);
            gpu.render(&mut hardware, Some(pointer))
                .expect("the GPU renders");
            let line = compared(name, &cpu, &hardware);
            let (differing, inside) = differing_picks(&cpu, &hardware, pointer);
            println!(
                "field {name}: {line} | picks differ in {differing} of {inside} window pixels"
            );
            assert!(
                differing * 200 <= inside,
                "{name}: the owners differ in {differing} of {inside} pixels"
            );
        }
        // The same key with new data and a new revision is written again, not left stale; and a
        // second field beside it takes the slot after it.
        field = lumpy(11, 97, 65, 1.3);
        field.revision = 1;
        let mut cpu = field_frame(0, &field);
        cpu.rasterize();
        let mut hardware = field_frame(0, &field);
        gpu.render(&mut hardware, Some(pointer))
            .expect("the GPU renders");
        println!(
            "field rewritten: {}",
            compared("rewritten", &cpu, &hardware)
        );
        let beside = lumpy(12, 33, 21, 0.4);
        let draw = |relief: Relief| {
            let mut frame = field_frame(0, &field);
            frame.field(
                &Camera::default(),
                Placement::Grid {
                    grid: Grid {
                        origin: [20.0, 20.0],
                        cell: 1.0,
                        columns: 33,
                        rows: 21,
                    },
                    z: 4.0,
                },
                &beside,
                relief,
            );
            frame
        };
        let mut cpu = draw(Relief::Flat);
        cpu.rasterize();
        let mut hardware = draw(Relief::Flat);
        gpu.render(&mut hardware, Some(pointer))
            .expect("the GPU renders");
        println!("two fields: {}", compared("two", &cpu, &hardware));
        // A field too big for the storage buffer it was first given grows the buffer, which
        // then holds it and, once written again, the fields that came before.
        let big = lumpy(14, 300, 300, 0.2);
        assert!(big.columns * big.rows * 2 > FIELD_STORAGE_WORDS);
        let mut cpu = field_frame(1, &big);
        cpu.rasterize();
        let mut hardware = field_frame(1, &big);
        gpu.render(&mut hardware, Some(pointer))
            .expect("the GPU renders");
        println!("grown storage: {}", compared("grown", &cpu, &hardware));
        let mut cpu = field_frame(0, &field);
        cpu.rasterize();
        let mut hardware = field_frame(0, &field);
        gpu.render(&mut hardware, Some(pointer))
            .expect("the GPU renders");
        println!("after growth: {}", compared("after", &cpu, &hardware));
        // Without owners every pixel picks nothing, and without shading the colours are the
        // nodes' own.
        let mut plain = lumpy(13, 48, 36, 0.7);
        plain.owners = None;
        plain.shading = None;
        let mut cpu = field_frame(0, &plain);
        cpu.rasterize();
        let mut hardware = field_frame(0, &plain);
        gpu.render(&mut hardware, Some(pointer))
            .expect("the GPU renders");
        println!("no owners: {}", compared("plain", &cpu, &hardware));
        assert!(cpu.picks.iter().all(|&pick| pick == NONE));
        assert_eq!(differing_picks(&cpu, &hardware, pointer).0, 0);
    }
}
