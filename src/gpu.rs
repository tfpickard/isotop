//! Hardware rasterizer for the display list through wgpu: Vulkan on Linux (NVIDIA, AMD, Intel),
//! Metal on Apple GPUs, DX12 on Windows. It renders colour, picking and depth like the CPU
//! rasterizer and reads colour and picking back for the terminal and mouse hit-testing.
//!
//! A frame goes through `Gpu::submit`, which stages the display list (`staging`, in the draw
//! calls `batch` plans: painter's order kept exactly, items that commute joining one call) and
//! starts the readback (`readback`), and `Gpu::collect`, which waits for the frame and fills its
//! pixels and picks. `Gpu::render` is both at once; `Renderer::pipelined` collects a frame after
//! it has submitted the next.

mod batch;
#[cfg(test)]
mod frames;
mod pipelines;
mod readback;
mod staging;

use std::mem::{offset_of, size_of};
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};

use crate::render::field::StorageLayout;
use crate::render::shared::{self, NONE};
use crate::render::{Backdrop, Frame};

pub(crate) use batch::Order;
use batch::Pass;
pub(crate) use readback::{Flight, Picks};
use readback::{Layout, Slots};
use staging::Staging;

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

/// The environment variable that lets `Gpu::new` take a software adapter (and only one); a live
/// session (`Gpu::hardware`) ignores it.
const SOFTWARE_OPT_IN: &str = "ISOTOP_ALLOW_SOFTWARE_GPU";

/// Whether the opt-in is on: exactly `1`. Anything else, or unset, leaves the choice unchanged.
fn software_opt_in(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether an adapter of this type may be used: without the opt-in every type but a CPU one,
/// with it a CPU one only.
fn accepts(device_type: wgpu::DeviceType, software: bool) -> bool {
    (device_type == wgpu::DeviceType::Cpu) == software
}

/// The shader source: the generated constants from `render::shared`, then `shaders.wgsl`.
fn shader_source() -> String {
    format!("{}{}", shared::prelude(), include_str!("shaders.wgsl"))
}

/// The textures a frame is drawn into, which are made again only when the frame's size changes.
struct Targets {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    pick: wgpu::Texture,
    color_view: wgpu::TextureView,
    pick_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
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
        Self {
            width,
            height,
            color_view: color.create_view(&Default::default()),
            pick_view: pick.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            color,
            pick,
        }
    }
}

/// A vertex or instance buffer reused across frames and grown when a frame needs more room.
#[derive(Default)]
struct Stream {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    /// How many buffers this stream has made, one for each time it outgrew the last.
    created: u64,
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
            self.created += 1;
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

/// The six buffers a frame's items are uploaded to.
#[derive(Default)]
struct Streams {
    vertices: Stream,
    balls: Stream,
    glows: Stream,
    glyphs: Stream,
    splats: Stream,
    fields: Stream,
}

/// The buffers a frame was uploaded to, for the draw calls to bind.
struct Uploaded {
    vertices: Option<wgpu::Buffer>,
    balls: Option<wgpu::Buffer>,
    glows: Option<wgpu::Buffer>,
    glyphs: Option<wgpu::Buffer>,
    splats: Option<wgpu::Buffer>,
    fields: Option<wgpu::Buffer>,
}

impl Uploaded {
    /// The buffer `pass` draws from, and which of the six it is.
    fn of(&self, pass: Pass) -> (usize, Option<&wgpu::Buffer>) {
        match pass {
            Pass::Balls => (1, self.balls.as_ref()),
            Pass::Glows => (2, self.glows.as_ref()),
            Pass::Glyphs => (3, self.glyphs.as_ref()),
            Pass::Splats => (4, self.splats.as_ref()),
            Pass::Fields => (5, self.fields.as_ref()),
            Pass::Triangles | Pass::Lines | Pass::Beams | Pass::Stars => {
                (0, self.vertices.as_ref())
            }
        }
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

/// What the last frame submitted cost in draw calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Items in the display list.
    pub(crate) items: usize,
    /// Draw calls the frame was made in, the sky's among them.
    pub(crate) draw_calls: usize,
    /// Draw calls it would have taken drawn exactly as recorded, with only adjacent items of one
    /// kind sharing one.
    pub(crate) recorded_draw_calls: usize,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub name: String,
    globals: wgpu::Buffer,
    bind_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    fields: FieldStorage,
    /// How many times the field storage buffer was made again.
    fields_grown: u64,
    pipelines: pipelines::Pipelines,
    targets: Option<Targets>,
    streams: Streams,
    staging: Staging,
    slots: Slots,
    scratch: Vec<u8>,
    failure: Arc<Mutex<Option<String>>>,
    order: Order,
    stats: Stats,
}

impl Gpu {
    /// Opens the preferred hardware adapter. Software adapters are refused: the CPU rasterizer
    /// is faster than an emulated GPU. `ISOTOP_ALLOW_SOFTWARE_GPU=1` is the one exception, for a
    /// parity run on a machine with no GPU (CI's lavapipe): it asks wgpu for its fallback adapter
    /// only and accepts nothing but a CPU-type one, so it cannot silently measure hardware. It is
    /// for headless runs and tests; a live session opens its adapter with `hardware`.
    pub fn new(power: wgpu::PowerPreference) -> Result<Self, String> {
        let software = software_opt_in(std::env::var(SOFTWARE_OPT_IN).ok().as_deref());
        Self::open(power, software)
    }

