//! The render pipelines of the GPU path, one per kind of draw call, built once with the device.

use std::mem::size_of;

use super::batch::Pass;
use super::{Ball, DEPTH, FieldVertex, Glow, Glyph, Splat, Vertex};

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const PICK: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// How a pipeline writes the targets.
#[derive(Clone, Copy)]
struct Style {
    /// Depth-tested, depth-writing and pickable; otherwise drawn over the scene without depth.
    solid: bool,
    blend: bool,
    /// Depth-tested like solid geometry, but writes only picks: no colour and no depth.
    pick_only: bool,
}

const SOLID: Style = Style {
    solid: true,
    blend: false,
    pick_only: false,
};
const BACKDROP: Style = Style {
    solid: false,
    blend: false,
    pick_only: false,
};
const TRANSLUCENT: Style = Style {
    solid: false,
    blend: true,
    pick_only: false,
};
const PICK_ONLY: Style = Style {
    solid: false,
    blend: false,
    pick_only: true,
};

/// The bind group layout every pipeline shares: the globals, and the storage buffer of fields.
pub(super) fn bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
    })
}

/// Every pipeline, by the kind of draw call it serves.
pub(super) struct Pipelines {
    sky: wgpu::RenderPipeline,
    twilight: wgpu::RenderPipeline,
    triangles: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    beams: wgpu::RenderPipeline,
    stars: wgpu::RenderPipeline,
    balls: wgpu::RenderPipeline,
    glows: wgpu::RenderPipeline,
    glyphs: wgpu::RenderPipeline,
    splats: wgpu::RenderPipeline,
    fields: wgpu::RenderPipeline,
}

impl Pipelines {
    pub(super) fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        bind_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("isotop"),
            bind_group_layouts: &[Some(bind_layout)],
            ..Default::default()
        });
        let vertex = wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Unorm8x4, 3 => Uint32],
        };
        let ball = wgpu::VertexBufferLayout {
            array_stride: size_of::<Ball>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32, 3 => Unorm8x4, 4 => Uint32, 5 => Uint32],
        };
        let glow = wgpu::VertexBufferLayout {
            array_stride: size_of::<Glow>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Unorm8x4],
        };
        let glyph = wgpu::VertexBufferLayout {
            array_stride: size_of::<Glyph>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Unorm8x4, 3 => Uint32x4],
        };
        let splat = wgpu::VertexBufferLayout {
            array_stride: size_of::<Splat>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Uint32],
        };
        let field = wgpu::VertexBufferLayout {
            array_stride: size_of::<FieldVertex>() as u64,
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
                    module: shader,
                    entry_point: Some(entry.0),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
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
        use wgpu::PrimitiveTopology::{LineList, PointList, TriangleList};
        let geometry = ("geometry_vertex", "geometry_fragment");
        Self {
            sky: make(("sky_vertex", "sky_fragment"), &[], TriangleList, BACKDROP),
            twilight: make(
                ("sky_vertex", "twilight_fragment"),
                &[],
                TriangleList,
                BACKDROP,
            ),
            triangles: make(geometry, &[Some(vertex.clone())], TriangleList, SOLID),
            lines: make(geometry, &[Some(vertex.clone())], LineList, SOLID),
            beams: make(geometry, &[Some(vertex.clone())], LineList, TRANSLUCENT),
            stars: make(geometry, &[Some(vertex)], PointList, BACKDROP),
            balls: make(
                ("ball_vertex", "ball_fragment"),
                &[Some(ball)],
                TriangleList,
                SOLID,
            ),
            glows: make(
                ("glow_vertex", "glow_fragment"),
                &[Some(glow)],
                TriangleList,
                TRANSLUCENT,
            ),
            glyphs: make(
                ("glyph_vertex", "glyph_fragment"),
                &[Some(glyph)],
                TriangleList,
                BACKDROP,
            ),
            splats: make(
                ("splat_vertex", "splat_fragment"),
                &[Some(splat)],
                TriangleList,
                PICK_ONLY,
            ),
            fields: make(
                ("field_vertex", "field_fragment"),
                &[Some(field)],
                TriangleList,
                SOLID,
            ),
        }
    }

    /// The sky's pipeline: the twilight one has its own entry point.
    pub(super) fn sky(&self, twilight: bool) -> &wgpu::RenderPipeline {
        if twilight { &self.twilight } else { &self.sky }
    }

    pub(super) fn of(&self, pass: Pass) -> &wgpu::RenderPipeline {
        match pass {
            Pass::Triangles => &self.triangles,
            Pass::Lines => &self.lines,
            Pass::Beams => &self.beams,
            Pass::Stars => &self.stars,
            Pass::Balls => &self.balls,
            Pass::Glows => &self.glows,
            Pass::Glyphs => &self.glyphs,
            Pass::Splats => &self.splats,
            Pass::Fields => &self.fields,
        }
    }
}
