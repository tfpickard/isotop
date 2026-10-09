//! Hardware rasterizer for the display list through wgpu: Vulkan on Linux (NVIDIA, AMD, Intel),
//! Metal on Apple GPUs, DX12 on Windows. It renders colour, picking and depth like the CPU
//! rasterizer and reads colour and picking back for the terminal and mouse hit-testing.

use std::ops::Range;
use std::sync::{Arc, Mutex, mpsc};

use bytemuck::{Pod, Zeroable};

use crate::render::{Color, Frame, Item, NONE};

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pass {
    Triangles,
    Lines,
    Beams,
    Stars,
    Balls,
    Glows,
    Glyphs,
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

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub name: String,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    sky: wgpu::RenderPipeline,
    pipelines: [(Pass, wgpu::RenderPipeline); 7],
    targets: Option<Targets>,
    streams: [Stream; 4],
    scratch: Vec<u8>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Gpu {
    /// Opens the preferred hardware adapter. Software adapters are refused: the CPU rasterizer
    /// is faster than an emulated GPU.
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
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
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
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: PICK,
                            blend: None,
                            write_mask: if style.solid {
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
                    depth_compare: Some(if style.solid {
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
        };
        let backdrop = Style {
            solid: false,
            blend: false,
        };
        let translucent = Style {
            solid: false,
            blend: true,
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
        ];
        let sky = make(("sky_vertex", "sky_fragment"), &[], TriangleList, backdrop);
        if let Some(error) = failure.lock().ok().and_then(|mut slot| slot.take()) {
            return Err(error);
        }
        Ok(Self {
            name: format!("{} ({:?})", info.name, info.backend),
            device,
            queue,
            globals,
            bind_group,
            sky,
            pipelines,
            targets: None,
            streams: Default::default(),
            scratch: Vec::new(),
            failure,
        })
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
    pub fn render(&mut self, frame: &mut Frame, pointer: Option<[f32; 2]>) -> Result<(), String> {
        let (width, height) = (frame.width, frame.height);
        let mut vertices = Vec::with_capacity(frame.items.len() * 2);
        let mut balls = Vec::new();
        let mut glows = Vec::new();
        let mut glyphs = Vec::new();
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
                Item::Glow { .. } => (Pass::Glows, glows.len()),
                Item::Glyph { .. } => (Pass::Glyphs, glyphs.len()),
                Item::Triangle(..) => (Pass::Triangles, vertices.len()),
                Item::Line(..) => (Pass::Lines, vertices.len()),
                Item::Beam(..) => (Pass::Beams, vertices.len()),
                Item::Star(..) => (Pass::Stars, vertices.len()),
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
                        track(p[2] + 0.02);
                        vertices.push(Vertex {
                            position: [p[0], p[1]],
                            depth: p[2] + 0.02,
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
                Item::Glow {
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
            }
            let end = match pass {
                Pass::Balls => balls.len(),
                Pass::Glows => glows.len(),
                Pass::Glyphs => glyphs.len(),
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
        let [vertex_stream, ball_stream, glow_stream, glyph_stream] = &mut self.streams;
        let vertex_buffer =
            vertex_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&vertices));
        let ball_buffer =
            ball_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&balls));
        let glow_buffer =
            glow_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&glows));
        let glyph_buffer =
            glyph_stream.upload(&self.device, &self.queue, bytemuck::cast_slice(&glyphs));
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
            pass.set_pipeline(&self.sky);
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
                    Pass::Balls | Pass::Glows | Pass::Glyphs => {
                        let buffer = match kind {
                            Pass::Balls => &ball_buffer,
                            Pass::Glows => &glow_buffer,
                            _ => &glyph_buffer,
                        };
                        if let Some(buffer) = buffer {
                            pass.set_vertex_buffer(0, buffer.slice(..));
                            pass.draw(0..6, range);
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
                    .chunks_exact_mut(3)
                    .zip(colors.chunks_exact(4))
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
                        .zip(source.chunks_exact(4))
                    {
                        *pick = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
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