    /// Opens the preferred hardware adapter whatever `ISOTOP_ALLOW_SOFTWARE_GPU` says: an
    /// interactive run never takes a software adapter, which would be slower than the CPU
    /// rasterizer.
    pub fn hardware(power: wgpu::PowerPreference) -> Result<Self, String> {
        Self::open(power, false)
    }

    fn open(power: wgpu::PowerPreference, software: bool) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: power,
            force_fallback_adapter: software,
            ..Default::default()
        }))
        .map_err(|error| format!("no GPU adapter: {error}"))?;
        let info = adapter.get_info();
        if !accepts(info.device_type, software) {
            return Err(if software {
                format!("{} is not a software adapter", info.name)
            } else {
                format!("{} is a software adapter", info.name)
            });
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
        let layout = pipelines::bind_layout(&device);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fields = FieldStorage::new(&device, FIELD_STORAGE_WORDS);
        let bind_group = bind_group(&device, &layout, &globals, &fields.buffer);
        let pipelines = pipelines::Pipelines::new(&device, &shader, &layout);
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
            fields_grown: 0,
            pipelines,
            targets: None,
            streams: Streams::default(),
            staging: Staging::default(),
            slots: Slots::default(),
            scratch: Vec::new(),
            failure,
            order: Order::Commuting,
            stats: Stats::default(),
        })
    }

    /// How the draw calls of a frame are laid out: `Commuting` unless a test or a measurement
    /// asks for the recorded order, which draws the same frame in more calls.
    #[cfg(test)]
    pub(crate) fn set_order(&mut self, order: Order) {
        self.order = order;
    }

    /// The draw calls of the last frame submitted.
    pub(crate) fn stats(&self) -> Stats {
        self.stats
    }

    /// How many GPU buffers have been made since the start: vertex and instance streams, readback
    /// buffers and the fields' storage. It stops growing once the frames have stopped growing.
    #[cfg(test)]
    pub(crate) fn buffers_created(&self) -> u64 {
        let streams = &self.streams;
        [
            &streams.vertices,
            &streams.balls,
            &streams.glows,
            &streams.glyphs,
            &streams.splats,
            &streams.fields,
        ]
        .iter()
        .map(|stream| stream.created)
        .sum::<u64>()
            + self.slots.created
            + self.fields_grown
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
            self.fields_grown += 1;
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

    /// Writes the globals and the staged items to the GPU, growing a buffer that is too small.
    fn upload(&mut self, frame: &Frame) -> Uploaded {
        let depth = self.staging.depth;
        let sky = frame.sky;
        let globals = Globals {
            size: [frame.width as f32, frame.height as f32],
            depth: [depth[0], 1.0 / (depth[1] - depth[0]).max(1e-3)],
            haze: [sky.haze[0], sky.haze[1], sky.haze[2], sky.time],
            sky: [sky.backdrop as u32 as f32, 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let (device, queue) = (&self.device, &self.queue);
        let (streams, staging) = (&mut self.streams, &self.staging);
        Uploaded {
            vertices: streams.vertices.upload(
                device,
                queue,
                bytemuck::cast_slice(&staging.vertices),
            ),
            balls: streams
                .balls
                .upload(device, queue, bytemuck::cast_slice(&staging.balls)),
            glows: streams
                .glows
                .upload(device, queue, bytemuck::cast_slice(&staging.glows)),
            glyphs: streams
                .glyphs
                .upload(device, queue, bytemuck::cast_slice(&staging.glyphs)),
            splats: streams
                .splats
                .upload(device, queue, bytemuck::cast_slice(&staging.splats)),
            fields: streams.fields.upload(
                device,
                queue,
                bytemuck::cast_slice(&staging.field_vertices),
            ),
        }
    }

    /// Records the frame's one render pass: the sky, then each draw call in order.
    fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        uploaded: &Uploaded,
        backdrop: Backdrop,
    ) {
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
        pass.set_pipeline(self.pipelines.sky(backdrop == Backdrop::Twilight));
        pass.draw(0..3, 0..1);
        // A buffer stays bound across pipelines, so it is bound again only when the draw calls
        // move to another of the six.
        let mut bound = None;
        for draw in &self.staging.draws {
            let (stream, buffer) = uploaded.of(draw.pass);
            let Some(buffer) = buffer else { continue };
            pass.set_pipeline(self.pipelines.of(draw.pass));
            if bound != Some(stream) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                bound = Some(stream);
            }
            match draw.pass {
                Pass::Balls | Pass::Glows | Pass::Glyphs | Pass::Splats => {
                    pass.draw(0..6, draw.range.clone());
                }
                _ => pass.draw(draw.range.clone(), 0..1),
            }
        }
    }

    /// Draws the frame's display list on the GPU and starts copying its colour, and its picks as
    /// `picks` says, into a buffer; the `Flight` it returns is `collect`ed to fill the frame.
    /// Fails if two frames are in flight already.
    pub(crate) fn submit(&mut self, frame: &Frame, picks: Picks) -> Result<Flight, String> {
        let (width, height) = (frame.width, frame.height);
        let plan = self.store_fields(frame)?;
        self.staging.stage(frame, &plan.offsets, self.order);
        self.stats = Stats {
            items: frame.items.len(),
            draw_calls: self.staging.draws.len() + 1,
            recorded_draw_calls: self.staging.planner.recorded_runs + 1,
        };
        let uploaded = self.upload(frame);
        if self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            self.targets = Some(Targets::new(&self.device, width, height));
        }
        let layout = Layout::new(width, height, picks);
        let (buffer, busy) = self.slots.acquire(&self.device, layout.bytes)?;
        let targets = self.targets.as_ref().expect("targets exist");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.draw(&mut encoder, targets, &uploaded, frame.sky.backdrop);
        let mut copies = vec![(&targets.color, [0, 0], [width, height], 0, layout.row)];
        if layout.pick_size[1] > 0 {
            copies.push((
                &targets.pick,
                layout.pick_origin,
                layout.pick_size,
                layout.pick_offset,
                layout.pick_row,
            ));
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
                    buffer: &buffer,
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
        let submission = self.queue.submit(Some(encoder.finish()));
        let flight = Flight::new(self.device.clone(), buffer, busy, submission, layout);
        self.failed()?;
        Ok(flight)
    }

    /// Waits for `flight`'s frame, which `submit` drew from `frame` or from one like it, and fills
    /// `frame`'s pixels and picks from it.
    pub(crate) fn collect(&mut self, flight: Flight, frame: &mut Frame) -> Result<(), String> {
        flight.finish(frame, &mut self.scratch)?;
        self.failed()
    }

    /// Rasterizes the frame's display list into its pixels and the picks `picks` asks for, and
    /// returns when they are there.
    pub(crate) fn render(&mut self, frame: &mut Frame, picks: Picks) -> Result<(), String> {
        let flight = self.submit(frame, picks)?;
        self.collect(flight, frame)
    }

    /// The first error the device reported on its own since the last call, if any.
    fn failed(&self) -> Result<(), String> {
        match self.failure.lock().ok().and_then(|mut slot| slot.take()) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_software_opt_in_is_exactly_one_and_flips_which_adapters_are_accepted() {
        use super::{accepts, software_opt_in};
        use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu};
        assert!(software_opt_in(Some("1")));
        for other in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!software_opt_in(other));
        }
        assert!(accepts(DiscreteGpu, false) && accepts(IntegratedGpu, false));
        assert!(!accepts(Cpu, false));
        assert!(accepts(Cpu, true));
        assert!(!accepts(DiscreteGpu, true) && !accepts(IntegratedGpu, true));
    }

    use super::readback::PICK_WINDOW;
    use super::*;
    use crate::medium::Grid;
    use crate::model::Identity;
    use crate::render::{
        Backdrop, Buffers, Camera, Field, Item, Placement, Relief, Shading, Sky, Target, Thing,
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
    #[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
    fn stacked_puffs_match_the_gpu_to_a_level_however_densely_they_stack() {
        let mut gpu = Gpu::new(wgpu::PowerPreference::LowPower)
            .expect("a GPU (hardware, or lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1)");
        for (count, spread, radius) in [
            (1800, [50.0, 35.0], 2.25),
            (8000, [80.0, 50.0], 2.25),
            (8000, [80.0, 50.0], 4.0),
        ] {
            let mut cpu = clump(count, spread, radius, true);
            cpu.rasterize();
            let mut hardware = clump(count, spread, radius, true);
            gpu.render(&mut hardware, Picks::Nowhere)
                .expect("the GPU renders");
            let (share, largest) = disagreement(&cpu, &hardware);
            assert!(
                share == 0.0 && largest <= 1,
                "{count} puffs of radius {radius}: {share} % of pixels over 2 levels, largest {largest}"
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
    fn pick_splats_and_thing_picks_survive_the_gpu_readback() {
        let mut gpu = Gpu::new(wgpu::PowerPreference::LowPower)
            .expect("a GPU (hardware, or lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1)");
        let mut expected = picks_frame();
        expected.rasterize();
        let mut frame = picks_frame();
        gpu.render(&mut frame, Picks::Around([60.0, 40.0]))
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
    #[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
    fn field_draws_the_same_frame_on_cpu_and_gpu() {
        let mut gpu = Gpu::new(wgpu::PowerPreference::LowPower)
            .expect("a GPU (hardware, or lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1)");
        let pointer = [320.0, 190.0];
        let mut field = lumpy(11, 97, 65, 0.0);
        for (scene, name) in [(0, "flat"), (1, "relief"), (2, "corners")] {
            let mut cpu = field_frame(scene, &field);
            cpu.rasterize();
            let mut hardware = field_frame(scene, &field);
            gpu.render(&mut hardware, Picks::Around(pointer))
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
        gpu.render(&mut hardware, Picks::Around(pointer))
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
        gpu.render(&mut hardware, Picks::Around(pointer))
            .expect("the GPU renders");
        println!("two fields: {}", compared("two", &cpu, &hardware));
        // A field too big for the storage buffer it was first given grows the buffer, which
        // then holds it and, once written again, the fields that came before.
        let big = lumpy(14, 300, 300, 0.2);
        assert!(big.columns * big.rows * 2 > FIELD_STORAGE_WORDS);
        let mut cpu = field_frame(1, &big);
        cpu.rasterize();
        let mut hardware = field_frame(1, &big);
        gpu.render(&mut hardware, Picks::Around(pointer))
            .expect("the GPU renders");
        println!("grown storage: {}", compared("grown", &cpu, &hardware));
        let mut cpu = field_frame(0, &field);
        cpu.rasterize();
        let mut hardware = field_frame(0, &field);
        gpu.render(&mut hardware, Picks::Around(pointer))
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
        gpu.render(&mut hardware, Picks::Around(pointer))
            .expect("the GPU renders");
        println!("no owners: {}", compared("plain", &cpu, &hardware));
        assert!(cpu.picks.iter().all(|&pick| pick == NONE));
        assert_eq!(differing_picks(&cpu, &hardware, pointer).0, 0);
    }
}
