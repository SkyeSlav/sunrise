//! Mods' render pipelines on the GPU (`Hooks::register_render_pipelines`): the pipelines built
//! from a resource load's translated GLSL core shaders (`crate::mod_pipelines`), and the quads
//! a frame draws with them. They draw with the world primitives, at their stages and into the
//! same targets, so they take the world's reversed depth, its draw resolution and TAA's jitter.
//! Pipelines are built with the renderer a load is applied with
//! ([`super::Renderer::build_mod_pipelines`]), never while quads are being submitted, so a
//! required pipeline the GPU refuses fails that load's renderer and the reload with it.
//!
//! Each pipeline binds vanilla's four std140 uniform blocks (`Globals` 0, `Projection` 1,
//! `DynamicTransforms` 2, `Fog` 3), plus `Sampler0` (4/5) and `SpriteRect` (10) when
//! `.textured(PipelineTexture::Sprite(_))` is declared, only for the bindings its translated
//! stages actually read.

use super::expression_blocks::{VANILLA_OFFSETS, VANILLA_SIZES, block_entry};
use super::mod_meshes::{ResolvedMeshDraw, wgpu_index_format};
use super::world_primitives::{Upload, VertexPages};
use crate::mod_pipelines::{Built, BuiltTexture, ModifierStacks, Pipelines};
use glam::Mat4;
use mod_api::{
    BindingKind, BlendFactor, BlendOp, ColorTargetState, ColorWrite, DepthCompare, DrawRange,
    Identifier, IndexFormat, MeshDraw, PipelineBlend, PipelineState, PolygonMode,
    PrimitiveTopology, SamplerAddress, SamplerFilter, VertexAttributeFormat, VertexFormat,
    VertexStepMode,
};
use std::collections::HashMap;
use sunrise_mod_pipeline::{
    ENTRY, OIT_STAGE_SAMPLERS, Options as PipelineOptions, SPRITE_RECT_BINDING,
};

/// One corner of a pipeline quad, camera-relative.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PipelineVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// Red in the low byte, then green, blue and alpha, as vanilla encodes them.
    pub color: u32,
}

impl PipelineVertex {
    /// Vertex attributes matching `PipelineVertex` (`pos`, `uv`, `color`).
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Unorm8x4];

    /// Vertex buffer layout shared by `ModPipelines` and translated shader-pack mod passes.
    pub const fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// Quads that follow each other in a frame's vertices and draw with one pipeline at one stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineRun {
    /// The pipeline's slot in the frame's [`Pipelines`].
    pub slot: u32,
    /// The stage it draws at (`STAGE_AFTER_OPAQUE_BLOCKS..=STAGE_CUSTOM_GEOMETRY`, or OIT stage
    /// `0..=2` for `Batch::oit_runs`).
    pub stage: u32,
    pub quads: u32,
    /// Interned modifier stack id (`0` = no active modifier).
    pub modifier_stack: u32,
}

/// Source of vertex and optional index buffers for one unified [`PipelineDraw`].
#[derive(Clone, Debug)]
pub enum PipelineDrawSource {
    /// Transient per-frame byte ranges inside `Batch::transient_vertices` and
    /// `Batch::transient_indices` (used by `draw_vertices` and the `pipeline_quads` shorthand).
    Transient {
        /// `(binding_index, byte_range_in_transient_vertices)`
        vertex_ranges: Vec<(u32, std::ops::Range<u64>)>,
        /// Optional `(index_format, byte_range_in_transient_indices)`
        index_range: Option<(IndexFormat, std::ops::Range<u64>)>,
    },
    /// Zero-copy retained GPU buffers resolved from a [`mod_api::MeshRef`].
    Retained(ResolvedMeshDraw),
}

/// One resolved set-1 binding entry for a [`PipelineDraw`] (`#5428`).
#[derive(Clone, Debug)]
pub enum ResolvedSet1Entry {
    /// Retained GPU uniform buffer (`UniformBufferRef`).
    UniformBuffer {
        id: u64,
        buffer: wgpu::Buffer,
        byte_len: u64,
    },
    /// Inline std140 uniform byte range in `Batch::inline_uniforms` / `Batch::oit_inline_uniforms`.
    InlineUniform { byte_range: std::ops::Range<usize> },
    /// Resolved 2D texture view with a unique cache key id.
    Texture {
        view_id: u64,
        view: wgpu::TextureView,
    },
    /// Expanded `i32`/`u32` texel buffer byte range in `Batch::texel_data` / `Batch::oit_texel_data`.
    TexelBuffer { byte_range: std::ops::Range<usize> },
    /// Transient frame-graph target colour attachment, resolved to a pooled GPU texture view
    /// right before executing the pass in [`super::mod_frame_graph`].
    TransientColor {
        target: mod_api::TransientTargetId,
        attachment: u8,
    },
    /// Transient frame-graph target depth attachment, resolved to a pooled GPU texture view
    /// right before executing the pass in [`super::mod_frame_graph`].
    TransientDepth { target: mod_api::TransientTargetId },
}

/// Resolved set-1 resource bindings for one [`PipelineDraw`] in exact declaration order.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDrawBindings {
    pub entries: Vec<ResolvedSet1Entry>,
    /// Optional push-constant byte range in `Batch::inline_uniforms` / `Batch::oit_inline_uniforms`.
    pub push_constants: Option<std::ops::Range<usize>>,
}

/// One unified mod-pipeline draw command (`draw_mesh`, `draw_vertices`, or `pipeline_quads`).
#[derive(Clone, Debug)]
pub struct PipelineDraw {
    /// The pipeline's slot in the frame's [`Pipelines`].
    pub slot: u32,
    /// The stage it draws at (`STAGE_AFTER_OPAQUE_BLOCKS..=STAGE_CUSTOM_GEOMETRY`, or OIT
    /// stage `0..=2` for `Batch::oit_draws`).
    pub stage: u32,
    /// Interned modifier stack id (`0` = no active modifier).
    pub modifier_stack: u32,
    /// Vertex/index range and instance range.
    pub draw: MeshDraw,
    /// Transient byte ranges or zero-copy retained GPU buffers.
    pub source: PipelineDrawSource,
    /// Resolved set-1 resource bindings (`None` when the pipeline has no set-1 layout or push
    /// constants).
    pub bindings: Option<ResolvedDrawBindings>,
}

/// A frame's pipeline draws, as the game queued them: legacy quad runs plus unified
/// `draw_vertices` / `draw_mesh` commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct Batch<'a> {
    /// The pipelines of the resources the frame draws with; `None` draws nothing.
    pub set: Option<&'a Pipelines>,
    pub vertices: &'a [PipelineVertex],
    pub runs: &'a [PipelineRun],
    /// Unified `draw_vertices` / `draw_mesh` / `pipeline_quads` commands for standard stages.
    pub draws: &'a [PipelineDraw],
    /// Transient per-frame vertex byte pool referenced by `draws`.
    pub transient_vertices: &'a [u8],
    /// Transient per-frame index byte pool referenced by `draws`.
    pub transient_indices: &'a [u8],
    /// Per-frame inline uniform & push-constant byte pool referenced by `draws`.
    pub inline_uniforms: &'a [u8],
    /// Per-frame expanded texel-buffer byte pool referenced by `draws`.
    pub texel_data: &'a [u8],
    /// Interned modifier stacks referenced by `runs[*].modifier_stack` and
    /// `draws[*].modifier_stack`; `None` means empty.
    pub modifiers: Option<&'a ModifierStacks>,
    /// Modifier definitions for `modifiers`; `None` reads `registered_modifiers()`.
    pub modifier_defs: Option<&'a [mod_api::PipelineModifier]>,
    /// Order-independent transparency pipeline quad vertices and runs (`OitTranslucent`).
    pub oit_vertices: &'a [PipelineVertex],
    pub oit_runs: &'a [PipelineRun],
    /// Unified `draw_vertices` / `draw_mesh` / `pipeline_quads` commands for OIT stages.
    pub oit_draws: &'a [PipelineDraw],
    /// Transient per-frame vertex byte pool referenced by `oit_draws`.
    pub oit_transient_vertices: &'a [u8],
    /// Transient per-frame index byte pool referenced by `oit_draws`.
    pub oit_transient_indices: &'a [u8],
    /// Per-frame inline uniform & push-constant byte pool referenced by `oit_draws`.
    pub oit_inline_uniforms: &'a [u8],
    /// Per-frame expanded texel-buffer byte pool referenced by `oit_draws`.
    pub oit_texel_data: &'a [u8],
    /// Seconds of game time (fallback `Globals.GameTime` when no full vanilla block is supplied).
    pub time: f32,
}

/// A run as uploaded: where its quads start in the vertex buffer.
struct Run {
    slot: usize,
    stage: usize,
    quads: std::ops::Range<u32>,
    modifier_stack: u32,
}

/// A unified draw command as prepared for the GPU render pass (zero-copy for retained meshes).
struct PreparedDraw {
    slot: usize,
    stage: usize,
    modifier_stack: u32,
    topology: PrimitiveTopology,
    draw: MeshDraw,
    source: PipelineDrawSource,
    set1_bind_group: Option<(wgpu::BindGroup, Vec<u32>)>,
}

/// Where textured pipelines' textures come from when the renderer builds them
/// ([`mod_api::RenderPipeline::textured`]).
#[derive(Clone, Copy)]
pub struct TextureSources<'a> {
    /// The 2D atlas the blocks and items atlases' sprites are drawn from.
    pub atlas: &'a wgpu::TextureView,
    /// A sprite's `[u0, v0, u1, v1]` in `atlas`, by its name there; `None` for no such sprite.
    pub sprite: &'a dyn Fn(&str) -> Option<[f32; 4]>,
}

/// Clip-space transform from Sunrise's world target (screen rows, reversed-Z `[1..0]`) to a
/// shader pack's `colortex0` + `depthtex0` (GL rows, GL `[0..1]` depth): maps `(x, y, z, w)`
/// to `(x, -y, w - z, w)` so `ndc_y' = -ndc_y` and `ndc_z' = 1 - ndc_z`.
fn pack_clip_matrix(view_proj: Mat4) -> Mat4 {
    let adapt = Mat4::from_cols_array(&[
        1.0, 0.0, 0.0, 0.0, //
        0.0, -1.0, 0.0, 0.0, //
        0.0, 0.0, -1.0, 0.0, //
        0.0, 0.0, 1.0, 1.0,
    ]);
    adapt * view_proj
}

/// Rewrite the `Projection` matrix (at byte offset 256..320) inside a `core_lines::uniforms`
/// 1024-byte vanilla block buffer with `pack_clip_matrix(projection)`.
fn pack_vanilla_blocks(vanilla: &[u8; 1024]) -> [u8; 1024] {
    let mut out = *vanilla;
    let proj_floats: [f32; 16] = bytemuck::pod_read_unaligned(&vanilla[256..320]);
    let adapted = pack_clip_matrix(Mat4::from_cols_array(&proj_floats));
    out[256..320].copy_from_slice(bytemuck::cast_slice(&adapted.to_cols_array()));
    out
}

/// Construct a default 1024-byte vanilla uniform buffer (`Globals`, `Projection = view_proj`,
/// `DynamicTransforms` with identity `ModelViewMat`/`TextureMat` and white `ColorModulator`,
/// zero `Fog`) for callers that have only `(view_proj, viewport, time)` such as standalone tests.
pub(crate) fn fallback_vanilla_blocks(
    view_proj: Mat4,
    viewport: [u32; 2],
    time: f32,
) -> [u8; 1024] {
    let mut data = [0f32; 256];
    // Globals (offset 0, 48 bytes): CameraBlockPos(0..3), GlintAlpha(3), CameraOffset(4..7),
    // GameTime(7), ScreenSize(8..10), MenuBlurRadius(10), UseRgss(11).
    data[7] = time;
    data[8] = viewport[0] as f32;
    data[9] = viewport[1] as f32;
    // Projection (offset 256 -> float 64): ProjMat.
    data[64..80].copy_from_slice(&view_proj.to_cols_array());
    // DynamicTransforms (offset 512 -> float 128): ModelViewMat, TextureMat, ColorModulator, ModelOffset.
    data[128..144].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
    data[144..160].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
    data[160..164].fill(1.0);
    let mut bytes = [0u8; 1024];
    bytes.copy_from_slice(bytemuck::cast_slice(&data));
    bytes
}

/// Per-pass draw payload for [`ModPipelines::execute_offscreen_pass`].
pub struct OffscreenPassData<'a> {
    pub pass: &'a mod_api::OffscreenPass,
    pub target: &'a super::mod_render_targets::ResolvedOffscreenTarget,
    pub draws: &'a [PipelineDraw],
    pub transient_vertices: &'a [u8],
    pub transient_indices: &'a [u8],
    pub inline_uniforms: &'a [u8],
    pub texel_data: &'a [u8],
}

/// Target attachment and Z-direction state for [`ModPipelines::build_render_pipeline`].
#[derive(Clone, Copy)]
struct TargetState<'a> {
    color_formats: &'a [wgpu::TextureFormat],
    depth_format: Option<wgpu::TextureFormat>,
    force_depth_attachment: bool,
    standard_z: bool,
    write_mask: wgpu::ColorWrites,
}

fn wgpu_compare(compare: DepthCompare) -> wgpu::CompareFunction {
    match compare {
        DepthCompare::Always => wgpu::CompareFunction::Always,
        DepthCompare::Less => wgpu::CompareFunction::Less,
        DepthCompare::LessEqual => wgpu::CompareFunction::LessEqual,
        DepthCompare::Equal => wgpu::CompareFunction::Equal,
        DepthCompare::NotEqual => wgpu::CompareFunction::NotEqual,
        DepthCompare::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
        DepthCompare::Greater => wgpu::CompareFunction::Greater,
        DepthCompare::Never => wgpu::CompareFunction::Never,
        _ => wgpu::CompareFunction::GreaterEqual,
    }
}

fn wgpu_blend_factor(factor: BlendFactor) -> wgpu::BlendFactor {
    match factor {
        BlendFactor::Zero => wgpu::BlendFactor::Zero,
        BlendFactor::One => wgpu::BlendFactor::One,
        BlendFactor::SrcColor => wgpu::BlendFactor::Src,
        BlendFactor::OneMinusSrcColor => wgpu::BlendFactor::OneMinusSrc,
        BlendFactor::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        BlendFactor::DstColor => wgpu::BlendFactor::Dst,
        BlendFactor::OneMinusDstColor => wgpu::BlendFactor::OneMinusDst,
        BlendFactor::DstAlpha => wgpu::BlendFactor::DstAlpha,
        BlendFactor::OneMinusDstAlpha => wgpu::BlendFactor::OneMinusDstAlpha,
        BlendFactor::SrcAlphaSaturate => wgpu::BlendFactor::SrcAlphaSaturated,
        BlendFactor::ConstantColor | BlendFactor::ConstantAlpha => wgpu::BlendFactor::Constant,
        BlendFactor::OneMinusConstantColor | BlendFactor::OneMinusConstantAlpha => {
            wgpu::BlendFactor::OneMinusConstant
        }
        _ => wgpu::BlendFactor::One,
    }
}

fn wgpu_blend_op(op: BlendOp) -> wgpu::BlendOperation {
    match op {
        BlendOp::Add => wgpu::BlendOperation::Add,
        BlendOp::Subtract => wgpu::BlendOperation::Subtract,
        BlendOp::ReverseSubtract => wgpu::BlendOperation::ReverseSubtract,
        BlendOp::Min => wgpu::BlendOperation::Min,
        BlendOp::Max => wgpu::BlendOperation::Max,
        _ => wgpu::BlendOperation::Add,
    }
}

fn wgpu_blend_state(blend: PipelineBlend) -> Option<wgpu::BlendState> {
    let func = blend.to_blend_function()?;
    Some(wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu_blend_factor(func.color.src),
            dst_factor: wgpu_blend_factor(func.color.dst),
            operation: wgpu_blend_op(func.color.op),
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu_blend_factor(func.alpha.src),
            dst_factor: wgpu_blend_factor(func.alpha.dst),
            operation: wgpu_blend_op(func.alpha.op),
        },
    })
}

fn wgpu_color_writes(write: ColorWrite) -> wgpu::ColorWrites {
    let mut mask = wgpu::ColorWrites::empty();
    if write.color {
        mask |= wgpu::ColorWrites::COLOR;
    }
    if write.alpha {
        mask |= wgpu::ColorWrites::ALPHA;
    }
    mask
}

fn wgpu_vertex_format(fmt: VertexAttributeFormat) -> wgpu::VertexFormat {
    match fmt {
        VertexAttributeFormat::Float32 => wgpu::VertexFormat::Float32,
        VertexAttributeFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
        VertexAttributeFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
        VertexAttributeFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
        VertexAttributeFormat::Float16x2 => wgpu::VertexFormat::Float16x2,
        VertexAttributeFormat::Float16x4 => wgpu::VertexFormat::Float16x4,
        VertexAttributeFormat::Unorm8x2 => wgpu::VertexFormat::Unorm8x2,
        VertexAttributeFormat::Unorm8x4 => wgpu::VertexFormat::Unorm8x4,
        VertexAttributeFormat::Snorm8x2 => wgpu::VertexFormat::Snorm8x2,
        VertexAttributeFormat::Snorm8x4 => wgpu::VertexFormat::Snorm8x4,
        VertexAttributeFormat::Uint8x2 => wgpu::VertexFormat::Uint8x2,
        VertexAttributeFormat::Uint8x4 => wgpu::VertexFormat::Uint8x4,
        VertexAttributeFormat::Sint8x2 => wgpu::VertexFormat::Sint8x2,
        VertexAttributeFormat::Sint8x4 => wgpu::VertexFormat::Sint8x4,
        VertexAttributeFormat::Unorm16x2 => wgpu::VertexFormat::Unorm16x2,
        VertexAttributeFormat::Unorm16x4 => wgpu::VertexFormat::Unorm16x4,
        VertexAttributeFormat::Snorm16x2 => wgpu::VertexFormat::Snorm16x2,
        VertexAttributeFormat::Snorm16x4 => wgpu::VertexFormat::Snorm16x4,
        VertexAttributeFormat::Uint16x2 => wgpu::VertexFormat::Uint16x2,
        VertexAttributeFormat::Uint16x4 => wgpu::VertexFormat::Uint16x4,
        VertexAttributeFormat::Sint16x2 => wgpu::VertexFormat::Sint16x2,
        VertexAttributeFormat::Sint16x4 => wgpu::VertexFormat::Sint16x4,
        VertexAttributeFormat::Uint32 => wgpu::VertexFormat::Uint32,
        VertexAttributeFormat::Uint32x2 => wgpu::VertexFormat::Uint32x2,
        VertexAttributeFormat::Uint32x3 => wgpu::VertexFormat::Uint32x3,
        VertexAttributeFormat::Uint32x4 => wgpu::VertexFormat::Uint32x4,
        VertexAttributeFormat::Sint32 => wgpu::VertexFormat::Sint32,
        VertexAttributeFormat::Sint32x2 => wgpu::VertexFormat::Sint32x2,
        VertexAttributeFormat::Sint32x3 => wgpu::VertexFormat::Sint32x3,
        VertexAttributeFormat::Sint32x4 => wgpu::VertexFormat::Sint32x4,
        VertexAttributeFormat::Unorm10_10_10_2 => wgpu::VertexFormat::Unorm10_10_10_2,
        _ => wgpu::VertexFormat::Float32,
    }
}

fn wgpu_step_mode(mode: VertexStepMode) -> wgpu::VertexStepMode {
    match mode {
        VertexStepMode::Vertex => wgpu::VertexStepMode::Vertex,
        VertexStepMode::Instance => wgpu::VertexStepMode::Instance,
        _ => wgpu::VertexStepMode::Vertex,
    }
}

/// Owned vertex buffer layout for one binding slot of a compiled mod pipeline.
struct OwnedVertexBufferLayout {
    binding: u8,
    stride: u64,
    step_mode: wgpu::VertexStepMode,
    attributes: Vec<wgpu::VertexAttribute>,
}

/// Convert a pipeline's sorted `(binding, VertexFormat)` pairs into owned wgpu vertex buffer
/// layouts with deterministic sequential locations `0..N`, refusing binding indices that exceed
/// `device.limits().max_vertex_buffers`.
fn build_vertex_layouts(
    bindings: &[(u8, VertexFormat)],
    max_vertex_buffers: u32,
) -> Result<Vec<OwnedVertexBufferLayout>, String> {
    let mut out = Vec::with_capacity(bindings.len());
    let mut next_location = 0u32;
    for (binding, fmt) in bindings {
        if u32::from(*binding) >= max_vertex_buffers {
            return Err(format!(
                "vertex binding {binding} exceeds adapter limit max_vertex_buffers ({max_vertex_buffers})"
            ));
        }
        let mut attributes = Vec::with_capacity(fmt.attributes.len());
        for attr in &fmt.attributes {
            attributes.push(wgpu::VertexAttribute {
                format: wgpu_vertex_format(attr.format),
                offset: u64::from(attr.offset),
                shader_location: next_location,
            });
            next_location += 1;
        }
        out.push(OwnedVertexBufferLayout {
            binding: *binding,
            stride: u64::from(fmt.stride),
            step_mode: wgpu_step_mode(fmt.step_mode),
            attributes,
        });
    }
    Ok(out)
}

fn wgpu_buffer_layouts(
    owned: &[OwnedVertexBufferLayout],
) -> Vec<Option<wgpu::VertexBufferLayout<'_>>> {
    let slot_count = owned
        .iter()
        .map(|l| usize::from(l.binding) + 1)
        .max()
        .unwrap_or(0);
    let mut layouts = vec![None; slot_count];
    for layout in owned {
        layouts[usize::from(layout.binding)] = Some(wgpu::VertexBufferLayout {
            array_stride: layout.stride,
            step_mode: layout.step_mode,
            attributes: &layout.attributes,
        });
    }
    layouts
}

fn wgpu_topology(topology: PrimitiveTopology) -> wgpu::PrimitiveTopology {
    match topology {
        PrimitiveTopology::Quads
        | PrimitiveTopology::Lines
        | PrimitiveTopology::Triangles
        | PrimitiveTopology::TriangleFan => wgpu::PrimitiveTopology::TriangleList,
        PrimitiveTopology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
        PrimitiveTopology::DebugLines => wgpu::PrimitiveTopology::LineList,
        PrimitiveTopology::DebugLineStrip => wgpu::PrimitiveTopology::LineStrip,
        PrimitiveTopology::Points => wgpu::PrimitiveTopology::PointList,
        _ => wgpu::PrimitiveTopology::TriangleList,
    }
}

/// Generate triangle-list indices (`[0, i, i + 1]` for `i` in `1..vertex_count - 1`) to emulate
/// [`PrimitiveTopology::TriangleFan`] on WebGPU/wgpu.
pub(crate) fn fan_indices_for(vertex_count: u32) -> Vec<u32> {
    if vertex_count < 3 {
        return Vec::new();
    }
    let triangles = (vertex_count - 2) as usize;
    let mut indices = Vec::with_capacity(triangles * 3);
    for i in 1..(vertex_count - 1) {
        indices.extend_from_slice(&[0, i, i + 1]);
    }
    indices
}

/// Engine-generated index buffer emulating [`PrimitiveTopology::TriangleFan`] as a triangle list.
#[derive(Default)]
pub(crate) struct FanIndices {
    buffer: Option<wgpu::Buffer>,
    vertex_capacity: u32,
}

impl FanIndices {
    /// Ensure the GPU fan index buffer covers at least `vertex_count` vertices and return it.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertex_count: u32,
    ) -> &wgpu::Buffer {
        self.ensure_with_upload(
            device,
            &mut |buf, bytes| queue.write_buffer(buf, 0, bytes),
            vertex_count,
        )
    }

    /// Ensure the GPU fan index buffer covers at least `vertex_count` vertices using `upload`.
    pub(crate) fn ensure_with_upload(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        vertex_count: u32,
    ) -> &wgpu::Buffer {
        let needed = vertex_count.max(3);
        if self.buffer.is_none() || needed > self.vertex_capacity {
            let capacity = needed.next_power_of_two().max(64);
            let data = fan_indices_for(capacity);
            let bytes: &[u8] = bytemuck::cast_slice(&data);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Mod pipeline triangle-fan indices"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            upload(&buffer, bytes);
            self.buffer = Some(buffer);
            self.vertex_capacity = capacity;
        }
        self.buffer.as_ref().expect("fan index buffer created")
    }

    pub(crate) fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }
}

/// Ensure a resizable transient GPU byte buffer holds at least `bytes.len()` bytes (capped at
/// `legal_buffer_limit(device)`) and upload `bytes` to it.
fn upload_transient_bytes(
    slot: &mut Option<wgpu::Buffer>,
    device: &wgpu::Device,
    upload: Upload<'_>,
    label: &'static str,
    usage: wgpu::BufferUsages,
    bytes: &[u8],
) {
    if bytes.is_empty() {
        return;
    }
    let limit = super::upload::legal_buffer_limit(device);
    let needed = (bytes.len() as u64).min(limit);
    let needs_alloc = slot
        .as_ref()
        .is_none_or(|b| b.size() < needed || b.size() > limit);
    if needs_alloc {
        let size = needed.next_power_of_two().clamp(4096, limit.max(4096));
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    if let Some(buf) = slot.as_ref() {
        let len = (bytes.len() as u64).min(buf.size()) as usize;
        let aligned = len & !3;
        if aligned > 0 {
            upload(buf, &bytes[..aligned]);
        }
    }
}

/// Cached lazy variant for one `(pipeline, modifier_stack)` pair.
enum CachedVariant {
    /// Modifier changed no effective state for this target; reuse the base pipeline.
    Base,
    /// Compiled variant pipeline.
    Compiled(wgpu::RenderPipeline),
    /// GPU refused this variant (already logged once with both ids); skip its runs.
    Refused,
}

/// One compiled OIT sub-pass (`DepthBounds`, `Transmittance`, or `Accumulate`) of a mod pipeline.
struct CompiledOitStage {
    pipeline: wgpu::RenderPipeline,
    pipeline_layout: wgpu::PipelineLayout,
    group_layout: wgpu::BindGroupLayout,
    vertex_shader: wgpu::ShaderModule,
    fragment_shader: wgpu::ShaderModule,
    vanilla_mask: u8,
    reads_sampler0: bool,
    reads_sprite_rect: bool,
    /// `(binding, cloud_samples_index)` for `DepthBoundsSampler` (0), `Coeff0` (1), `Coeff1` (2).
    oit_samplers: Vec<(u32, usize)>,
    /// Cached bind group for `ModPipelines::oit_generation`.
    group: Option<wgpu::BindGroup>,
    /// Lazy modifier variants indexed by `modifier_stack`.
    variants: Vec<Option<CachedVariant>>,
}

struct CompiledOit {
    stages: [CompiledOitStage; 3],
    /// Sprite atlas texture view and rect buffer kept alive for OIT bind groups.
    sprite: Option<(wgpu::TextureView, wgpu::Buffer)>,
}

/// A built pipeline: its compiled render pipeline, its per-pipeline bind group (binding only
/// the set-0 blocks/samplers its stages read, backed by the shared vanilla uniform buffer plus
/// optional sprite texture/rect), and the shader modules and fixed state needed to lazily
/// One compiled entry in a pipeline's set-1 `BindGroupLayout`.
#[derive(Clone, Copy, Debug)]
struct CompiledSet1Entry {
    binding: u32,
    kind: BindingKind,
}

/// Cache key for one resource slot in a cached set-1 `wgpu::BindGroup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Set1ResourceKey {
    /// Retained uniform buffer `(unique_id, byte_len)`.
    RetainedUniform(u64, u64),
    /// Inline uniform slice bound with dynamic offset (`bound_size`).
    InlineUniform(u64),
    /// Texture view `(unique_view_id)`.
    Texture(u64),
    /// Texel storage buffer range `(offset, size)`.
    TexelBuffer(u64, u64),
}

/// Cache key for steady-state zero-allocation set-1 `wgpu::BindGroup` reuse.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Set1CacheKey {
    slot: usize,
    generation: u64,
    resources: Vec<Set1ResourceKey>,
}

/// compile modifier and shader-pack target variants.
struct Compiled {
    id: Identifier,
    pipeline: wgpu::RenderPipeline,
    pipeline_layout: wgpu::PipelineLayout,
    group: wgpu::BindGroup,
    pack_group: wgpu::BindGroup,
    set1_layout: Option<wgpu::BindGroupLayout>,
    set1_entries: Vec<CompiledSet1Entry>,
    push_constant_slot: Option<(u32, u32)>,
    vertex_shader: wgpu::ShaderModule,
    /// Fragment shader targeting the main frame (linear-decoded when `linear_light`).
    fragment_shader: wgpu::ShaderModule,
    /// Vanilla-gamma fragment shader for shader-pack `colortex0` targets (when `linear_light`
    /// differs from the pack's gamma encoding, `pack_fragment_shader` holds the undecoded stage).
    pack_fragment_shader: Option<wgpu::ShaderModule>,
    state: PipelineState,
    color_targets: Vec<Option<ColorTargetState>>,
    vertex_layouts: Vec<OwnedVertexBufferLayout>,
    /// Lazy modifier variants indexed by interned `modifier_stack` id (`0` unused; base is used).
    variants: Vec<Option<CachedVariant>>,
    /// Compiled OIT variants (`DepthBounds`, `Transmittance`, `Accumulate`) when opted in and
    /// supported by the device.
    oit: Option<CompiledOit>,
    #[cfg(test)]
    textured: bool,
}

/// The pipelines and the buffer that draw a [`Batch`].
pub struct ModPipelines {
    color_format: wgpu::TextureFormat,
    linear_light: bool,
    /// Shared nearest/default sampler for `Sampler0`.
    sampler: wgpu::Sampler,
    /// Non-filtering nearest sampler for OIT float textures (`DepthBoundsSampler`, `Coeff0`, `Coeff1`).
    non_filtering_sampler: wgpu::Sampler,
    /// Shared set-1 samplers indexed by `[filter][address]` (`Nearest`/`Linear` × `Clamp`/`Repeat`/`Mirror`).
    mod_samplers: [[wgpu::Sampler; 3]; 2],
    /// Shared 1024-byte vanilla uniform buffer (`Globals`, `Projection`, `DynamicTransforms`, `Fog`).
    vanilla: wgpu::Buffer,
    /// CPU copy of the most recently uploaded `vanilla` blocks so offscreen passes can override
    /// `Projection`/`ModelViewMat`/`ScreenSize` and restore afterwards in command-buffer order.
    last_vanilla_blocks: [u8; 1024],
    /// Same blocks with `Projection` adapted to shader-pack GL coordinates (`pack_clip_matrix`).
    pack_vanilla: wgpu::Buffer,
    vertices: VertexPages,
    oit_vertices: VertexPages,
    transient_vbuf: Option<wgpu::Buffer>,
    transient_ibuf: Option<wgpu::Buffer>,
    oit_transient_vbuf: Option<wgpu::Buffer>,
    oit_transient_ibuf: Option<wgpu::Buffer>,
    /// Per-frame growing GPU uniform ring buffer for `UniformValue::Inline` and push constants.
    inline_uniform_buf: Option<wgpu::Buffer>,
    inline_uniform_staging: Vec<u8>,
    /// Per-frame growing GPU read-only storage buffer for `TexelBuffer` bindings.
    texel_storage_buf: Option<wgpu::Buffer>,
    texel_staging: Vec<u8>,
    /// Steady-state cache of set-1 `wgpu::BindGroup`s (cleared on reload or ring reallocation).
    set1_bind_group_cache: HashMap<Set1CacheKey, wgpu::BindGroup>,
    fan_indices: FanIndices,
    /// The load `pipelines` was built from; 0 before any.
    generation: u64,
    /// OIT target generation last bound into `CompiledOitStage::group`; 0 before any.
    oit_generation: u64,
    /// By slot; `None` for a pipeline the load refused.
    pipelines: Vec<Option<Compiled>>,
    /// Lazily built for a shader pack's `(colortex0 format, rgb_only)` + GL `[0..1]` depth.
    pack_pipelines: Option<(
        (wgpu::TextureFormat, bool),
        Vec<Option<wgpu::RenderPipeline>>,
    )>,
    /// Lazily built offscreen variants keyed by `(slot, color_formats, depth_format)`.
    offscreen_pipelines: HashMap<
        (usize, Vec<wgpu::TextureFormat>, Option<wgpu::TextureFormat>),
        Option<wgpu::RenderPipeline>,
    >,
    runs: Vec<Run>,
    oit_runs: Vec<Run>,
    draws: Vec<PreparedDraw>,
    oit_draws: Vec<PreparedDraw>,
}

fn sampler_filter_idx(filter: SamplerFilter) -> usize {
    match filter {
        SamplerFilter::Linear => 1,
        _ => 0,
    }
}

fn sampler_address_idx(address: SamplerAddress) -> usize {
    match address {
        SamplerAddress::Repeat => 1,
        SamplerAddress::MirroredRepeat => 2,
        _ => 0,
    }
}

fn create_mod_samplers(device: &wgpu::Device) -> [[wgpu::Sampler; 3]; 2] {
    let filters = [wgpu::FilterMode::Nearest, wgpu::FilterMode::Linear];
    let addresses = [
        wgpu::AddressMode::ClampToEdge,
        wgpu::AddressMode::Repeat,
        wgpu::AddressMode::MirrorRepeat,
    ];
    std::array::from_fn(|f| {
        std::array::from_fn(|a| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Mod set-1 sampler"),
                address_mode_u: addresses[a],
                address_mode_v: addresses[a],
                address_mode_w: addresses[a],
                mag_filter: filters[f],
                min_filter: filters[f],
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            })
        })
    })
}

fn build_set1_layout(
    device: &wgpu::Device,
    built: &Built,
) -> Result<
    (
        Option<wgpu::BindGroupLayout>,
        Vec<CompiledSet1Entry>,
        Option<(u32, u32)>,
    ),
    String,
> {
    if built.bind_group_layout.is_empty() && built.push_constant_size == 0 {
        return Ok((None, Vec::new(), None));
    }
    let limits = device.limits();
    let mut layout_entries = Vec::new();
    let mut compiled_entries = Vec::new();
    let mut next_binding = 0u32;
    let mut uniform_count = 0u32;
    let mut dynamic_uniform_count = 0u32;
    let mut texture_count = 0u32;
    let mut sampler_count = 0u32;
    let mut storage_count = 0u32;
    for entry in &built.bind_group_layout.entries {
        match entry.kind {
            BindingKind::UniformBuffer { min_size } => {
                if u64::from(min_size) > u64::from(limits.max_uniform_buffer_binding_size) {
                    return Err(format!(
                        "uniform buffer `{}` min_size ({min_size}) exceeds device max_uniform_buffer_binding_size ({})",
                        entry.name, limits.max_uniform_buffer_binding_size
                    ));
                }
                uniform_count += 1;
                dynamic_uniform_count += 1;
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(u64::from(min_size)),
                    },
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                });
                next_binding += 1;
            }
            BindingKind::CombinedImageSampler { filter, .. } => {
                texture_count += 1;
                sampler_count += 1;
                let filterable = filter == SamplerFilter::Linear;
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding + 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(if filterable {
                        wgpu::SamplerBindingType::Filtering
                    } else {
                        wgpu::SamplerBindingType::NonFiltering
                    }),
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                });
                next_binding += 2;
            }
            BindingKind::TexelBuffer { .. } => {
                storage_count += 1;
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next_binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(4),
                    },
                    count: None,
                });
                compiled_entries.push(CompiledSet1Entry {
                    binding: next_binding,
                    kind: entry.kind,
                });
                next_binding += 1;
            }
            _ => {
                return Err(format!(
                    "unsupported bind-group entry kind for `{}`",
                    entry.name
                ));
            }
        }
    }
    let push_constant_slot = if built.push_constant_size > 0 {
        uniform_count += 1;
        dynamic_uniform_count += 1;
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: next_binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(u64::from(built.push_constant_size)),
            },
            count: None,
        });
        let slot = (next_binding, built.push_constant_size);
        next_binding += 1;
        Some(slot)
    } else {
        None
    };
    let _ = next_binding;
    if dynamic_uniform_count > limits.max_dynamic_uniform_buffers_per_pipeline_layout {
        return Err(format!(
            "set-1 uniform buffer count ({dynamic_uniform_count}) exceeds device max_dynamic_uniform_buffers_per_pipeline_layout ({})",
            limits.max_dynamic_uniform_buffers_per_pipeline_layout
        ));
    }
    if uniform_count > limits.max_uniform_buffers_per_shader_stage {
        return Err(format!(
            "set-1 uniform buffer count ({uniform_count}) exceeds device max_uniform_buffers_per_shader_stage ({})",
            limits.max_uniform_buffers_per_shader_stage
        ));
    }
    if texture_count > limits.max_sampled_textures_per_shader_stage {
        return Err(format!(
            "set-1 sampled texture count ({texture_count}) exceeds device max_sampled_textures_per_shader_stage ({})",
            limits.max_sampled_textures_per_shader_stage
        ));
    }
    if sampler_count > limits.max_samplers_per_shader_stage {
        return Err(format!(
            "set-1 sampler count ({sampler_count}) exceeds device max_samplers_per_shader_stage ({})",
            limits.max_samplers_per_shader_stage
        ));
    }
    if storage_count > limits.max_storage_buffers_per_shader_stage {
        return Err(format!(
            "set-1 storage/texel buffer count ({storage_count}) exceeds device max_storage_buffers_per_shader_stage ({})",
            limits.max_storage_buffers_per_shader_stage
        ));
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Mod pipeline set-1"),
        entries: &layout_entries,
    });
    Ok((Some(layout), compiled_entries, push_constant_slot))
}

fn align_up_usize(value: usize, align: usize) -> usize {
    if align <= 1 {
        return value;
    }
    let rem = value % align;
    if rem == 0 {
        value
    } else {
        value + (align - rem)
    }
}

impl ModPipelines {
    /// Storage for pipelines drawing into `(color_format, linear_light)` over a reversed
    /// Depth32Float depth.
    pub fn new(
        device: &wgpu::Device,
        (color_format, linear_light): (wgpu::TextureFormat, bool),
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mod pipeline texture"),
            ..Default::default()
        });
        let non_filtering_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mod pipeline OIT sampler"),
            ..Default::default()
        });
        let mod_samplers = create_mod_samplers(device);
        let buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 1024,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let vanilla = buffer("Mod pipeline vanilla uniforms");
        let pack_vanilla = buffer("Mod pipeline pack vanilla uniforms");
        Self {
            color_format,
            linear_light,
            sampler,
            non_filtering_sampler,
            mod_samplers,
            vanilla,
            last_vanilla_blocks: fallback_vanilla_blocks(Mat4::IDENTITY, [1, 1], 0.0),
            pack_vanilla,
            vertices: VertexPages::default(),
            oit_vertices: VertexPages::default(),
            transient_vbuf: None,
            transient_ibuf: None,
            oit_transient_vbuf: None,
            oit_transient_ibuf: None,
            inline_uniform_buf: None,
            inline_uniform_staging: Vec::new(),
            texel_storage_buf: None,
            texel_staging: Vec::new(),
            set1_bind_group_cache: HashMap::new(),
            fan_indices: FanIndices::default(),
            generation: 0,
            oit_generation: 0,
            pipelines: Vec::new(),
            pack_pipelines: None,
            offscreen_pipelines: HashMap::new(),
            runs: Vec::new(),
            oit_runs: Vec::new(),
            draws: Vec::new(),
            oit_draws: Vec::new(),
        }
    }

    fn build_render_pipeline(
        device: &wgpu::Device,
        label: &str,
        pipeline_layout: &wgpu::PipelineLayout,
        vertex_shader: &wgpu::ShaderModule,
        fragment_shader: &wgpu::ShaderModule,
        vertex_layouts: &[OwnedVertexBufferLayout],
        state: PipelineState,
        color_targets: &[Option<ColorTargetState>],
        target: TargetState<'_>,
    ) -> Result<wgpu::RenderPipeline, String> {
        if state.polygon == PolygonMode::Line
            && !device
                .features()
                .contains(wgpu::Features::POLYGON_MODE_LINE)
        {
            return Err("PolygonMode::Line requires the POLYGON_MODE_LINE device feature".into());
        }
        let targets: Vec<Option<wgpu::ColorTargetState>> = if color_targets.is_empty() {
            let blend = wgpu_blend_state(state.blend);
            let write_mask = wgpu_color_writes(state.color_write) & target.write_mask;
            target
                .color_formats
                .iter()
                .map(|&format| {
                    Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask,
                    })
                })
                .collect()
        } else {
            target
                .color_formats
                .iter()
                .enumerate()
                .map(|(idx, &format)| {
                    let ct = color_targets.get(idx).copied().flatten()?;
                    let (blend, write) = if idx == 0 {
                        (state.blend, state.color_write)
                    } else {
                        (ct.blend, ct.write)
                    };
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: wgpu_blend_state(blend),
                        write_mask: wgpu_color_writes(write) & target.write_mask,
                    })
                })
                .collect()
        };
        let depth_stencil = match (target.depth_format, state.depth_stencil) {
            (Some(format), Some(ds)) => {
                let compare = if target.standard_z {
                    ds.compare.flip_for_standard_z()
                } else {
                    ds.compare
                };
                let (constant, slope_scale) = if target.standard_z {
                    (
                        ds.depth_bias_constant.saturating_neg(),
                        -ds.depth_bias_slope_scale,
                    )
                } else {
                    (ds.depth_bias_constant, ds.depth_bias_slope_scale)
                };
                Some(wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(ds.write),
                    depth_compare: Some(wgpu_compare(compare)),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant,
                        slope_scale,
                        clamp: 0.0,
                    },
                })
            }
            (Some(format), None) if target.force_depth_attachment => {
                Some(wgpu::DepthStencilState {
                    format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                })
            }
            _ => None,
        };

        let buffers = wgpu_buffer_layouts(vertex_layouts);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu_topology(state.topology),
                cull_mode: state.cull.then_some(wgpu::Face::Back),
                polygon_mode: match state.polygon {
                    PolygonMode::Line => wgpu::PolygonMode::Line,
                    _ => wgpu::PolygonMode::Fill,
                },
                ..Default::default()
            },
            depth_stencil,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        match internal.or(validation) {
            None => Ok(pipeline),
            Some(error) => Err(error.to_string()),
        }
    }

    fn build_oit_render_pipeline(
        device: &wgpu::Device,
        label: &str,
        pipeline_layout: &wgpu::PipelineLayout,
        vertex_shader: &wgpu::ShaderModule,
        fragment_shader: &wgpu::ShaderModule,
        vertex_layouts: &[OwnedVertexBufferLayout],
        state: PipelineState,
        stage: super::oit::Stage,
    ) -> Result<wgpu::RenderPipeline, String> {
        if state.polygon == PolygonMode::Line
            && !device
                .features()
                .contains(wgpu::Features::POLYGON_MODE_LINE)
        {
            return Err("PolygonMode::Line requires the POLYGON_MODE_LINE device feature".into());
        }
        let targets = super::oit::stage_targets(stage);
        let (depth_compare, bias) = match state.depth_stencil {
            Some(ds) => (
                wgpu_compare(ds.compare),
                wgpu::DepthBiasState {
                    constant: ds.depth_bias_constant,
                    slope_scale: ds.depth_bias_slope_scale,
                    clamp: 0.0,
                },
            ),
            None => (wgpu::CompareFunction::GreaterEqual, Default::default()),
        };
        let buffers = wgpu_buffer_layouts(vertex_layouts);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu_topology(state.topology),
                cull_mode: state.cull.then_some(wgpu::Face::Back),
                polygon_mode: match state.polygon {
                    PolygonMode::Line => wgpu::PolygonMode::Line,
                    _ => wgpu::PolygonMode::Fill,
                },
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // OIT passes attach depth read-only (`depth_ops: None`).
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare),
                stencil: Default::default(),
                bias,
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader,
                entry_point: Some(ENTRY),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        match internal.or(validation) {
            None => Ok(pipeline),
            Some(error) => Err(error.to_string()),
        }
    }

    fn build_oit_stages(
        device: &wgpu::Device,
        id: &Identifier,
        vertex_layouts: &[OwnedVertexBufferLayout],
        state: PipelineState,
        variants: &[sunrise_glsl::Translated; 3],
        sprite_binding: Option<&(wgpu::TextureView, [f32; 4])>,
        set1_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<CompiledOit, String> {
        use wgpu::util::DeviceExt;
        let sprite = sprite_binding.map(|(view, rect)| {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod pipeline OIT sprite rect"),
                contents: bytemuck::cast_slice(rect),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            (view.clone(), buf)
        });
        let mut built_stages = Vec::with_capacity(3);
        for (stage_idx, (stage, program)) in super::oit::Stage::ALL
            .into_iter()
            .zip(variants.iter())
            .enumerate()
        {
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Mod pipeline OIT (vertex)"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.vertex.clone())),
            });
            let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Mod pipeline OIT (fragment)"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.fragment.clone())),
            });

            let mut layout_entries = Vec::new();
            let mut vanilla_mask = 0u8;
            for b in 0..4u32 {
                let stages = program.stages_reading(b);
                if !stages.is_empty() {
                    vanilla_mask |= 1 << b;
                    layout_entries.push(block_entry(b, VANILLA_SIZES[b as usize], stages));
                }
            }
            let sampler0_stages = program.stages_reading(4) | program.stages_reading(5);
            let reads_sampler0 = !sampler0_stages.is_empty() && sprite.is_some();
            if reads_sampler0 {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: sampler0_stages,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: sampler0_stages,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
            }
            let sprite_rect_stages = program.stages_reading(SPRITE_RECT_BINDING);
            let reads_sprite_rect = !sprite_rect_stages.is_empty() && sprite.is_some();
            if reads_sprite_rect {
                layout_entries.push(block_entry(SPRITE_RECT_BINDING, 16, sprite_rect_stages));
            }
            let mut oit_samplers = Vec::new();
            for (other_idx, name) in program.other.iter().enumerate() {
                let Some(sample_idx) = OIT_STAGE_SAMPLERS.iter().position(|s| s == name) else {
                    continue;
                };
                let binding = program.other_binding(other_idx);
                let stages = program.stages_reading(binding) | program.stages_reading(binding + 1);
                if stages.is_empty() {
                    continue;
                }
                oit_samplers.push((binding, sample_idx));
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: stages,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: binding + 1,
                    visibility: stages,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                });
            }
            let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Mod pipeline OIT"),
                entries: &layout_entries,
            });
            let mut layouts = vec![Some(&group_layout)];
            if let Some(set1) = set1_layout {
                layouts.push(Some(set1));
            }
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Mod pipeline OIT"),
                bind_group_layouts: &layouts,
                ..Default::default()
            });
            let internal = pollster::block_on(internal.pop());
            let validation = pollster::block_on(validation.pop());
            if let Some(error) = internal.or(validation) {
                return Err(format!("OIT stage {stage_idx}: {error}"));
            }
            let label = format!("{id}/oit/{stage_idx}");
            let pipeline = Self::build_oit_render_pipeline(
                device,
                &label,
                &pipeline_layout,
                &vertex_shader,
                &fragment_shader,
                vertex_layouts,
                state,
                stage,
            )
            .map_err(|e| format!("OIT stage {stage_idx}: {e}"))?;
            built_stages.push(CompiledOitStage {
                pipeline,
                pipeline_layout,
                group_layout,
                vertex_shader,
                fragment_shader,
                vanilla_mask,
                reads_sampler0,
                reads_sprite_rect,
                oit_samplers,
                group: None,
                variants: Vec::new(),
            });
        }
        Ok(CompiledOit {
            stages: built_stages.try_into().ok().expect("three OIT stages"),
            sprite,
        })
    }

    /// One load's pipeline: its translated GLSL naga modules with the fixed state it registered,
    /// its per-pipeline set-0 layout and bind groups (`vanilla` and `pack_vanilla`), and its
    /// sprite texture/rect when textured — or the GPU's error.
    fn pipeline(
        &self,
        device: &wgpu::Device,
        built: &Built,
        textures: Option<TextureSources<'_>>,
    ) -> Result<Compiled, String> {
        use wgpu::util::DeviceExt;
        let sprite_binding = match &built.texture {
            None => None,
            Some(BuiltTexture::Sprite(name)) => {
                let sources = textures.ok_or("no texture can be bound here")?;
                let Some(rect) = (sources.sprite)(name) else {
                    return Err(format!(
                        "its texture sprite {name}: {}",
                        crate::mod_pipelines::texture_refusal(
                            mod_api::PipelineTextureError::NoSuchSprite
                        )
                    ));
                };
                Some((sources.atlas.clone(), rect))
            }
        };

        let vertex_layouts =
            build_vertex_layouts(&built.vertex_bindings, device.limits().max_vertex_buffers)?;
        let linear_fragment_module = if self.linear_light {
            let define_refs: Vec<&str> = built.defines.iter().map(String::as_str).collect();
            let vertex_inputs_owned: Vec<(String, &'static str)> = built
                .vertex_bindings
                .iter()
                .flat_map(|(_, fmt)| {
                    fmt.attributes
                        .iter()
                        .map(|a| (a.name.clone(), a.format.glsl_type()))
                })
                .collect();
            let vertex_input_refs: Vec<(&str, &str)> = vertex_inputs_owned
                .iter()
                .map(|(n, t)| (n.as_str(), *t))
                .collect();
            let mod_bindings = built.mod_bindings();
            let options = PipelineOptions {
                textured: built.texture.is_some(),
                defines: &define_refs,
                vertex_inputs: &vertex_input_refs,
                mod_bindings: &mod_bindings,
                push_constant_size: built.push_constant_size,
                linear_light: true,
                check_backends: false,
                interface: crate::core_shader::interface_limits(&device.limits()),
            };
            Some(
                sunrise_mod_pipeline::translate_linear_fragment(
                    &built.vertex_expanded,
                    &built.fragment_expanded,
                    &options,
                )
                .map_err(|e| format!("linear-light fragment decode failed: {e}"))?,
            )
        } else {
            None
        };

        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);

        let make_module = |label: &str, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex_shader = make_module("Mod pipeline (vertex)", &built.program.vertex);
        let (fragment_shader, pack_fragment_shader) = match &linear_fragment_module {
            Some(linear_mod) => (
                make_module("Mod pipeline (fragment, linear)", linear_mod),
                Some(make_module(
                    "Mod pipeline (fragment, gamma)",
                    &built.program.fragment,
                )),
            ),
            None => (
                make_module("Mod pipeline (fragment)", &built.program.fragment),
                None,
            ),
        };

        let sprite_rect_buf = sprite_binding.as_ref().map(|(_, rect)| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod pipeline sprite rect"),
                contents: bytemuck::cast_slice(rect),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });

        let program = &built.program;
        let mut layout_entries = Vec::new();
        for b in 0..4u32 {
            let stages = program.stages_reading(b);
            if !stages.is_empty() {
                layout_entries.push(block_entry(b, VANILLA_SIZES[b as usize], stages));
            }
        }
        let sampler0_stages = program.stages_reading(4) | program.stages_reading(5);
        if !sampler0_stages.is_empty() && sprite_binding.is_some() {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: sampler0_stages,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: sampler0_stages,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let sprite_rect_stages = program.stages_reading(SPRITE_RECT_BINDING);
        if !sprite_rect_stages.is_empty() && sprite_rect_buf.is_some() {
            layout_entries.push(block_entry(SPRITE_RECT_BINDING, 16, sprite_rect_stages));
        }

        let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pipeline"),
            entries: &layout_entries,
        });
        let make_group = |label: &str, uniform_buf: &wgpu::Buffer| {
            let mut entries = Vec::with_capacity(layout_entries.len());
            for b in 0..4u32 {
                if !program.stages_reading(b).is_empty() {
                    entries.push(wgpu::BindGroupEntry {
                        binding: b,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: uniform_buf,
                            offset: VANILLA_OFFSETS[b as usize],
                            size: wgpu::BufferSize::new(VANILLA_SIZES[b as usize]),
                        }),
                    });
                }
            }
            if !sampler0_stages.is_empty()
                && let Some((view, _)) = &sprite_binding
            {
                entries.push(wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(view),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                });
            }
            if !sprite_rect_stages.is_empty()
                && let Some(buf) = &sprite_rect_buf
            {
                entries.push(wgpu::BindGroupEntry {
                    binding: SPRITE_RECT_BINDING,
                    resource: buf.as_entire_binding(),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &group_layout,
                entries: &entries,
            })
        };
        let group = make_group("Mod pipeline", &self.vanilla);
        let pack_group = make_group("Mod pipeline (pack)", &self.pack_vanilla);
        let (set1_layout, set1_entries, push_constant_slot) = build_set1_layout(device, built)?;
        let mut bind_group_layouts = vec![Some(&group_layout)];
        if let Some(ref set1) = set1_layout {
            bind_group_layouts.push(Some(set1));
        }
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mod pipeline"),
            bind_group_layouts: &bind_group_layouts,
            ..Default::default()
        });
        let pipeline_label = built.id.to_string();
        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        if let Some(error) = internal.or(validation) {
            return Err(error.to_string());
        }
        let pipeline = Self::build_render_pipeline(
            device,
            &pipeline_label,
            &pipeline_layout,
            &vertex_shader,
            &fragment_shader,
            &vertex_layouts,
            built.state,
            &built.color_targets,
            TargetState {
                color_formats: &[self.color_format],
                depth_format: Some(wgpu::TextureFormat::Depth32Float),
                force_depth_attachment: false,
                standard_z: false,
                write_mask: wgpu::ColorWrites::ALL,
            },
        )?;
        let oit = if let Some(variants) = &built.oit
            && super::oit::supported(device)
        {
            Some(Self::build_oit_stages(
                device,
                &built.id,
                &vertex_layouts,
                built.state,
                variants,
                sprite_binding.as_ref(),
                set1_layout.as_ref(),
            )?)
        } else {
            None
        };
        Ok(Compiled {
            id: built.id.clone(),
            pipeline,
            pipeline_layout,
            group,
            pack_group,
            set1_layout,
            set1_entries,
            push_constant_slot,
            vertex_shader,
            fragment_shader,
            pack_fragment_shader,
            state: built.state,
            color_targets: built.color_targets.clone(),
            vertex_layouts,
            variants: Vec::new(),
            oit,
            #[cfg(test)]
            textured: sprite_binding.is_some(),
        })
    }

    /// Ensure pack-target variants (`colortex0` format + GL `[0..1]` standard-Z depth) exist
    /// for all loaded mod pipelines.
    pub fn ensure_pack_pipelines(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        rgb: bool,
    ) {
        let key = (format, rgb);
        if self
            .pack_pipelines
            .as_ref()
            .is_some_and(|(prev, list)| *prev == key && list.len() == self.pipelines.len())
        {
            return;
        }
        let color_formats = [format];
        let target = TargetState {
            color_formats: &color_formats,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
            force_depth_attachment: false,
            standard_z: true,
            write_mask: if rgb {
                wgpu::ColorWrites::COLOR
            } else {
                wgpu::ColorWrites::ALL
            },
        };
        let compiled = self
            .pipelines
            .iter()
            .enumerate()
            .map(|(slot, entry)| {
                let c = entry.as_ref()?;
                let frag = c
                    .pack_fragment_shader
                    .as_ref()
                    .unwrap_or(&c.fragment_shader);
                match Self::build_render_pipeline(
                    device,
                    &format!("{}/pack", c.id),
                    &c.pipeline_layout,
                    &c.vertex_shader,
                    frag,
                    &c.vertex_layouts,
                    c.state,
                    &c.color_targets,
                    target,
                ) {
                    Ok(pipeline) => Some(pipeline),
                    Err(error) => {
                        log::warn!(
                            "Sunrise: mod pipeline slot {slot} refused for shader-pack target {format:?}: {error}"
                        );
                        None
                    }
                }
            })
            .collect();
        self.pack_pipelines = Some((key, compiled));
    }

    /// Builds the pipelines of `set` unless they are built already. A required pipeline the GPU
    /// refuses is the error, by id, and leaves this renderer and `set` as they were; an optional
    /// one is logged once, draws nothing and resolves invalid.
    /// A textured pipeline whose texture can't be bound (`textures`) is refused the same way.
    pub(super) fn build(
        &mut self,
        device: &wgpu::Device,
        set: &Pipelines,
        textures: Option<TextureSources<'_>>,
    ) -> Result<(), String> {
        self.build_with(set, |this, built| this.pipeline(device, built, textures))
    }

    fn build_with(
        &mut self,
        set: &Pipelines,
        compile: impl Fn(&Self, &Built) -> Result<Compiled, String>,
    ) -> Result<(), String> {
        if set.generation() == self.generation {
            return Ok(());
        }
        let mut refused = Vec::new();
        let pipelines: Vec<_> = set
            .built()
            .iter()
            .enumerate()
            .map(|(slot, built)| {
                let built = built.as_ref()?;
                compile(self, built)
                    .map_err(|error| refused.push((slot, built, error)))
                    .ok()
            })
            .collect();
        let required: Vec<_> = refused
            .iter()
            .filter(|(_, built, _)| !built.optional)
            .map(|(_, built, error)| format!("required {}", gpu_refusal(built, error)))
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        for (slot, built, error) in refused {
            if set.invalidate(slot) {
                log::warn!("Sunrise: {}", gpu_refusal(built, &error));
            }
        }
        for (slot, pipeline) in pipelines.iter().enumerate() {
            if pipeline.is_some() {
                set.revalidate(slot);
            }
        }
        self.generation = set.generation();
        self.oit_generation = 0;
        self.pipelines = pipelines;
        self.pack_pipelines = None;
        self.offscreen_pipelines.clear();
        self.set1_bind_group_cache.clear();
        Ok(())
    }

    /// Borrow each pipeline slot's optional set-1 `wgpu::BindGroupLayout` (for `PackPost`).
    pub fn set1_bind_group_layouts(&self) -> Vec<Option<&wgpu::BindGroupLayout>> {
        self.pipelines
            .iter()
            .map(|slot| slot.as_ref().and_then(|c| c.set1_layout.as_ref()))
            .collect()
    }

    /// Ensure OIT bind groups (`DepthBoundsSampler`, `Coeff0`, `Coeff1`) match the current OIT
    /// targets generation. Rebuilt only when `samples.generation` changes (e.g. resize/reload).
    pub(super) fn prepare_oit_groups(
        &mut self,
        device: &wgpu::Device,
        samples: Option<super::oit::CloudSamples<'_>>,
    ) {
        let Some(samples) = samples else {
            return;
        };
        if samples.generation == self.oit_generation {
            return;
        }
        for compiled in self.pipelines.iter_mut().flatten() {
            let Some(oit) = compiled.oit.as_mut() else {
                continue;
            };
            for stage in &mut oit.stages {
                let mut entries = Vec::new();
                for b in 0..4u32 {
                    if (stage.vanilla_mask & (1 << b)) != 0 {
                        entries.push(wgpu::BindGroupEntry {
                            binding: b,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.vanilla,
                                offset: VANILLA_OFFSETS[b as usize],
                                size: wgpu::BufferSize::new(VANILLA_SIZES[b as usize]),
                            }),
                        });
                    }
                }
                if stage.reads_sampler0
                    && let Some((view, _)) = &oit.sprite
                {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(view),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    });
                }
                if stage.reads_sprite_rect
                    && let Some((_, buf)) = &oit.sprite
                {
                    entries.push(wgpu::BindGroupEntry {
                        binding: SPRITE_RECT_BINDING,
                        resource: buf.as_entire_binding(),
                    });
                }
                for &(binding, sample_idx) in &stage.oit_samplers {
                    entries.push(wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(samples.views[sample_idx]),
                    });
                    entries.push(wgpu::BindGroupEntry {
                        binding: binding + 1,
                        resource: wgpu::BindingResource::Sampler(&self.non_filtering_sampler),
                    });
                }
                stage.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Mod pipeline OIT"),
                    layout: &stage.group_layout,
                    entries: &entries,
                }));
            }
        }
        self.oit_generation = samples.generation;
    }

    /// Lazily compile a modified variant for `(slot, stack_id)` if not cached yet.
    fn ensure_modifier_variant(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        compiled: &mut Compiled,
        stack_id: u32,
        stacks: &ModifierStacks,
        defs: &[mod_api::PipelineModifier],
    ) {
        let idx = stack_id as usize;
        if idx == 0 {
            return;
        }
        if compiled.variants.len() <= idx {
            compiled.variants.resize_with(idx + 1, || None);
        }
        if compiled.variants[idx].is_some() {
            return;
        }
        let modified = stacks.apply(stack_id, compiled.state, defs);
        if modified == compiled.state {
            compiled.variants[idx] = Some(CachedVariant::Base);
            return;
        }
        let label = stacks.label(stack_id, &compiled.id.to_string(), defs);
        let color_formats = [color_format];
        match Self::build_render_pipeline(
            device,
            &label,
            &compiled.pipeline_layout,
            &compiled.vertex_shader,
            &compiled.fragment_shader,
            &compiled.vertex_layouts,
            modified,
            &compiled.color_targets,
            TargetState {
                color_formats: &color_formats,
                depth_format: Some(wgpu::TextureFormat::Depth32Float),
                force_depth_attachment: false,
                standard_z: false,
                write_mask: wgpu::ColorWrites::ALL,
            },
        ) {
            Ok(pipeline) => compiled.variants[idx] = Some(CachedVariant::Compiled(pipeline)),
            Err(error) => {
                log::warn!("Sunrise: pipeline modifier variant {label} refused: {error}");
                compiled.variants[idx] = Some(CachedVariant::Refused);
            }
        }
    }

    /// Lazily compile a modified OIT variant for `(slot, oit_stage, stack_id)` if not cached yet.
    /// `blend` and `depth_write` overrides are ignored because OIT owns target blending and
    /// read-only depth.
    fn ensure_oit_modifier_variant(
        device: &wgpu::Device,
        compiled: &mut Compiled,
        stage_idx: usize,
        stack_id: u32,
        stacks: &ModifierStacks,
        defs: &[mod_api::PipelineModifier],
    ) {
        let idx = stack_id as usize;
        if idx == 0 {
            return;
        }
        let Some(oit) = compiled.oit.as_mut() else {
            return;
        };
        let oit_stage = &mut oit.stages[stage_idx];
        if oit_stage.variants.len() <= idx {
            oit_stage.variants.resize_with(idx + 1, || None);
        }
        if oit_stage.variants[idx].is_some() {
            return;
        }
        let modified = stacks.apply(stack_id, compiled.state, defs);
        let depth_key = |ds: Option<mod_api::DepthState>| {
            ds.map(|d| {
                (
                    d.compare,
                    d.depth_bias_constant,
                    d.depth_bias_slope_scale.to_bits(),
                )
            })
        };
        if depth_key(modified.depth_stencil) == depth_key(compiled.state.depth_stencil)
            && modified.cull == compiled.state.cull
            && modified.polygon == compiled.state.polygon
            && modified.topology == compiled.state.topology
        {
            oit_stage.variants[idx] = Some(CachedVariant::Base);
            return;
        }
        let label = format!(
            "{}/oit/{stage_idx}",
            stacks.label(stack_id, &compiled.id.to_string(), defs)
        );
        match Self::build_oit_render_pipeline(
            device,
            &label,
            &oit_stage.pipeline_layout,
            &oit_stage.vertex_shader,
            &oit_stage.fragment_shader,
            &compiled.vertex_layouts,
            modified,
            super::oit::Stage::ALL[stage_idx],
        ) {
            Ok(pipeline) => oit_stage.variants[idx] = Some(CachedVariant::Compiled(pipeline)),
            Err(error) => {
                log::warn!("Sunrise: OIT pipeline modifier variant {label} refused: {error}");
                oit_stage.variants[idx] = Some(CachedVariant::Refused);
            }
        }
    }

    /// Upload the frame's quads and vanilla uniform blocks (`vanilla` as `core_lines::uniforms`
    /// lays them out, or synthesized from `view_proj`/`viewport`/`batch.time` when `None`),
    /// returning the number of indices needed for its largest page.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'_>,
        view_proj: Mat4,
        viewport: [u32; 2],
        vanilla_blocks: Option<&[u8; 1024]>,
    ) -> u32 {
        self.runs.clear();
        self.oit_runs.clear();
        self.draws.clear();
        self.oit_draws.clear();
        if batch
            .set
            .is_none_or(|set| set.generation() != self.generation)
        {
            return 0;
        }
        let modifier_defs = batch
            .modifier_defs
            .unwrap_or_else(crate::mod_pipelines::registered_modifiers);
        let mut next = 0;
        for run in batch.runs {
            let quads = next..next + run.quads;
            next = quads.end;
            if quads.end as usize * 4 > batch.vertices.len() {
                continue;
            }
            let Some(Some(compiled)) = self.pipelines.get_mut(run.slot as usize) else {
                continue;
            };
            if run.modifier_stack != ModifierStacks::EMPTY
                && let Some(stacks) = batch.modifiers
            {
                Self::ensure_modifier_variant(
                    device,
                    self.color_format,
                    compiled,
                    run.modifier_stack,
                    stacks,
                    modifier_defs,
                );
            }
            self.runs.push(Run {
                slot: run.slot as usize,
                stage: run.stage as usize,
                quads,
                modifier_stack: run.modifier_stack,
            });
        }
        let mut oit_next = 0;
        for run in batch.oit_runs {
            let quads = oit_next..oit_next + run.quads;
            oit_next = quads.end;
            if quads.end as usize * 4 > batch.oit_vertices.len() || run.stage >= 3 {
                continue;
            }
            let Some(Some(compiled)) = self.pipelines.get_mut(run.slot as usize) else {
                continue;
            };
            if compiled.oit.is_none() {
                continue;
            }
            if run.modifier_stack != ModifierStacks::EMPTY
                && let Some(stacks) = batch.modifiers
            {
                Self::ensure_oit_modifier_variant(
                    device,
                    compiled,
                    run.stage as usize,
                    run.modifier_stack,
                    stacks,
                    modifier_defs,
                );
            }
            self.oit_runs.push(Run {
                slot: run.slot as usize,
                stage: run.stage as usize,
                quads,
                modifier_stack: run.modifier_stack,
            });
        }
        let mut max_index_quads = 0u32;
        let mut max_fan_verts = 0u32;
        for cmd in batch.draws {
            let Some(Some(compiled)) = self.pipelines.get_mut(cmd.slot as usize) else {
                continue;
            };
            let topology = if cmd.modifier_stack != ModifierStacks::EMPTY
                && let Some(stacks) = batch.modifiers
            {
                Self::ensure_modifier_variant(
                    device,
                    self.color_format,
                    compiled,
                    cmd.modifier_stack,
                    stacks,
                    modifier_defs,
                );
                stacks
                    .apply(cmd.modifier_stack, compiled.state, modifier_defs)
                    .topology
            } else {
                compiled.state.topology
            };
            if let DrawRange::Vertices(ref range) = cmd.draw.range {
                let count = range.end.saturating_sub(range.start);
                match topology {
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                        max_index_quads = max_index_quads.max(count / 4);
                    }
                    PrimitiveTopology::TriangleFan => {
                        max_fan_verts = max_fan_verts.max(count);
                    }
                    _ => {}
                }
            }
            self.draws.push(PreparedDraw {
                slot: cmd.slot as usize,
                stage: cmd.stage as usize,
                modifier_stack: cmd.modifier_stack,
                topology,
                draw: cmd.draw.clone(),
                source: cmd.source.clone(),
                set1_bind_group: None,
            });
        }
        for cmd in batch.oit_draws {
            if cmd.stage >= 3 {
                continue;
            }
            let Some(Some(compiled)) = self.pipelines.get_mut(cmd.slot as usize) else {
                continue;
            };
            if compiled.oit.is_none() {
                continue;
            }
            let topology = if cmd.modifier_stack != ModifierStacks::EMPTY
                && let Some(stacks) = batch.modifiers
            {
                Self::ensure_oit_modifier_variant(
                    device,
                    compiled,
                    cmd.stage as usize,
                    cmd.modifier_stack,
                    stacks,
                    modifier_defs,
                );
                stacks
                    .apply(cmd.modifier_stack, compiled.state, modifier_defs)
                    .topology
            } else {
                compiled.state.topology
            };
            if let DrawRange::Vertices(ref range) = cmd.draw.range {
                let count = range.end.saturating_sub(range.start);
                match topology {
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                        max_index_quads = max_index_quads.max(count / 4);
                    }
                    PrimitiveTopology::TriangleFan => {
                        max_fan_verts = max_fan_verts.max(count);
                    }
                    _ => {}
                }
            }
            self.oit_draws.push(PreparedDraw {
                slot: cmd.slot as usize,
                stage: cmd.stage as usize,
                modifier_stack: cmd.modifier_stack,
                topology,
                draw: cmd.draw.clone(),
                source: cmd.source.clone(),
                set1_bind_group: None,
            });
        }
        if self.runs.is_empty()
            && self.oit_runs.is_empty()
            && self.draws.is_empty()
            && self.oit_draws.is_empty()
        {
            if let Some(blocks) = vanilla_blocks {
                self.last_vanilla_blocks.copy_from_slice(blocks);
                upload(&self.vanilla, blocks);
                let pack_blocks = pack_vanilla_blocks(blocks);
                upload(&self.pack_vanilla, &pack_blocks);
            }
            return 0;
        }
        self.prepare_set1_bindings(device, upload, batch);
        if let Some(last) = self.runs.last() {
            let quads = last.quads.end;
            self.vertices.upload(
                device,
                upload,
                "Mod pipeline quads",
                bytemuck::cast_slice(&batch.vertices[..quads as usize * 4]),
                4 * std::mem::size_of::<PipelineVertex>() as u64,
                0,
            );
            max_index_quads = max_index_quads.max(self.vertices.index_quads(quads));
        }
        if let Some(last) = self.oit_runs.last() {
            let quads = last.quads.end;
            self.oit_vertices.upload(
                device,
                upload,
                "Mod pipeline OIT quads",
                bytemuck::cast_slice(&batch.oit_vertices[..quads as usize * 4]),
                4 * std::mem::size_of::<PipelineVertex>() as u64,
                0,
            );
            max_index_quads = max_index_quads.max(self.oit_vertices.index_quads(quads));
        }
        upload_transient_bytes(
            &mut self.transient_vbuf,
            device,
            upload,
            "Mod pipeline transient vertices",
            wgpu::BufferUsages::VERTEX,
            batch.transient_vertices,
        );
        upload_transient_bytes(
            &mut self.transient_ibuf,
            device,
            upload,
            "Mod pipeline transient indices",
            wgpu::BufferUsages::INDEX,
            batch.transient_indices,
        );
        upload_transient_bytes(
            &mut self.oit_transient_vbuf,
            device,
            upload,
            "Mod pipeline OIT transient vertices",
            wgpu::BufferUsages::VERTEX,
            batch.oit_transient_vertices,
        );
        upload_transient_bytes(
            &mut self.oit_transient_ibuf,
            device,
            upload,
            "Mod pipeline OIT transient indices",
            wgpu::BufferUsages::INDEX,
            batch.oit_transient_indices,
        );
        if max_fan_verts >= 3 {
            self.fan_indices
                .ensure_with_upload(device, upload, max_fan_verts);
        }
        let fallback;
        let blocks = match vanilla_blocks {
            Some(b) => b,
            None => {
                fallback = fallback_vanilla_blocks(view_proj, viewport, batch.time);
                &fallback
            }
        };
        self.last_vanilla_blocks.copy_from_slice(blocks);
        upload(&self.vanilla, blocks);
        let pack_blocks = pack_vanilla_blocks(blocks);
        upload(&self.pack_vanilla, &pack_blocks);
        max_index_quads
    }

    /// Maximum quad index count needed by `draws` for an offscreen pass.
    pub fn offscreen_needed_quads(&self, draws: &[PipelineDraw]) -> u32 {
        let mut max_quads = 0u32;
        for cmd in draws {
            let Some(Some(compiled)) = self.pipelines.get(cmd.slot as usize) else {
                continue;
            };
            if let DrawRange::Vertices(ref range) = cmd.draw.range
                && matches!(
                    compiled.state.topology,
                    PrimitiveTopology::Quads | PrimitiveTopology::Lines
                )
            {
                max_quads = max_quads.max(range.end.saturating_sub(range.start) / 4);
            }
        }
        max_quads
    }

    /// Ensure an offscreen `wgpu::RenderPipeline` variant exists for `(slot, color_formats, depth_format)`.
    fn ensure_offscreen_pipeline(
        &mut self,
        device: &wgpu::Device,
        slot: usize,
        color_formats: &[wgpu::TextureFormat],
        depth_format: Option<wgpu::TextureFormat>,
    ) {
        let key = (slot, color_formats.to_vec(), depth_format);
        if self.offscreen_pipelines.contains_key(&key) {
            return;
        }
        let compiled_opt = self.pipelines.get(slot).and_then(Option::as_ref);
        let built = compiled_opt.and_then(|c| {
            let target = TargetState {
                color_formats,
                depth_format,
                force_depth_attachment: depth_format.is_some(),
                standard_z: false,
                write_mask: wgpu::ColorWrites::ALL,
            };
            match Self::build_render_pipeline(
                device,
                &format!("{}/offscreen", c.id),
                &c.pipeline_layout,
                &c.vertex_shader,
                &c.fragment_shader,
                &c.vertex_layouts,
                c.state,
                &c.color_targets,
                target,
            ) {
                Ok(pipeline) => Some(pipeline),
                Err(error) => {
                    log::warn!(
                        "Sunrise: mod pipeline slot {slot} refused for offscreen target ({color_formats:?}, {depth_format:?}): {error}"
                    );
                    None
                }
            }
        });
        self.offscreen_pipelines.insert(key, built);
    }

    /// Record an offscreen render pass targeting `data.target` into `encoder`.
    ///
    /// Overrides `Globals.ScreenSize`, optional `Projection.ProjMat`, and optional
    /// `DynamicTransforms.ModelViewMat` in command-buffer order via `copy_buffer_to_buffer`
    /// and restores the main pass's vanilla uniform blocks immediately after the pass ends.
    pub fn execute_offscreen_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        indices: &wgpu::Buffer,
        data: &OffscreenPassData<'_>,
    ) {
        let upload = Upload::Queue(queue);
        let mut max_fan_verts = 0u32;
        for cmd in data.draws {
            let slot = cmd.slot as usize;
            self.ensure_offscreen_pipeline(
                device,
                slot,
                &data.target.color_formats,
                data.target.depth_format,
            );
            if let Some(Some(compiled)) = self.pipelines.get(slot)
                && compiled.state.topology == PrimitiveTopology::TriangleFan
                && let DrawRange::Vertices(ref range) = cmd.draw.range
            {
                max_fan_verts = max_fan_verts.max(range.end.saturating_sub(range.start));
            }
        }
        if max_fan_verts >= 3 {
            self.fan_indices
                .ensure_with_upload(device, upload, max_fan_verts);
        }
        // Per-pass immutable GPU buffers so multiple offscreen passes in one CommandEncoder
        // (e.g. a multi-pass frame graph) never clobber each other's vertices, indices, or set-1
        // staging data.
        let transient_vbuf = (!data.transient_vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod offscreen transient vertices"),
                contents: data.transient_vertices,
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let transient_ibuf = (!data.transient_indices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod offscreen transient indices"),
                contents: data.transient_indices,
                usage: wgpu::BufferUsages::INDEX,
            })
        });

        // Stage per-draw set-1 inline uniforms and texel buffers into dedicated offscreen
        // buffers so main-pass buffers are never overwritten before submission.
        let ubo_align = (device.limits().min_uniform_buffer_offset_alignment as usize).max(16);
        let storage_align = (device.limits().min_storage_buffer_offset_alignment as usize).max(4);
        let mut inline_staging: Vec<u8> = Vec::new();
        let mut texel_staging: Vec<u8> = Vec::new();
        let mut staged_meta: Vec<Option<(Vec<(u32, u64)>, Vec<(u64, u64)>)>> =
            Vec::with_capacity(data.draws.len());

        for cmd in data.draws {
            let Some(Some(compiled)) = self.pipelines.get(cmd.slot as usize) else {
                staged_meta.push(None);
                continue;
            };
            if compiled.set1_layout.is_none() {
                staged_meta.push(None);
                continue;
            }
            let Some(bindings) = &cmd.bindings else {
                staged_meta.push(None);
                continue;
            };
            if bindings.entries.len() != compiled.set1_entries.len() {
                staged_meta.push(None);
                continue;
            }
            let mut inline_spans = Vec::new();
            let mut texel_spans = Vec::new();
            let mut ok = true;
            for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                match (entry, spec.kind) {
                    (
                        ResolvedSet1Entry::InlineUniform { byte_range },
                        BindingKind::UniformBuffer { min_size },
                    ) => {
                        let Some(slice) = data.inline_uniforms.get(byte_range.clone()) else {
                            ok = false;
                            break;
                        };
                        let start = align_up_usize(inline_staging.len(), ubo_align);
                        inline_staging.resize(start, 0);
                        let padded_len = align_up_usize(slice.len().max(min_size as usize), 16);
                        inline_staging.extend_from_slice(slice);
                        inline_staging.resize(start + padded_len, 0);
                        inline_spans.push((start as u32, padded_len as u64));
                    }
                    (
                        ResolvedSet1Entry::TexelBuffer { byte_range },
                        BindingKind::TexelBuffer { .. },
                    ) => {
                        let Some(slice) = data.texel_data.get(byte_range.clone()) else {
                            ok = false;
                            break;
                        };
                        if slice.is_empty() || !slice.len().is_multiple_of(4) {
                            ok = false;
                            break;
                        }
                        let start = align_up_usize(texel_staging.len(), storage_align);
                        texel_staging.resize(start, 0);
                        texel_staging.extend_from_slice(slice);
                        texel_spans.push((start as u64, slice.len() as u64));
                    }
                    _ => {}
                }
            }
            if ok && let Some((_, pc_size)) = compiled.push_constant_slot {
                if let Some(ref pc_range) = bindings.push_constants
                    && let Some(slice) = data.inline_uniforms.get(pc_range.clone())
                {
                    let start = align_up_usize(inline_staging.len(), ubo_align);
                    inline_staging.resize(start, 0);
                    let padded_len = align_up_usize(slice.len().max(pc_size as usize), 16);
                    inline_staging.extend_from_slice(slice);
                    inline_staging.resize(start + padded_len, 0);
                    inline_spans.push((start as u32, padded_len as u64));
                } else {
                    ok = false;
                }
            }
            staged_meta.push(ok.then_some((inline_spans, texel_spans)));
        }

        let inline_buf = (!inline_staging.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod offscreen inline uniforms"),
                contents: &inline_staging,
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });
        let texel_buf = (!texel_staging.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod offscreen texel buffers"),
                contents: &texel_staging,
                usage: wgpu::BufferUsages::STORAGE,
            })
        });

        let mut prepared_draws: Vec<PreparedDraw> = Vec::with_capacity(data.draws.len());
        for (cmd, meta) in data.draws.iter().zip(staged_meta) {
            let Some(Some(compiled)) = self.pipelines.get(cmd.slot as usize) else {
                continue;
            };
            let set1_bind_group = if let Some(ref set1_layout) = compiled.set1_layout {
                let (Some(bindings), Some((inline_spans, texel_spans))) = (&cmd.bindings, meta)
                else {
                    continue;
                };
                let mut dynamic_offsets: Vec<u32> = Vec::new();
                let mut bg_entries: Vec<wgpu::BindGroupEntry<'_>> = Vec::new();
                let mut inline_i = 0usize;
                let mut texel_i = 0usize;
                let mut valid = true;
                for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                    match (entry, spec.kind) {
                        (
                            ResolvedSet1Entry::UniformBuffer {
                                buffer, byte_len, ..
                            },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            dynamic_offsets.push(0);
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(*byte_len),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::InlineUniform { .. },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            let (offset, size) = inline_spans[inline_i];
                            inline_i += 1;
                            dynamic_offsets.push(offset);
                            let Some(buf) = inline_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::Texture { view, .. },
                            BindingKind::CombinedImageSampler { filter, address },
                        ) => {
                            let sampler = &self.mod_samplers[sampler_filter_idx(filter)]
                                [sampler_address_idx(address)];
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            });
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding + 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            });
                        }
                        (
                            ResolvedSet1Entry::TexelBuffer { .. },
                            BindingKind::TexelBuffer { .. },
                        ) => {
                            let (offset, size) = texel_spans[texel_i];
                            texel_i += 1;
                            let Some(buf) = texel_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if valid && let Some((pc_binding, _)) = compiled.push_constant_slot {
                    let (offset, size) = inline_spans[inline_i];
                    dynamic_offsets.push(offset);
                    if let Some(buf) = inline_buf.as_ref() {
                        bg_entries.push(wgpu::BindGroupEntry {
                            binding: pc_binding,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: buf,
                                offset: 0,
                                size: wgpu::BufferSize::new(size),
                            }),
                        });
                    } else {
                        valid = false;
                    }
                }
                if !valid {
                    continue;
                }
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Mod offscreen set-1"),
                    layout: set1_layout,
                    entries: &bg_entries,
                });
                Some((bg, dynamic_offsets))
            } else {
                None
            };
            prepared_draws.push(PreparedDraw {
                slot: cmd.slot as usize,
                stage: cmd.stage as usize,
                modifier_stack: cmd.modifier_stack,
                topology: compiled.state.topology,
                draw: cmd.draw.clone(),
                source: cmd.source.clone(),
                set1_bind_group,
            });
        }

        // Patch vanilla uniform blocks for this offscreen pass (ScreenSize, optional ProjMat,
        // optional ModelViewMat) in command-buffer order so consecutive passes never clobber.
        let mut patched = self.last_vanilla_blocks;
        let screen_size = [data.target.size[0] as f32, data.target.size[1] as f32];
        patched[32..40].copy_from_slice(bytemuck::cast_slice(&screen_size));
        if let Some(proj) = data.pass.projection {
            patched[256..320].copy_from_slice(bytemuck::cast_slice(&proj));
        }
        if let Some(view) = data.pass.view {
            patched[512..576].copy_from_slice(bytemuck::cast_slice(&view));
        }
        let patched_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod offscreen vanilla override"),
            contents: &patched,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        let restore_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod offscreen vanilla restore"),
            contents: &self.last_vanilla_blocks,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        encoder.copy_buffer_to_buffer(&patched_buf, 0, &self.vanilla, 0, 1024);

        let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = data
            .target
            .color_views
            .iter()
            .enumerate()
            .map(|(idx, view)| {
                let load = match data.pass.clear_color.get(idx).copied().flatten() {
                    Some([r, g, b, a]) => wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(r),
                        g: f64::from(g),
                        b: f64::from(b),
                        a: f64::from(a),
                    }),
                    None => wgpu::LoadOp::Load,
                };
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();

        let depth_stencil_attachment = data.target.depth_view.as_ref().map(|view| {
            let depth_load = match data.pass.clear_depth {
                Some(d) => wgpu::LoadOp::Clear(d),
                None => wgpu::LoadOp::Load,
            };
            let stencil_ops = data.target.has_stencil.then(|| wgpu::Operations {
                load: match data.pass.clear_stencil {
                    Some(s) => wgpu::LoadOp::Clear(s),
                    None => wgpu::LoadOp::Load,
                },
                store: wgpu::StoreOp::Store,
            });
            wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops,
            }
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Mod offscreen pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment,
                ..Default::default()
            });
            for cmd in &prepared_draws {
                let Some(Some(compiled)) = self.pipelines.get(cmd.slot) else {
                    continue;
                };
                let key = (
                    cmd.slot,
                    data.target.color_formats.clone(),
                    data.target.depth_format,
                );
                let Some(Some(pipeline)) = self.offscreen_pipelines.get(&key) else {
                    continue;
                };
                rpass.set_pipeline(pipeline);
                rpass.set_bind_group(0, &compiled.group, &[]);
                if compiled.set1_layout.is_some() {
                    let Some((ref bg, ref offsets)) = cmd.set1_bind_group else {
                        continue;
                    };
                    rpass.set_bind_group(1, bg, offsets);
                }
                self.issue_prepared_draw(
                    &mut rpass,
                    indices,
                    transient_vbuf.as_ref(),
                    transient_ibuf.as_ref(),
                    cmd,
                );
            }
        }

        encoder.copy_buffer_to_buffer(&restore_buf, 0, &self.vanilla, 0, 1024);
    }

    /// Whether any prepared runs or unified draws fall in stages `first..=last`.
    pub fn has_runs(&self, first: usize, last: usize) -> bool {
        self.runs
            .iter()
            .any(|run| first <= run.stage && run.stage <= last)
            || self
                .draws
                .iter()
                .any(|draw| first <= draw.stage && draw.stage <= last)
    }

    /// Whether any prepared runs or unified draws fall in stages `first..=last` for slots not
    /// in `bound_slots`.
    pub fn has_unbound_runs(&self, first: usize, last: usize, bound_slots: &[usize]) -> bool {
        self.runs
            .iter()
            .any(|run| first <= run.stage && run.stage <= last && !bound_slots.contains(&run.slot))
            || self.draws.iter().any(|draw| {
                first <= draw.stage && draw.stage <= last && !bound_slots.contains(&draw.slot)
            })
    }

    /// Append uploaded vertex chunks (for both transient draws and retained `MeshRef` draws)
    /// whose slot is bound to `PackPost` into `out`, reusing `out`'s existing allocation.
    pub fn pack_bound_chunks_into(
        &self,
        indices: &wgpu::Buffer,
        bound_slots: &[usize],
        out: &mut Vec<super::pack_post::ModDrawChunk>,
    ) {
        if (self.runs.is_empty() && self.draws.is_empty()) || bound_slots.is_empty() {
            return;
        }
        for run in &self.runs {
            if !bound_slots.contains(&run.slot) {
                continue;
            }
            for (buffer, quads) in self.vertices.ranges(0, run.quads.clone()) {
                out.push(super::pack_post::ModDrawChunk::quads(
                    run.slot,
                    buffer.clone(),
                    indices.clone(),
                    quads,
                ));
            }
        }
        for cmd in &self.draws {
            if !bound_slots.contains(&cmd.slot) || cmd.draw.instances.is_empty() {
                continue;
            }
            let (vertex_buffers, explicit_index) = match &cmd.source {
                PipelineDrawSource::Transient {
                    vertex_ranges,
                    index_range,
                } => {
                    let Some(vbuf) = &self.transient_vbuf else {
                        continue;
                    };
                    let vbs = vertex_ranges
                        .iter()
                        .map(|(binding, range)| (*binding, vbuf.clone(), range.clone()))
                        .collect();
                    let ib = index_range.as_ref().and_then(|(fmt, range)| {
                        self.transient_ibuf
                            .as_ref()
                            .map(|buf| (buf.clone(), wgpu_index_format(*fmt), range.clone()))
                    });
                    (vbs, ib)
                }
                PipelineDrawSource::Retained(resolved) => {
                    let vbs = resolved
                        .vertex_buffers
                        .iter()
                        .map(|(binding, buf, len)| (*binding, buf.clone(), 0..*len))
                        .collect();
                    let ib = resolved
                        .index_buffer
                        .as_ref()
                        .map(|(buf, fmt)| (buf.clone(), *fmt, 0..buf.size()));
                    (vbs, ib)
                }
            };
            let kind = match &cmd.draw.range {
                DrawRange::Vertices(range) => {
                    let count = range.end.saturating_sub(range.start);
                    if count == 0 {
                        continue;
                    }
                    match cmd.topology {
                        PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                            let quads = count / 4;
                            if quads == 0 {
                                continue;
                            }
                            super::pack_post::ModDrawKind::Indexed {
                                buffer: indices.clone(),
                                format: wgpu::IndexFormat::Uint32,
                                indices: 0..quads * 6,
                                base_vertex: range.start as i32,
                            }
                        }
                        PrimitiveTopology::TriangleFan => {
                            let Some(fan_buf) = self.fan_indices.buffer() else {
                                continue;
                            };
                            let idx_count = cmd.topology.index_count(count);
                            if idx_count == 0 {
                                continue;
                            }
                            super::pack_post::ModDrawKind::Indexed {
                                buffer: fan_buf.clone(),
                                format: wgpu::IndexFormat::Uint32,
                                indices: 0..idx_count,
                                base_vertex: range.start as i32,
                            }
                        }
                        _ => super::pack_post::ModDrawKind::Vertices {
                            vertices: range.clone(),
                        },
                    }
                }
                DrawRange::Indexed {
                    indices: idx_range,
                    base_vertex,
                } => {
                    let Some((buf, format, _byte_range)) = explicit_index else {
                        continue;
                    };
                    if idx_range.is_empty() {
                        continue;
                    }
                    super::pack_post::ModDrawKind::Indexed {
                        buffer: buf,
                        format,
                        indices: idx_range.clone(),
                        base_vertex: *base_vertex,
                    }
                }
                _ => continue,
            };
            out.push(super::pack_post::ModDrawChunk {
                slot: cmd.slot,
                vertex_buffers,
                kind,
                instances: cmd.draw.instances.clone(),
                mod_bind_group: cmd.set1_bind_group.clone(),
            });
        }
    }

    /// Stage all per-frame inline uniforms, push constants, and texel buffers from `batch` into
    /// growing GPU buffers and build/lookup cached set-1 `wgpu::BindGroup`s for each prepared draw.
    fn prepare_set1_bindings(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'_>,
    ) {
        let ubo_align = (device.limits().min_uniform_buffer_offset_alignment as usize).max(16);
        let storage_align = (device.limits().min_storage_buffer_offset_alignment as usize).max(4);
        self.inline_uniform_staging.clear();
        self.texel_staging.clear();

        struct StagedDraw {
            is_oit: bool,
            prepared_idx: usize,
            slot: usize,
            inline_spans: Vec<(u32, u64)>,
            texel_spans: Vec<(u64, u64)>,
        }
        let mut staged: Vec<(StagedDraw, &ResolvedDrawBindings)> = Vec::new();

        let mut stage_list = |is_oit: bool,
                              cmds: &[PipelineDraw],
                              inline_pool: &[u8],
                              texel_pool: &[u8]| {
            let mut prepared_idx = 0usize;
            for cmd in cmds {
                let valid = if is_oit {
                    cmd.stage < 3
                        && self
                            .pipelines
                            .get(cmd.slot as usize)
                            .and_then(Option::as_ref)
                            .is_some_and(|c| c.oit.is_some())
                } else {
                    self.pipelines
                        .get(cmd.slot as usize)
                        .and_then(Option::as_ref)
                        .is_some()
                };
                if !valid {
                    continue;
                }
                let idx = prepared_idx;
                prepared_idx += 1;
                let Some(compiled) = self
                    .pipelines
                    .get(cmd.slot as usize)
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                if compiled.set1_layout.is_none() {
                    continue;
                }
                let Some(bindings) = &cmd.bindings else {
                    continue;
                };
                if bindings.entries.len() != compiled.set1_entries.len() {
                    continue;
                }
                let mut inline_spans = Vec::new();
                let mut texel_spans = Vec::new();
                let mut ok = true;
                for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                    match (entry, spec.kind) {
                        (
                            ResolvedSet1Entry::InlineUniform { byte_range },
                            BindingKind::UniformBuffer { min_size },
                        ) => {
                            let Some(slice) = inline_pool.get(byte_range.clone()) else {
                                ok = false;
                                break;
                            };
                            let start =
                                align_up_usize(self.inline_uniform_staging.len(), ubo_align);
                            self.inline_uniform_staging.resize(start, 0);
                            let padded_len = align_up_usize(slice.len().max(min_size as usize), 16);
                            self.inline_uniform_staging.extend_from_slice(slice);
                            self.inline_uniform_staging.resize(start + padded_len, 0);
                            inline_spans.push((start as u32, padded_len as u64));
                        }
                        (
                            ResolvedSet1Entry::TexelBuffer { byte_range },
                            BindingKind::TexelBuffer { .. },
                        ) => {
                            let Some(slice) = texel_pool.get(byte_range.clone()) else {
                                ok = false;
                                break;
                            };
                            if slice.is_empty() || !slice.len().is_multiple_of(4) {
                                ok = false;
                                break;
                            }
                            let start = align_up_usize(self.texel_staging.len(), storage_align);
                            self.texel_staging.resize(start, 0);
                            self.texel_staging.extend_from_slice(slice);
                            texel_spans.push((start as u64, slice.len() as u64));
                        }
                        _ => {}
                    }
                }
                if ok && let Some((_, pc_size)) = compiled.push_constant_slot {
                    if let Some(ref pc_range) = bindings.push_constants
                        && let Some(slice) = inline_pool.get(pc_range.clone())
                    {
                        let start = align_up_usize(self.inline_uniform_staging.len(), ubo_align);
                        self.inline_uniform_staging.resize(start, 0);
                        let padded_len = align_up_usize(slice.len().max(pc_size as usize), 16);
                        self.inline_uniform_staging.extend_from_slice(slice);
                        self.inline_uniform_staging.resize(start + padded_len, 0);
                        inline_spans.push((start as u32, padded_len as u64));
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    staged.push((
                        StagedDraw {
                            is_oit,
                            prepared_idx: idx,
                            slot: cmd.slot as usize,
                            inline_spans,
                            texel_spans,
                        },
                        bindings,
                    ));
                }
            }
        };

        stage_list(false, batch.draws, batch.inline_uniforms, batch.texel_data);
        stage_list(
            true,
            batch.oit_draws,
            batch.oit_inline_uniforms,
            batch.oit_texel_data,
        );
        if staged.is_empty() {
            return;
        }

        if !self.inline_uniform_staging.is_empty() {
            let need = self.inline_uniform_staging.len() as u64;
            let grew = self
                .inline_uniform_buf
                .as_ref()
                .is_none_or(|b| b.size() < need);
            if grew {
                let cap = need.next_power_of_two().max(4096);
                self.inline_uniform_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Mod pipeline inline uniforms"),
                    size: cap,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.set1_bind_group_cache.clear();
            }
            if let Some(buf) = &self.inline_uniform_buf {
                upload.write(buf, 0, &self.inline_uniform_staging);
            }
        }

        if !self.texel_staging.is_empty() {
            let need = self.texel_staging.len() as u64;
            let grew = self
                .texel_storage_buf
                .as_ref()
                .is_none_or(|b| b.size() < need);
            if grew {
                let cap = need.next_power_of_two().max(4096);
                self.texel_storage_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Mod pipeline texel buffers"),
                    size: cap,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.set1_bind_group_cache.clear();
            }
            if let Some(buf) = &self.texel_storage_buf {
                upload.write(buf, 0, &self.texel_staging);
            }
        }

        for (sd, bindings) in staged {
            let Some(Some(compiled)) = self.pipelines.get(sd.slot) else {
                continue;
            };
            let Some(ref set1_layout) = compiled.set1_layout else {
                continue;
            };
            let mut key_resources = Vec::with_capacity(
                compiled.set1_entries.len() + usize::from(compiled.push_constant_slot.is_some()),
            );
            let mut dynamic_offsets: Vec<u32> = Vec::new();
            let mut inline_cursor = 0usize;
            let mut texel_cursor = 0usize;
            let mut valid = true;
            for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                match (entry, spec.kind) {
                    (
                        ResolvedSet1Entry::UniformBuffer { id, byte_len, .. },
                        BindingKind::UniformBuffer { .. },
                    ) => {
                        key_resources.push(Set1ResourceKey::RetainedUniform(*id, *byte_len));
                        dynamic_offsets.push(0);
                    }
                    (
                        ResolvedSet1Entry::InlineUniform { .. },
                        BindingKind::UniformBuffer { .. },
                    ) => {
                        let (offset, size) = sd.inline_spans[inline_cursor];
                        inline_cursor += 1;
                        key_resources.push(Set1ResourceKey::InlineUniform(size));
                        dynamic_offsets.push(offset);
                    }
                    (
                        ResolvedSet1Entry::Texture { view_id, .. },
                        BindingKind::CombinedImageSampler { .. },
                    ) => {
                        key_resources.push(Set1ResourceKey::Texture(*view_id));
                    }
                    (ResolvedSet1Entry::TexelBuffer { .. }, BindingKind::TexelBuffer { .. }) => {
                        let (offset, size) = sd.texel_spans[texel_cursor];
                        texel_cursor += 1;
                        key_resources.push(Set1ResourceKey::TexelBuffer(offset, size));
                    }
                    _ => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            if compiled.push_constant_slot.is_some() {
                let (offset, size) = sd.inline_spans[inline_cursor];
                key_resources.push(Set1ResourceKey::InlineUniform(size));
                dynamic_offsets.push(offset);
            }

            let cache_key = Set1CacheKey {
                slot: sd.slot,
                generation: self.generation,
                resources: key_resources,
            };
            let bind_group = if let Some(cached) = self.set1_bind_group_cache.get(&cache_key) {
                cached.clone()
            } else {
                let mut bg_entries: Vec<wgpu::BindGroupEntry<'_>> = Vec::new();
                let mut inline_i = 0usize;
                let mut texel_i = 0usize;
                for (entry, spec) in bindings.entries.iter().zip(&compiled.set1_entries) {
                    match (entry, spec.kind) {
                        (
                            ResolvedSet1Entry::UniformBuffer {
                                buffer, byte_len, ..
                            },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(*byte_len),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::InlineUniform { .. },
                            BindingKind::UniformBuffer { .. },
                        ) => {
                            let (_, size) = sd.inline_spans[inline_i];
                            inline_i += 1;
                            let Some(buf) = self.inline_uniform_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset: 0,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        (
                            ResolvedSet1Entry::Texture { view, .. },
                            BindingKind::CombinedImageSampler { filter, address },
                        ) => {
                            let sampler = &self.mod_samplers[sampler_filter_idx(filter)]
                                [sampler_address_idx(address)];
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            });
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding + 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            });
                        }
                        (
                            ResolvedSet1Entry::TexelBuffer { .. },
                            BindingKind::TexelBuffer { .. },
                        ) => {
                            let (offset, size) = sd.texel_spans[texel_i];
                            texel_i += 1;
                            let Some(buf) = self.texel_storage_buf.as_ref() else {
                                valid = false;
                                break;
                            };
                            bg_entries.push(wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: buf,
                                    offset,
                                    size: wgpu::BufferSize::new(size),
                                }),
                            });
                        }
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if valid && let Some((pc_binding, _)) = compiled.push_constant_slot {
                    let (_, size) = sd.inline_spans[inline_i];
                    if let Some(buf) = self.inline_uniform_buf.as_ref() {
                        bg_entries.push(wgpu::BindGroupEntry {
                            binding: pc_binding,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: buf,
                                offset: 0,
                                size: wgpu::BufferSize::new(size),
                            }),
                        });
                    } else {
                        valid = false;
                    }
                }
                if !valid {
                    continue;
                }
                let created = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Mod pipeline set-1"),
                    layout: set1_layout,
                    entries: &bg_entries,
                });
                self.set1_bind_group_cache
                    .insert(cache_key, created.clone());
                created
            };

            let target = if sd.is_oit {
                self.oit_draws.get_mut(sd.prepared_idx)
            } else {
                self.draws.get_mut(sd.prepared_idx)
            };
            if let Some(prepared) = target {
                prepared.set1_bind_group = Some((bind_group, dynamic_offsets));
            }
        }
    }

    /// Draws the prepared runs and unified draws of stages `first..=last` in the order queued,
    /// with `indices` (6 a quad, `u32`), skipping slots in `skip_slots` (e.g. slots bound to a
    /// shader pack's translated mod pass). Leaves the pass with its own pipeline, bind group 0
    /// and buffers.
    pub fn draw_excluding(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
        skip_slots: &[usize],
    ) {
        self.draw_runs(
            pass,
            indices,
            false,
            self.runs
                .iter()
                .filter(|run| {
                    first <= run.stage && run.stage <= last && !skip_slots.contains(&run.slot)
                })
                .map(|run| (run, run.quads.clone())),
        );
        self.draw_prepared_cmds(
            pass,
            indices,
            false,
            self.draws.iter().filter(|cmd| {
                first <= cmd.stage && cmd.stage <= last && !skip_slots.contains(&cmd.slot)
            }),
        );
    }

    /// Same as [`Self::draw_excluding`], targeting a shader pack's `colortex0` + `depthtex0`
    /// prepared with [`Self::ensure_pack_pipelines`], skipping slots bound to translated pack
    /// programs.
    pub fn draw_pack(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        first: usize,
        last: usize,
        bound_slots: &[usize],
    ) {
        self.draw_runs(
            pass,
            indices,
            true,
            self.runs
                .iter()
                .filter(|run| {
                    first <= run.stage && run.stage <= last && !bound_slots.contains(&run.slot)
                })
                .map(|run| (run, run.quads.clone())),
        );
        self.draw_prepared_cmds(
            pass,
            indices,
            true,
            self.draws.iter().filter(|cmd| {
                first <= cmd.stage && cmd.stage <= last && !bound_slots.contains(&cmd.slot)
            }),
        );
    }

    /// Draw prepared standard-pass pipeline runs for the given run-index ranges.
    pub fn draw_prepared_runs(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        ranges: &[std::ops::Range<u32>],
    ) {
        self.draw_runs(
            pass,
            indices,
            false,
            ranges.iter().flat_map(|r| {
                let start = (r.start as usize).min(self.runs.len());
                let end = (r.end as usize).min(self.runs.len());
                self.runs[start..end]
                    .iter()
                    .map(|run| (run, run.quads.clone()))
            }),
        );
    }

    /// Draw prepared standard-pass unified pipeline draws (`draw_mesh`, `draw_vertices`,
    /// `pipeline_quads`) for the given draw-index ranges.
    pub fn draw_prepared_draws(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        ranges: &[std::ops::Range<u32>],
    ) {
        self.draw_prepared_cmds(
            pass,
            indices,
            false,
            ranges.iter().flat_map(|r| {
                let start = (r.start as usize).min(self.draws.len());
                let end = (r.end as usize).min(self.draws.len());
                self.draws[start..end].iter()
            }),
        );
    }

    /// Draw all OIT mod-pipeline runs and unified draws for `stage` into the active OIT pass.
    pub(super) fn draw_oit(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        indices: &wgpu::Buffer,
    ) {
        let stage_idx = stage as usize;
        self.draw_oit_runs(
            pass,
            stage_idx,
            indices,
            self.oit_runs.iter().filter(|run| run.stage == stage_idx),
        );
        self.draw_oit_cmds(
            pass,
            stage_idx,
            indices,
            self.oit_draws.iter().filter(|cmd| cmd.stage == stage_idx),
        );
    }

    /// Draw prepared OIT mod-pipeline runs for `stage` and the given run-index ranges.
    pub(super) fn draw_prepared_oit(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        indices: &wgpu::Buffer,
        ranges: &[std::ops::Range<u32>],
    ) {
        let stage_idx = stage as usize;
        self.draw_oit_runs(
            pass,
            stage_idx,
            indices,
            ranges.iter().flat_map(|r| {
                let start = (r.start as usize).min(self.oit_runs.len());
                let end = (r.end as usize).min(self.oit_runs.len());
                self.oit_runs[start..end]
                    .iter()
                    .filter(move |run| run.stage == stage_idx)
            }),
        );
    }

    /// Draw prepared OIT unified pipeline draws for `stage` and the given draw-index ranges.
    pub(super) fn draw_prepared_oit_draws(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        stage: super::oit::Stage,
        indices: &wgpu::Buffer,
        ranges: &[std::ops::Range<u32>],
    ) {
        let stage_idx = stage as usize;
        self.draw_oit_cmds(
            pass,
            stage_idx,
            indices,
            ranges.iter().flat_map(|r| {
                let start = (r.start as usize).min(self.oit_draws.len());
                let end = (r.end as usize).min(self.oit_draws.len());
                self.oit_draws[start..end]
                    .iter()
                    .filter(move |cmd| cmd.stage == stage_idx)
            }),
        );
    }

    fn issue_prepared_draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        quad_indices: &wgpu::Buffer,
        transient_vbuf: Option<&wgpu::Buffer>,
        transient_ibuf: Option<&wgpu::Buffer>,
        cmd: &PreparedDraw,
    ) {
        if cmd.draw.instances.is_empty() {
            return;
        }
        match &cmd.source {
            PipelineDrawSource::Transient {
                vertex_ranges,
                index_range,
            } => {
                let Some(vbuf) = transient_vbuf else {
                    return;
                };
                for (binding, range) in vertex_ranges {
                    if range.end > vbuf.size() {
                        return;
                    }
                    pass.set_vertex_buffer(*binding, vbuf.slice(range.clone()));
                }
                match &cmd.draw.range {
                    DrawRange::Vertices(range) => {
                        self.issue_non_indexed_draw(
                            pass,
                            quad_indices,
                            cmd.topology,
                            range.clone(),
                            cmd.draw.instances.clone(),
                        );
                    }
                    DrawRange::Indexed {
                        indices,
                        base_vertex,
                    } => {
                        let Some((fmt, ib_range)) = index_range else {
                            return;
                        };
                        let Some(ibuf) = transient_ibuf else {
                            return;
                        };
                        if ib_range.end > ibuf.size() || indices.is_empty() {
                            return;
                        }
                        pass.set_index_buffer(
                            ibuf.slice(ib_range.clone()),
                            wgpu_index_format(*fmt),
                        );
                        pass.draw_indexed(
                            indices.clone(),
                            *base_vertex,
                            cmd.draw.instances.clone(),
                        );
                    }
                    _ => {}
                }
            }
            PipelineDrawSource::Retained(resolved) => {
                for (binding, buf, len) in &resolved.vertex_buffers {
                    pass.set_vertex_buffer(*binding, buf.slice(0..*len));
                }
                match &cmd.draw.range {
                    DrawRange::Vertices(range) => {
                        self.issue_non_indexed_draw(
                            pass,
                            quad_indices,
                            cmd.topology,
                            range.clone(),
                            cmd.draw.instances.clone(),
                        );
                    }
                    DrawRange::Indexed {
                        indices,
                        base_vertex,
                    } => {
                        let Some((ibuf, fmt)) = &resolved.index_buffer else {
                            return;
                        };
                        if indices.is_empty() {
                            return;
                        }
                        pass.set_index_buffer(ibuf.slice(..), *fmt);
                        pass.draw_indexed(
                            indices.clone(),
                            *base_vertex,
                            cmd.draw.instances.clone(),
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn issue_non_indexed_draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        quad_indices: &wgpu::Buffer,
        topology: PrimitiveTopology,
        range: std::ops::Range<u32>,
        instances: std::ops::Range<u32>,
    ) {
        let count = range.end.saturating_sub(range.start);
        if count == 0 || instances.is_empty() {
            return;
        }
        match topology {
            PrimitiveTopology::Quads | PrimitiveTopology::Lines => {
                let quads = count / 4;
                if quads == 0 {
                    return;
                }
                pass.set_index_buffer(quad_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..quads * 6, range.start as i32, instances);
            }
            PrimitiveTopology::TriangleFan => {
                let Some(fan_buf) = self.fan_indices.buffer() else {
                    return;
                };
                let idx_count = topology.index_count(count);
                if idx_count == 0 {
                    return;
                }
                pass.set_index_buffer(fan_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..idx_count, range.start as i32, instances);
            }
            _ => {
                pass.draw(range, instances);
            }
        }
    }

    fn draw_prepared_cmds<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        pack: bool,
        cmds: impl Iterator<Item = &'a PreparedDraw>,
    ) {
        let pack_list = if pack {
            let Some((_, list)) = &self.pack_pipelines else {
                return;
            };
            Some(list)
        } else {
            None
        };
        for cmd in cmds {
            let Some(Some(compiled)) = self.pipelines.get(cmd.slot) else {
                continue;
            };
            if compiled.state.depth_stencil.is_none() {
                continue;
            }
            let pipeline = match pack_list {
                Some(list) => match list.get(cmd.slot) {
                    Some(Some(p)) => p,
                    _ => continue,
                },
                None => {
                    if cmd.modifier_stack == ModifierStacks::EMPTY {
                        &compiled.pipeline
                    } else {
                        match compiled.variants.get(cmd.modifier_stack as usize) {
                            Some(Some(CachedVariant::Base)) => &compiled.pipeline,
                            Some(Some(CachedVariant::Compiled(p))) => p,
                            _ => continue,
                        }
                    }
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(
                0,
                if pack {
                    &compiled.pack_group
                } else {
                    &compiled.group
                },
                &[],
            );
            if compiled.set1_layout.is_some() {
                let Some((ref bg, ref offsets)) = cmd.set1_bind_group else {
                    continue;
                };
                pass.set_bind_group(1, bg, offsets);
            }
            self.issue_prepared_draw(
                pass,
                indices,
                self.transient_vbuf.as_ref(),
                self.transient_ibuf.as_ref(),
                cmd,
            );
        }
    }

    fn draw_oit_cmds<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        stage_idx: usize,
        indices: &wgpu::Buffer,
        cmds: impl Iterator<Item = &'a PreparedDraw>,
    ) {
        for cmd in cmds {
            let Some(Some(compiled)) = self.pipelines.get(cmd.slot) else {
                continue;
            };
            let Some(oit) = &compiled.oit else {
                continue;
            };
            let oit_stage = &oit.stages[stage_idx];
            let Some(group) = &oit_stage.group else {
                continue;
            };
            let pipeline = if cmd.modifier_stack == ModifierStacks::EMPTY {
                &oit_stage.pipeline
            } else {
                match oit_stage.variants.get(cmd.modifier_stack as usize) {
                    Some(Some(CachedVariant::Base)) => &oit_stage.pipeline,
                    Some(Some(CachedVariant::Compiled(p))) => p,
                    _ => continue,
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            if compiled.set1_layout.is_some() {
                let Some((ref bg, ref offsets)) = cmd.set1_bind_group else {
                    continue;
                };
                pass.set_bind_group(1, bg, offsets);
            }
            self.issue_prepared_draw(
                pass,
                indices,
                self.oit_transient_vbuf.as_ref(),
                self.oit_transient_ibuf.as_ref(),
                cmd,
            );
        }
    }

    fn draw_oit_runs<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        stage_idx: usize,
        indices: &wgpu::Buffer,
        runs: impl Iterator<Item = &'a Run>,
    ) {
        let mut index_bound = false;
        for run in runs {
            let Some(Some(compiled)) = self.pipelines.get(run.slot) else {
                continue;
            };
            let Some(oit) = &compiled.oit else {
                continue;
            };
            let oit_stage = &oit.stages[stage_idx];
            let Some(group) = &oit_stage.group else {
                continue;
            };
            let pipeline = if run.modifier_stack == ModifierStacks::EMPTY {
                &oit_stage.pipeline
            } else {
                match oit_stage.variants.get(run.modifier_stack as usize) {
                    Some(Some(CachedVariant::Base)) => &oit_stage.pipeline,
                    Some(Some(CachedVariant::Compiled(p))) => p,
                    _ => continue,
                }
            };
            if !index_bound {
                index_bound = true;
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            for (buffer, range) in self.oit_vertices.ranges(0, run.quads.clone()) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(range.start * 6..range.end * 6, 0, 0..1);
            }
        }
    }

    fn draw_runs<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        pack: bool,
        runs: impl Iterator<Item = (&'a Run, std::ops::Range<u32>)>,
    ) {
        let pack_list = if pack {
            let Some((_, list)) = &self.pack_pipelines else {
                return;
            };
            Some(list)
        } else {
            None
        };
        let mut index_bound = false;
        for (run, range) in runs {
            let Some(Some(compiled)) = self.pipelines.get(run.slot) else {
                continue;
            };
            if compiled.state.depth_stencil.is_none() {
                continue;
            }
            let pipeline = match pack_list {
                Some(list) => match list.get(run.slot) {
                    Some(Some(p)) => p,
                    _ => continue,
                },
                None => {
                    if run.modifier_stack == ModifierStacks::EMPTY {
                        &compiled.pipeline
                    } else {
                        match compiled.variants.get(run.modifier_stack as usize) {
                            Some(Some(CachedVariant::Base)) => &compiled.pipeline,
                            Some(Some(CachedVariant::Compiled(p))) => p,
                            _ => continue,
                        }
                    }
                }
            };
            if !index_bound {
                index_bound = true;
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            pass.set_pipeline(pipeline);
            pass.set_bind_group(
                0,
                if pack {
                    &compiled.pack_group
                } else {
                    &compiled.group
                },
                &[],
            );
            for (buffer, range) in self.vertices.ranges(0, range) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(range.start * 6..range.end * 6, 0, 0..1);
            }
        }
    }
}

fn gpu_refusal(built: &Built, error: &str) -> String {
    format!(
        "render pipeline {} from mod {} refused by the GPU: {error}",
        built.id, built.owner
    )
}

impl super::Renderer {
    /// Builds the mods' pipelines of the load this renderer is made for, before the load is
    /// published: the required ones the GPU refuses are the error, by id. A textured pipeline's
    /// sprite is looked up with `sprite` (its `[u0, v0, u1, v1]` in the entity atlas, the 2D
    /// atlas world primitives draw from), and a sprite it doesn't find refuses that pipeline.
    pub fn build_mod_pipelines(
        &mut self,
        set: &Pipelines,
        sprite: &dyn Fn(&str) -> Option<[f32; 4]>,
    ) -> Result<(), String> {
        let textures = TextureSources {
            atlas: &self.entity_atlas,
            sprite,
        };
        self.world_primitives
            .as_mut()
            .expect("world primitives")
            .pipelines
            .build(&self.device, set, Some(textures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::world_primitives::{self, STAGES, WorldPrimitives};
    use glam::Vec3;
    use mod_api::{BlendFunction, Identifier, RenderPipeline, WorldDepth};

    const SIZE: u32 = 64;
    /// The frame before any quad: a quarter grey, stored as 64 in 8 bits.
    const CLEAR: f64 = 64.0 / 255.0;

    #[test]
    fn pack_clip_matrix_flips_y_and_inverts_reversed_z_depth() {
        let proj = Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.5, 0.05);
        let adapted = pack_clip_matrix(proj);
        for p in [
            glam::Vec4::new(0.0, 0.0, -0.05, 1.0),
            glam::Vec4::new(0.25, 0.5, -4.0, 1.0),
            glam::Vec4::new(-1.5, -0.75, -20.0, 1.0),
        ] {
            let orig = proj * p;
            let got = adapted * p;
            assert_eq!(got.x, orig.x);
            assert_eq!(got.y, -orig.y);
            assert_eq!(got.z, orig.w - orig.z);
            assert_eq!(got.w, orig.w);
        }
    }

    const VSH: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>
in vec3 Position;
in vec2 UV0;
in vec4 Color;
out vec2 texCoord0;
out vec4 vertexColor;
void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
";

    const PASS: &str = "\
#version 330
#include <minecraft:dynamictransforms.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor * ColorModulator;
}
";

    const HALF_GAMMA: &str = "\
#version 330
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    // Vanilla gamma value 0.7297 (~186/255), whose linear 2.2 power is 0.5.
    fragColor = vec4(vec3(0.72974), 1.0);
}
";

    const INPUTS: &str = "\
#version 330
#include <minecraft:globals.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vec4(step(0.5, texCoord0.x), GameTime, step(0.5, texCoord0.y), 1.0);
}
";

    fn stub_asset(file: &str) -> Option<String> {
        let rest = file.strip_prefix("assets/")?;
        let (ns, path) = rest.split_once('/')?;
        sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned)
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let (_, device, queue) = crate::gfx::test_gpu::optional_device()?;
        Some((device, queue))
    }

    /// The load of `generation` with one optional pipeline for each fragment shader, in slot order.
    fn load(generation: u64, shaders: &[(&str, PipelineBlend, WorldDepth)]) -> Pipelines {
        let registered: Vec<_> = (0..shaders.len())
            .map(|slot| {
                let id = Identifier::parse(&format!("test:p{slot}")).unwrap();
                let shader = Identifier::parse(&format!("test:core/p{slot}")).unwrap();
                RenderPipeline::new(id, shader, shaders[slot].1, shaders[slot].2).optional()
            })
            .collect();
        Pipelines::build(&registered, generation, |file| {
            if let Some(rest) = file.strip_prefix("assets/test/shaders/core/p") {
                if rest.ends_with(".vsh") {
                    return Ok(Some(VSH.to_owned()));
                }
                if let Some(num) = rest.strip_suffix(".fsh") {
                    let slot: usize = num.parse().unwrap();
                    return Ok(Some(shaders[slot].0.to_owned()));
                }
            }
            Ok(stub_asset(file))
        })
        .unwrap()
    }

    fn view_proj() -> Mat4 {
        crate::gfx::world_projection(70.0, 1.0, 512.0)
    }

    /// A quad facing the camera `z` blocks ahead of it, over the pixels `x0..x1`, `y0..y1` of
    /// the frame (y down), its texture coordinates spanning 0 to 1 and `color` as stored bytes.
    fn quad([x0, y0, x1, y1]: [f32; 4], z: f32, color: [u8; 4]) -> [PipelineVertex; 4] {
        let depth = view_proj().project_point3(Vec3::new(0.0, 0.0, -z)).z;
        let inverse = view_proj().inverse();
        let at = |x: f32, y: f32, uv: [f32; 2]| PipelineVertex {
            pos: inverse
                .project_point3(Vec3::new(
                    x / SIZE as f32 * 2.0 - 1.0,
                    1.0 - y / SIZE as f32 * 2.0,
                    depth,
                ))
                .to_array(),
            uv,
            color: u32::from_le_bytes(color),
        };
        [
            at(x0, y0, [0.0, 0.0]),
            at(x0, y1, [0.0, 1.0]),
            at(x1, y1, [1.0, 1.0]),
            at(x1, y0, [1.0, 0.0]),
        ]
    }

    /// Draws `batch`'s stages up to `last` over the cleared frame of `format`, behind a wall
    /// 5 blocks ahead, and reads the frame back as RGBA floats.
    fn render(
        (device, queue): &(wgpu::Device, wgpu::Queue),
        primitives: &mut WorldPrimitives,
        format: wgpu::TextureFormat,
        batch: Batch<'_>,
        last: usize,
    ) -> impl Fn(u32, u32) -> [f32; 4] + use<> {
        let texture = |size: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
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
        let color = texture(
            SIZE,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            SIZE,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let atlas = texture(
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let wall = view_proj().project_point3(Vec3::new(0.0, 0.0, -5.0)).z;
        if let Some(set) = batch.set {
            primitives.pipelines.build(device, set, None).unwrap();
        }
        let drawn = primitives.prepare(
            device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &world_primitives::Batch {
                pipelines: batch,
                ..Default::default()
            },
            view_proj(),
            [SIZE, SIZE],
            &atlas.create_view(&Default::default()),
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: CLEAR,
                            g: CLEAR,
                            b: CLEAR,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wall),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if drawn {
                primitives.draw_through(&mut pass, &mut 0, last);
            }
        }
        let float = format == wgpu::TextureFormat::Rgba16Float;
        let texel = if float { 8 } else { 4 };
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * texel),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * texel),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        move |x: u32, y: u32| {
            let i = ((y * SIZE + x) * texel) as usize;
            std::array::from_fn(|channel| {
                if float {
                    let at = i + channel * 2;
                    half::f16::from_le_bytes([bytes[at], bytes[at + 1]]).to_f32()
                } else {
                    f32::from(bytes[i + channel])
                }
            })
        }
    }

    fn assert_near(got: [f32; 4], expected: [f32; 4], tolerance: f32, what: &str) {
        assert!(
            got.iter()
                .zip(expected)
                .all(|(got, expected)| (got - expected).abs() <= tolerance),
            "{what}: got {got:?}, expected {expected:?}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn gpu_validation_failure_is_contained_and_invalidates_handle() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let renderer = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        let mut broken = set.built()[0].clone().unwrap();
        broken.owner = "owning_mod".into();
        assert_eq!(
            gpu_refusal(&broken, "compiler refusal"),
            "render pipeline test:p0 from mod owning_mod refused by the GPU: compiler refusal"
        );
        // An empty naga module has no `main` entry point, so pipeline creation fails cleanly.
        broken.program = std::sync::Arc::new(crate::core_shader::validated(
            broken.program.vertex.clone(),
            naga::Module::default(),
            Vec::new(),
        ));
        assert!(renderer.pipeline(&device, &broken, None).is_err());
        assert!(set.invalidate(0));
        assert_eq!(set.resolve(&id), Err(mod_api::RenderPipelineError::Invalid));
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
    }

    #[test]
    #[ignore = "daily"]
    fn optional_gpu_refusal_is_published_only_once() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let mut renderer = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        renderer
            .build_with(&set, |_, _| Err("injected refusal".into()))
            .unwrap();
        assert_eq!(renderer.generation, 41);
        assert!(renderer.pipelines[0].is_none());
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
        assert!(!set.invalidate(0));
        renderer
            .build_with(&set, |_, _| panic!("already compiled"))
            .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_later_successful_build_makes_a_refused_slot_usable_again() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let id = Identifier::parse("test:p0").unwrap();
        let handle = set.resolve(&id).unwrap();
        let mut refusing = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        refusing
            .build_with(&set, |_, _| Err("injected refusal".into()))
            .unwrap();
        assert_eq!(set.slot(handle), Err(mod_api::RenderPipelineError::Invalid));
        let mut recovered = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        recovered.build(&device, &set, None).unwrap();
        assert!(recovered.pipelines[0].is_some());
        assert_eq!(set.slot(handle), Ok(0));
        assert!(set.resolve(&id).is_ok());
    }

    #[test]
    #[ignore = "daily"]
    fn required_gpu_failure_does_not_publish_and_mixed_success_does() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let set = load(
            41,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let mut renderer = ModPipelines::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        renderer.build(&device, &set, None).unwrap();
        let shaders = [
            (PASS, PipelineBlend::Translucent, WorldDepth::Tested),
            (PASS, PipelineBlend::Translucent, WorldDepth::Tested),
        ];
        let mut registered: Vec<_> = (0..shaders.len())
            .map(|slot| {
                let id = Identifier::parse(&format!("test:p{slot}")).unwrap();
                let shader = Identifier::parse(&format!("test:core/p{slot}")).unwrap();
                RenderPipeline::new(id, shader, shaders[slot].1, shaders[slot].2).optional()
            })
            .collect();
        registered[0].optional = false;
        let next = Pipelines::build(&registered, 42, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(PASS.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();
        let id = Identifier::parse("test:p0").unwrap();
        let handle = next.resolve(&id).unwrap();
        let error = renderer
            .build_with(&next, |_, _| Err("injected refusal".into()))
            .unwrap_err();
        assert_eq!(
            error,
            "required render pipeline test:p0 from mod  refused by the GPU: injected refusal"
        );
        assert_eq!(renderer.generation, 41);
        assert_eq!(renderer.pipelines.len(), 1);
        assert_eq!(next.slot(handle), Ok(0));
        assert!(next.resolve(&Identifier::parse("test:p1").unwrap()).is_ok());
        renderer.build(&device, &next, None).unwrap();
        assert_eq!(renderer.generation, 42);
        assert_eq!(renderer.pipelines.len(), 2);
        assert!(renderer.pipelines.iter().all(Option::is_some));
    }

    #[test]
    #[ignore = "daily"]
    fn pipelines_convert_colour_both_ways_and_keep_their_blend_depth_and_stage() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        use PipelineBlend::{Additive, Translucent};
        use WorldDepth::{Tested, ThroughWalls};
        let set = load(
            3,
            &[
                (PASS, Translucent, Tested),
                (HALF_GAMMA, Translucent, ThroughWalls),
                // Refused by the load: the pipelines after it still draw.
                ("void nothing() {}", Translucent, Tested),
                (HALF_GAMMA, Additive, Tested),
                (INPUTS, Translucent, Tested),
            ],
        );
        assert!(set.built()[2].is_none());
        let grey = [128, 128, 128, 255];
        let white = [255; 4];
        let vertices = [
            // Top left: a grey quad returned as it came, in front of the wall and behind it.
            quad([0.0, 0.0, 16.0, 32.0], 2.0, grey),
            quad([16.0, 0.0, 32.0, 32.0], 10.0, grey),
            // Top right: 0.72974 (~186) through walls, at a later stage.
            quad([32.0, 0.0, 64.0, 32.0], 10.0, white),
            // Bottom left: 0.72974 (~186) added to the frame.
            quad([0.0, 32.0, 32.0, 64.0], 2.0, white),
            // Bottom right: the texture coordinate and Globals.GameTime.
            quad([32.0, 32.0, 64.0, 64.0], 2.0, white),
        ]
        .concat();
        let run = |slot, stage, quads| PipelineRun {
            slot,
            stage,
            quads,
            modifier_stack: 0,
        };
        let runs = [run(0, 0, 2), run(1, 2, 1), run(3, 0, 1), run(4, 0, 1)];
        let batch = Batch {
            set: Some(&set),
            vertices: &vertices,
            runs: &runs,
            time: 1.0,
            ..Default::default()
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));

        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        let clear = [64.0, 64.0, 64.0, 255.0];
        assert_near(
            at(8, 16),
            [128.0, 128.0, 128.0, 255.0],
            1.0,
            "vanilla gamma colour passes through unchanged",
        );
        assert_near(
            at(24, 16),
            clear,
            0.0,
            "depth tested: hidden behind the wall",
        );
        assert_near(
            at(48, 16),
            [186.0, 186.0, 186.0, 255.0],
            1.0,
            "gamma 0.72974 through walls",
        );
        assert_near(
            at(16, 48),
            [250.0, 250.0, 250.0, 255.0],
            1.0,
            "additive: 186 over the frame's 64",
        );
        assert_near(
            at(40, 40),
            [0.0, 255.0, 0.0, 255.0],
            0.0,
            "the quad's first quarter: uv under 0.5, GameTime 1",
        );
        assert_near(
            at(56, 56),
            [255.0, 255.0, 255.0, 255.0],
            0.0,
            "the quad's last quarter: uv over 0.5",
        );

        // Only the first stage: the later stage's quad is not drawn yet.
        let at = render(&gpu, &mut primitives, format, batch, 0);
        assert_near(
            at(48, 16),
            clear,
            0.0,
            "a later stage's quad waits for its stage",
        );
        assert_near(
            at(8, 16),
            [128.0, 128.0, 128.0, 255.0],
            1.0,
            "the first stage draws",
        );
    }

    /// In a float frame light above white survives as written (gamma encoding), in a linear-light
    /// float frame `fragment_finish` decodes gamma 0.72974 to linear 0.5 (and sanitizes negative
    /// RGB to 0 while preserving alpha), and a new load's shader replaces the old one's pipeline.
    #[test]
    #[ignore = "daily"]
    fn a_float_frame_keeps_light_above_white_and_a_reload_builds_the_new_shader() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let bright = "\
#version 330
out vec4 fragColor;
void main() { fragColor = vec4(1.37035, 1.0, 0.0, 1.0); }
";
        let green = "\
#version 330
out vec4 fragColor;
void main() { fragColor = vec4(0.0, 1.0, 0.0, 1.0); }
";
        let half_and_neg = "\
#version 330
out vec4 fragColor;
void main() { fragColor = vec4(0.72974, -0.5, 1.0, 1.0); }
";
        let state = (PipelineBlend::Translucent, WorldDepth::Tested);
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255; 4]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 4,
            quads: 1,
            modifier_stack: 0,
        }];
        let format = wgpu::TextureFormat::Rgba16Float;

        // Gamma float frame: values above 1.0 survive verbatim.
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let draw = |p: &mut WorldPrimitives, set: &Pipelines| {
            let batch = Batch {
                set: Some(set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            };
            render(&gpu, p, format, batch, STAGES - 1)(32, 32)
        };
        let before = load(1, &[(bright, state.0, state.1)]);
        assert_near(
            draw(&mut primitives, &before),
            [1.37035, 1.0, 0.0, 1.0],
            0.002,
            "gamma value above white in a float frame",
        );
        let after = load(2, &[(green, state.0, state.1)]);
        assert_near(
            draw(&mut primitives, &after),
            [0.0, 1.0, 0.0, 1.0],
            0.002,
            "the reloaded shader",
        );

        // Linear-light float frame: gamma 0.72974 decodes to linear 0.5, negative clamps to 0.
        let mut linear_primitives = WorldPrimitives::new(&gpu.0, (format, true));
        let linear_set = load(3, &[(half_and_neg, state.0, state.1)]);
        assert_near(
            draw(&mut linear_primitives, &linear_set),
            [0.5, 0.0, 1.0, 1.0],
            0.005,
            "linear-light frame decodes gamma 0.72974 to 0.5 and sanitizes negatives",
        );
    }

    /// Two pipelines sharing one GLSL program but differing in `.define(...)` draw different
    /// pixels on the GPU (#5177).
    #[test]
    #[ignore = "daily"]
    fn defines_specialize_one_glsl_program_into_distinct_gpu_pipelines() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let fsh = "\
#version 330
out vec4 fragColor;
void main() {
#ifdef RED_MODE
    fragColor = vec4(1.0, 0.0, 0.0, 1.0);
#else
    fragColor = vec4(0.0, float(GREEN_LEVEL) / 2.0, 1.0, 1.0);
#endif
}
";
        let shared = Identifier::parse("test:core/shared").unwrap();
        let registered = [
            RenderPipeline::new(
                Identifier::parse("test:red").unwrap(),
                shared.clone(),
                PipelineBlend::Translucent,
                WorldDepth::Tested,
            )
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .define("RED_MODE"),
            RenderPipeline::new(
                Identifier::parse("test:cyan").unwrap(),
                shared,
                PipelineBlend::Translucent,
                WorldDepth::Tested,
            )
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .define_value("GREEN_LEVEL", "2"),
        ];
        let set = Pipelines::build(&registered, 11, |file| match file {
            "assets/test/shaders/core/shared.fsh" => Ok(Some(fsh.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        let vertices = [
            quad([0.0, 0.0, 32.0, 64.0], 2.0, [255; 4]),
            quad([32.0, 0.0, 64.0, 64.0], 2.0, [255; 4]),
        ]
        .concat();
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
            PipelineRun {
                slot: 1,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(at(16, 32), [255.0, 0.0, 0.0, 255.0], 1.0, "RED_MODE define");
        assert_near(
            at(48, 32),
            [0.0, 255.0, 255.0, 255.0],
            1.0,
            "GREEN_LEVEL 2 define",
        );
    }

    /// A textured pipeline samples a 2x1 sprite's two texels across its quad via
    /// `<sunrise:pipeline_sprite.glsl>`'s `sampleSprite(texCoord0)`, and one naming half the
    /// atlas samples only that sprite's part, repeating it past 1.
    #[test]
    #[ignore = "daily"]
    fn textured_pipelines_sample_their_file_or_their_sprite() {
        use mod_api::PipelineTexture;
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = (&gpu.0, &gpu.1);
        let sample = "\
#version 330
#include <sunrise:pipeline_sprite.glsl>
in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = sampleSprite(texCoord0);
}
";
        let id = |slot: usize| Identifier::parse(&format!("test:p{slot}")).unwrap();
        let shader_id = |slot: usize| Identifier::parse(&format!("test:core/p{slot}")).unwrap();
        let state = (PipelineBlend::Translucent, WorldDepth::Tested);
        let pipeline =
            |slot| RenderPipeline::new(id(slot), shader_id(slot), state.0, state.1).optional();
        let registered = [
            pipeline(0).textured(PipelineTexture::Sprite(id(0))),
            pipeline(1).textured(PipelineTexture::Sprite(id(1))),
            pipeline(2).textured(PipelineTexture::Sprite(
                Identifier::parse("test:none").unwrap(),
            )),
            pipeline(3),
        ];
        let red_green = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let set = Pipelines::build(&registered, 7, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(VSH.to_owned()))
            } else if file.ends_with("p3.fsh") {
                Ok(Some(PASS.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(sample.to_owned()))
            } else {
                Ok(stub_asset(file))
            }
        })
        .unwrap();
        let atlas = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &red_green,
        );
        let atlas = atlas.create_view(&Default::default());
        let sprite = |name: &str| match name {
            "test:p0" => Some([0.0, 0.0, 1.0, 1.0]),
            "test:p1" => Some([0.5, 0.0, 1.0, 1.0]),
            _ => None,
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(device, (format, false));
        let sources = TextureSources {
            atlas: &atlas,
            sprite: &sprite,
        };
        primitives
            .pipelines
            .build(device, &set, Some(sources))
            .unwrap();
        assert_eq!(
            set.resolve(&id(2)),
            Err(mod_api::RenderPipelineError::Invalid),
            "a sprite the atlas lacks"
        );
        assert!(primitives.pipelines.pipelines[0].as_ref().unwrap().textured);
        assert!(!primitives.pipelines.pipelines[3].as_ref().unwrap().textured);
        let mut vertices = Vec::new();
        for (row, slot) in [(0.0, 0), (16.0, 1), (32.0, 3)] {
            let mut quad = quad([0.0, row, 64.0, row + 16.0], 2.0, [255, 255, 255, 255]);
            if slot == 1 {
                quad.iter_mut().for_each(|v| v.uv[0] *= 2.0);
            }
            vertices.extend(quad);
        }
        let runs = [0, 1, 3].map(|slot| PipelineRun {
            slot,
            stage: 4,
            quads: 1,
            modifier_stack: 0,
        });
        let pixel = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        let (red, green) = ([255.0, 0.0, 0.0, 255.0], [0.0, 255.0, 0.0, 255.0]);
        assert_near(pixel(8, 8), red, 1.0, "the 2x1 sprite's left texel");
        assert_near(pixel(56, 8), green, 1.0, "the 2x1 sprite's right texel");
        assert_near(pixel(8, 24), green, 1.0, "the sprite, not the atlas's red");
        assert_near(pixel(40, 24), green, 1.0, "the sprite repeated");
        assert_near(pixel(8, 40), [255.0; 4], 1.0, "the untextured pipeline");
    }

    #[test]
    #[ignore = "daily"]
    fn pipeline_quads_chunk_across_small_max_buffer_size_and_preserve_order() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        let limit = 20 * 1024;
        let gpu = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_buffer_size: limit,
                max_uniform_buffer_binding_size: limit,
                max_storage_buffer_binding_size: limit,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .expect("GPU device with small max_buffer_size");
        let set = load(
            77,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let quad_bytes = 4 * std::mem::size_of::<PipelineVertex>() as u64;
        let max_quads = world_primitives::VertexPages::limit(limit, quad_bytes);
        assert!(max_quads > 0);
        let first_count = max_quads + 20;
        let mut vertices = Vec::with_capacity((first_count as usize + 1) * 4);
        for _ in 0..first_count {
            vertices.extend_from_slice(&quad([0.0, 0.0, 64.0, 64.0], 2.0, [255, 0, 0, 255]));
        }
        vertices.extend_from_slice(&quad([32.0, 0.0, 64.0, 64.0], 2.0, [0, 255, 0, 255]));
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: first_count,
                modifier_stack: 0,
            },
            PipelineRun {
                slot: 0,
                stage: (STAGES - 1) as u32,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert!(primitives.pipelines.vertices.buffers.len() >= 2);
        assert!(
            primitives
                .pipelines
                .vertices
                .buffers
                .iter()
                .all(|buffer| buffer.size() <= limit)
        );
        assert_eq!(at(16, 32), [255.0, 0.0, 0.0, 255.0], "earlier stage");
        assert_eq!(
            at(48, 32),
            [0.0, 255.0, 0.0, 255.0],
            "later stage over earlier stage"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn modifier_through_walls_nested_outermost_wins_and_refused_line_skips_only_its_runs() {
        use mod_api::PipelineModifier;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        // Request a device WITHOUT POLYGON_MODE_LINE so a Line modifier is guaranteed refused.
        let gpu = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::empty(),
            ..Default::default()
        }))
        .expect("GPU device without POLYGON_MODE_LINE");

        let set = load(
            91,
            &[(PASS, PipelineBlend::Translucent, WorldDepth::Tested)],
        );
        let modifiers = [
            PipelineModifier::new(Identifier::parse("test:xray").unwrap())
                .depth(WorldDepth::ThroughWalls),
            PipelineModifier::new(Identifier::parse("test:tested").unwrap())
                .depth(WorldDepth::Tested),
            PipelineModifier::new(Identifier::parse("test:wireframe").unwrap())
                .polygon(PolygonMode::Line),
        ];
        let mut stacks = ModifierStacks::new();
        let xray = stacks.intern(&[0]);
        // Nested: outer = xray (0), inner = tested (1) -> outermost (xray) wins!
        let nested_xray = stacks.intern(&[0, 1]);
        // Nested: outer = tested (1), inner = xray (0) -> outermost (tested) wins!
        let nested_tested = stacks.intern(&[1, 0]);
        let wireframe = stacks.intern(&[2]);

        // render() puts a nearer depth wall on x=16..32 at z=4.0 (quads at z=8.0 are behind it).
        let vertices = [
            // Behind wall, with xray modifier -> visible red.
            quad([16.0, 0.0, 24.0, 32.0], 8.0, [255, 0, 0, 255]),
            // Behind wall, nested [xray, tested] -> outermost xray wins -> visible green.
            quad([24.0, 0.0, 32.0, 32.0], 8.0, [0, 255, 0, 255]),
            // Behind wall, nested [tested, xray] -> outermost tested wins -> hidden by wall.
            quad([16.0, 32.0, 32.0, 64.0], 8.0, [0, 0, 255, 255]),
            // Refused Line modifier on right half -> skipped, leaving clear colour.
            quad([32.0, 0.0, 48.0, 64.0], 2.0, [255, 255, 0, 255]),
            // Unmodified quad beside refused run -> still draws normally!
            quad([48.0, 0.0, 64.0, 64.0], 2.0, [0, 255, 255, 255]),
        ]
        .concat();
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: xray,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: nested_xray,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: nested_tested,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: wireframe,
            },
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                modifiers: Some(&stacks),
                modifier_defs: Some(&modifiers),
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        let clear = [64.0, 64.0, 64.0, 255.0];
        assert_near(
            at(20, 16),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "ThroughWalls modifier draws behind nearer depth",
        );
        assert_near(
            at(28, 16),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "nested [xray, tested]: outermost ThroughWalls wins",
        );
        assert_near(
            at(24, 48),
            clear,
            0.0,
            "nested [tested, xray]: outermost Tested wins and hides behind wall",
        );
        assert_near(
            at(40, 32),
            clear,
            0.0,
            "refused Line modifier variant skips its run",
        );
        assert_near(
            at(56, 32),
            [0.0, 255.0, 255.0, 255.0],
            1.0,
            "other runs still draw beside a refused modifier variant",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn opaque_pipeline_writes_solid_color_without_blending() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [RenderPipeline::new(
            Identifier::parse("test:opaque").unwrap(),
            Identifier::parse("test:core/opaque").unwrap(),
            PipelineBlend::Opaque,
            WorldDepth::Tested,
        )
        .depth_write(true)
        .optional()];
        let set = Pipelines::build(&registered, 92, |file| match file {
            "assets/test/shaders/core/opaque.vsh" => Ok(Some(VSH.to_owned())),
            "assets/test/shaders/core/opaque.fsh" => Ok(Some(PASS.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        // Half-alpha red quad: Opaque writes raw RGB (255, 0, 0) instead of alpha-blending with clear 64.
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255, 0, 0, 128]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 0,
            quads: 1,
            modifier_stack: 0,
        }];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(32, 32),
            [255.0, 0.0, 0.0, 128.0],
            1.0,
            "Opaque writes raw colour without alpha blending",
        );
    }

    #[test]
    fn fan_indices_emulate_triangle_fan_from_vertex_zero() {
        assert_eq!(fan_indices_for(0), Vec::<u32>::new());
        assert_eq!(fan_indices_for(2), Vec::<u32>::new());
        assert_eq!(fan_indices_for(3), vec![0, 1, 2]);
        assert_eq!(fan_indices_for(5), vec![0, 1, 2, 0, 2, 3, 0, 3, 4]);
    }

    #[test]
    #[ignore = "daily"]
    fn depth_bias_lifts_coplanar_quad_over_wall_and_strict_greater_rejects_without_bias() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [
            RenderPipeline::new(
                Identifier::parse("test:unbiased").unwrap(),
                Identifier::parse("test:core/unbiased").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::Tested,
            )
            .depth_compare(DepthCompare::Greater)
            .optional(),
            RenderPipeline::new(
                Identifier::parse("test:biased").unwrap(),
                Identifier::parse("test:core/biased").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::Tested,
            )
            .depth_compare(DepthCompare::Greater)
            .depth_bias(100, 1.0)
            .optional(),
        ];
        let set = Pipelines::build(&registered, 93, |file| match file {
            "assets/test/shaders/core/unbiased.vsh" | "assets/test/shaders/core/biased.vsh" => {
                Ok(Some(VSH.to_owned()))
            }
            "assets/test/shaders/core/unbiased.fsh" | "assets/test/shaders/core/biased.fsh" => {
                Ok(Some(PASS.to_owned()))
            }
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        // render() clears depth to the wall at z = 5.0.
        // Left half: coplanar at z = 5.0 with DepthCompare::Greater and no bias -> rejected.
        // Right half: coplanar at z = 5.0 with DepthCompare::Greater and positive bias -> lifted.
        let vertices = [
            quad([0.0, 0.0, 32.0, 64.0], 5.0, [255, 0, 0, 255]),
            quad([32.0, 0.0, 64.0, 64.0], 5.0, [0, 255, 0, 255]),
        ]
        .concat();
        let runs = [
            PipelineRun {
                slot: 0,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
            PipelineRun {
                slot: 1,
                stage: 0,
                quads: 1,
                modifier_stack: 0,
            },
        ];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(16, 32),
            [64.0, 64.0, 64.0, 255.0],
            1.0,
            "coplanar quad without bias fails strict Greater against wall",
        );
        assert_near(
            at(48, 32),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "positive depth bias lifts coplanar quad over wall",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn custom_blend_invert_produces_exact_inverted_pixels() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [RenderPipeline::new(
            Identifier::parse("test:invert").unwrap(),
            Identifier::parse("test:core/invert").unwrap(),
            PipelineBlend::Custom(BlendFunction::INVERT),
            WorldDepth::ThroughWalls,
        )
        .optional()];
        let set = Pipelines::build(&registered, 94, |file| match file {
            "assets/test/shaders/core/invert.vsh" => Ok(Some(VSH.to_owned())),
            "assets/test/shaders/core/invert.fsh" => Ok(Some(PASS.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();
        // Clear is 64/255; BlendFunction::INVERT with red (255, 0, 0, 255):
        // R = 1*(1 - 64/255) + (64/255)*0 = 191/255, G = B = 64/255, A = 255.
        let vertices = quad([0.0, 0.0, 64.0, 64.0], 2.0, [255, 0, 0, 255]);
        let runs = [PipelineRun {
            slot: 0,
            stage: 0,
            quads: 1,
            modifier_stack: 0,
        }];
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&gpu.0, (format, false));
        let at = render(
            &gpu,
            &mut primitives,
            format,
            Batch {
                set: Some(&set),
                vertices: &vertices,
                runs: &runs,
                time: 0.0,
                ..Default::default()
            },
            STAGES - 1,
        );
        assert_near(
            at(32, 32),
            [191.0, 64.0, 64.0, 255.0],
            1.5,
            "BlendFunction::INVERT inverts cleared destination with red source",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn depthless_triangles_debug_lines_and_triangle_fan_draw_expected_pixels() {
        use wgpu::util::DeviceExt;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let registered = [
            RenderPipeline::new(
                Identifier::parse("test:tri").unwrap(),
                Identifier::parse("test:core/tri").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::ThroughWalls,
            )
            .depth_stencil(None)
            .topology(PrimitiveTopology::Triangles)
            .optional(),
            RenderPipeline::new(
                Identifier::parse("test:fan").unwrap(),
                Identifier::parse("test:core/fan").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::ThroughWalls,
            )
            .depth_stencil(None)
            .topology(PrimitiveTopology::TriangleFan)
            .optional(),
            RenderPipeline::new(
                Identifier::parse("test:line").unwrap(),
                Identifier::parse("test:core/line").unwrap(),
                PipelineBlend::Opaque,
                WorldDepth::ThroughWalls,
            )
            .depth_stencil(None)
            .topology(PrimitiveTopology::DebugLines)
            .optional(),
        ];
        let set = Pipelines::build(&registered, 95, |file| match file {
            "assets/test/shaders/core/tri.vsh"
            | "assets/test/shaders/core/fan.vsh"
            | "assets/test/shaders/core/line.vsh" => Ok(Some(VSH.to_owned())),
            "assets/test/shaders/core/tri.fsh"
            | "assets/test/shaders/core/fan.fsh"
            | "assets/test/shaders/core/line.fsh" => Ok(Some(PASS.to_owned())),
            other => Ok(stub_asset(other)),
        })
        .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut mod_pipelines = ModPipelines::new(&device, (format, false));
        mod_pipelines.build(&device, &set, None).unwrap();
        let blocks = fallback_vanilla_blocks(view_proj(), [SIZE, SIZE], 0.0);
        queue.write_buffer(&mod_pipelines.vanilla, 0, &blocks);

        // Build test vertices:
        // - Triangle on top-left (x=0..32, y=0..32) in red.
        // - 4-vertex TriangleFan covering top-right (x=32..64, y=0..32) in green.
        // - Horizontal DebugLines across y=48 (x=4..60) in blue.
        let tri_quad = quad([0.0, 0.0, 32.0, 32.0], 2.0, [255, 0, 0, 255]);
        let tri_verts = [tri_quad[0], tri_quad[1], tri_quad[2]];
        let fan_verts = quad([32.0, 0.0, 64.0, 32.0], 2.0, [0, 255, 0, 255]);
        let line_quad = quad([4.0, 48.5, 60.0, 49.5], 2.0, [0, 0, 255, 255]);
        let line_verts = [line_quad[0], line_quad[3]];

        let tri_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&tri_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let fan_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&fan_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let line_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&line_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let fan_idx = mod_pipelines.fan_indices.ensure(&device, &queue, 4).clone();

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: CLEAR,
                            g: CLEAR,
                            b: CLEAR,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            // Draw Triangles
            let tri_c = mod_pipelines.pipelines[0].as_ref().unwrap();
            pass.set_pipeline(&tri_c.pipeline);
            pass.set_bind_group(0, &tri_c.group, &[]);
            pass.set_vertex_buffer(0, tri_buf.slice(..));
            pass.draw(0..3, 0..1);

            // Draw TriangleFan via engine-generated fan index buffer
            let fan_c = mod_pipelines.pipelines[1].as_ref().unwrap();
            pass.set_pipeline(&fan_c.pipeline);
            pass.set_bind_group(0, &fan_c.group, &[]);
            pass.set_vertex_buffer(0, fan_buf.slice(..));
            pass.set_index_buffer(fan_idx.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..PrimitiveTopology::TriangleFan.index_count(4), 0, 0..1);

            // Draw DebugLines
            let line_c = mod_pipelines.pipelines[2].as_ref().unwrap();
            pass.set_pipeline(&line_c.pipeline);
            pass.set_bind_group(0, &line_c.group, &[]);
            pass.set_vertex_buffer(0, line_buf.slice(..));
            pass.draw(0..2, 0..1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, y: u32| -> [f32; 4] {
            let i = ((y * SIZE + x) * 4) as usize;
            std::array::from_fn(|c| f32::from(bytes[i + c]))
        };
        assert_near(
            at(8, 24),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "depthless Triangles draws red triangle",
        );
        assert_near(
            at(56, 8),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "depthless TriangleFan draws second fan triangle via engine fan index buffer",
        );
        assert_near(
            at(32, 48),
            [0.0, 0.0, 255.0, 255.0],
            1.0,
            "depthless DebugLines draws horizontal blue line",
        );
    }

    /// Retained GPU `MeshRef` with a compact 8-byte vertex format (`Uint8x4` + `Unorm8x4`) and
    /// `Uint16` index buffer draws identically across two frames without re-uploading, and
    /// returns `MeshError::Stale` after `destroy` (#1159).
    #[test]
    #[ignore = "daily"]
    fn retained_compact_8byte_mesh_draws_two_frames_and_destroy_returns_stale() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let vsh = "\
#version 330
in uvec4 PackedPos;
in vec4 PackedColor;
out vec4 vertexColor;
void main() {
    vec2 ndc = vec2(float(PackedPos.x) / 32.0 - 1.0, 1.0 - float(PackedPos.y) / 32.0);
    gl_Position = vec4(ndc, 0.5, 1.0);
    vertexColor = PackedColor;
}
";
        let fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let compact_fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("PackedPos", VertexAttributeFormat::Uint8x4)
            .attribute("PackedColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        assert_eq!(compact_fmt.stride, 8);
        let id = Identifier::parse("example:compact_mesh").unwrap();
        let reg = [RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .topology(PrimitiveTopology::Triangles)
        .vertex_binding(0, compact_fmt)];
        let set = Pipelines::build(&reg, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .unwrap();

        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct CompactVertex {
            pos: [u8; 4],
            color: [u8; 4],
        }
        let verts = [
            CompactVertex {
                pos: [0, 0, 0, 0],
                color: [20, 220, 40, 255],
            },
            CompactVertex {
                pos: [0, 64, 0, 0],
                color: [20, 220, 40, 255],
            },
            CompactVertex {
                pos: [64, 64, 0, 0],
                color: [20, 220, 40, 255],
            },
            CompactVertex {
                pos: [64, 0, 0, 0],
                color: [20, 220, 40, 255],
            },
        ];
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let vb_inits = [mod_api::MeshBufferInit::from_bytes(
            8,
            bytemuck::cast_slice(&verts),
        )];
        let desc = mod_api::MeshDescriptor::new(&vb_inits).with_indices(
            mod_api::MeshIndexInit::from_bytes(IndexFormat::Uint16, bytemuck::cast_slice(&indices)),
        );
        let mut meshes = crate::gfx::mod_meshes::ModMeshes::default();
        let mesh = meshes.create_mesh(&device, &queue, None, &desc).unwrap();
        let mesh_draw = MeshDraw::indexed(0..6, 0);
        let resolved = meshes
            .resolve_draw(
                None,
                mesh,
                set.vertex_bindings(0).unwrap(),
                PrimitiveTopology::Triangles,
                &mesh_draw,
            )
            .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: mesh_draw.clone(),
            source: PipelineDrawSource::Retained(resolved),
            bindings: None,
        };
        let gpu = (device, queue);
        for frame in 0..2 {
            let batch = Batch {
                set: Some(&set),
                draws: std::slice::from_ref(&draw_cmd),
                time: 0.0,
                ..Default::default()
            };
            let px = render(&gpu, &mut primitives, format, batch, STAGES - 1)(32, 32);
            assert_near(
                px,
                [20.0, 220.0, 40.0, 255.0],
                1.0,
                &format!("retained compact 8-byte mesh frame {frame}"),
            );
        }

        meshes.destroy(None, mesh).unwrap();
        assert_eq!(
            meshes
                .resolve_draw(
                    None,
                    mesh,
                    set.vertex_bindings(0).unwrap(),
                    PrimitiveTopology::Triangles,
                    &mesh_draw,
                )
                .unwrap_err(),
            mod_api::PipelineDrawError::Mesh(mod_api::MeshError::Stale)
        );
    }

    /// Multi-binding vertex format with `VertexStepMode::Instance` draws 4 offset coloured
    /// quads in a single instanced draw call (#1159).
    #[test]
    #[ignore = "daily"]
    fn instanced_vertex_binding_draws_four_offset_quads_in_one_draw_call() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let vsh = "\
#version 330
in vec2 LocalPos;
in vec2 InstOffset;
in vec4 InstColor;
out vec4 vertexColor;
void main() {
    vec2 p = LocalPos + InstOffset;
    vec2 ndc = vec2(p.x / 32.0 - 1.0, 1.0 - p.y / 32.0);
    gl_Position = vec4(ndc, 0.5, 1.0);
    vertexColor = InstColor;
}
";
        let fsh = "\
#version 330
in vec4 vertexColor;
out vec4 fragColor;
void main() {
    fragColor = vertexColor;
}
";
        let vert_fmt = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("LocalPos", VertexAttributeFormat::Float32x2)
            .build()
            .unwrap();
        let inst_fmt = VertexFormat::builder(VertexStepMode::Instance)
            .attribute("InstOffset", VertexAttributeFormat::Float32x2)
            .attribute("InstColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        let id = Identifier::parse("example:instanced").unwrap();
        let reg = [RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            WorldDepth::ThroughWalls,
        )
        .topology(PrimitiveTopology::Quads)
        .vertex_binding(0, vert_fmt)
        .vertex_binding(1, inst_fmt)];
        let set = Pipelines::build(&reg, 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(sunrise_mod_pipeline::stubs::read(
                    file.strip_prefix("assets/")
                        .and_then(|r| r.split_once('/'))
                        .map(|(ns, p)| format!("{ns}:{p}"))
                        .as_deref()
                        .unwrap_or(""),
                )
                .map(str::to_owned))
            }
        })
        .unwrap();

        let unit_quad: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 32.0], [32.0, 32.0], [32.0, 0.0]];
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct InstanceRec {
            offset: [f32; 2],
            color: [u8; 4],
        }
        let instances = [
            InstanceRec {
                offset: [0.0, 0.0],
                color: [255, 0, 0, 255],
            },
            InstanceRec {
                offset: [32.0, 0.0],
                color: [0, 255, 0, 255],
            },
            InstanceRec {
                offset: [0.0, 32.0],
                color: [0, 0, 255, 255],
            },
            InstanceRec {
                offset: [32.0, 32.0],
                color: [255, 255, 0, 255],
            },
        ];
        let vb_inits = [
            mod_api::MeshBufferInit::from_bytes(8, bytemuck::cast_slice(&unit_quad)),
            mod_api::MeshBufferInit::from_bytes(12, bytemuck::cast_slice(&instances)),
        ];
        let desc = mod_api::MeshDescriptor::new(&vb_inits);
        let mut meshes = crate::gfx::mod_meshes::ModMeshes::default();
        let mesh = meshes.create_mesh(&device, &queue, None, &desc).unwrap();
        let mesh_draw = MeshDraw::vertices(0..4).with_instances(0..4);
        let resolved = meshes
            .resolve_draw(
                None,
                mesh,
                set.vertex_bindings(0).unwrap(),
                PrimitiveTopology::Quads,
                &mesh_draw,
            )
            .unwrap();

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: mesh_draw,
            source: PipelineDrawSource::Retained(resolved),
            bindings: None,
        };
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw_cmd),
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(16, 16), [255.0, 0.0, 0.0, 255.0], 1.0, "instance 0 red");
        assert_near(
            at(48, 16),
            [0.0, 255.0, 0.0, 255.0],
            1.0,
            "instance 1 green",
        );
        assert_near(at(16, 48), [0.0, 0.0, 255.0, 255.0], 1.0, "instance 2 blue");
        assert_near(
            at(48, 48),
            [255.0, 255.0, 0.0, 255.0],
            1.0,
            "instance 3 yellow",
        );
    }

    #[test]
    #[ignore = "daily"]
    fn set1_uniform_buffer_inline_uniform_push_constant_and_texel_buffer_draw_expected_pixels() {
        let _guard = crate::gfx::test_gpu::lock();
        let (device, queue) = gpu();
        let id = Identifier::parse("mod_c3:set1_combo").unwrap();
        let vsh = "#version 330\n\
            #include <minecraft:projection.glsl>\n\
            #include <minecraft:dynamictransforms.glsl>\n\
            layout(push_constant) uniform Push { vec4 Offset; };\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec4 vColor;\n\
            void main() {\n\
                gl_Position = ProjMat * ModelViewMat * vec4(Position + Offset.xyz, 1.0);\n\
                vColor = Color;\n\
            }\n";
        let fsh = "#version 330\n\
            layout(std140) uniform BaseTint { vec4 Base; };\n\
            layout(std140) uniform DrawTint { vec4 Scale; };\n\
            uniform isamplerBuffer Lut;\n\
            in vec4 vColor;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                float lutScale = float(texelFetch(Lut, 1).r) / 255.0;\n\
                fragColor = vec4(Base.rgb * Scale.rgb * lutScale, 1.0) * vColor;\n\
            }\n";
        let p = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .uniform_buffer("BaseTint", 16)
        .uniform_buffer("DrawTint", 16)
        .texel_buffer("Lut", mod_api::TexelBufferFormat::R32Sint)
        .push_constant_size(16);
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(None)
            }
        })
        .expect("set-1 pipeline must translate");

        let mut uniforms = crate::gfx::mod_uniforms::ModUniformBuffers::default();
        let base_bytes: [f32; 4] = [1.0, 0.5, 1.0, 1.0];
        let ubo = uniforms
            .create_uniform_buffer(&device, &queue, None, bytemuck::cast_slice(&base_bytes))
            .unwrap();
        let (ubo_id, ubo_buf, ubo_len) = uniforms.resolve(None, ubo, 16).unwrap();

        let quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 32.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [32.0, 32.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [32.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let vbytes = bytemuck::cast_slice(&quad);
        let inline1: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
        let pc1: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
        let inline2: [f32; 4] = [0.0, 1.0, 1.0, 1.0];
        let pc2: [f32; 4] = [32.0, 0.0, 0.0, 0.0];
        let mut inline_pool = Vec::new();
        inline_pool.extend_from_slice(bytemuck::cast_slice(&inline1));
        inline_pool.extend_from_slice(bytemuck::cast_slice(&pc1));
        inline_pool.extend_from_slice(bytemuck::cast_slice(&inline2));
        inline_pool.extend_from_slice(bytemuck::cast_slice(&pc2));
        let lut_values: [i32; 2] = [0, 255];
        let texel_pool: &[u8] = bytemuck::cast_slice(&lut_values);

        let draws = [
            PipelineDraw {
                slot: 0,
                stage: 4,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    entries: vec![
                        ResolvedSet1Entry::UniformBuffer {
                            id: ubo_id,
                            buffer: ubo_buf.clone(),
                            byte_len: ubo_len,
                        },
                        ResolvedSet1Entry::InlineUniform { byte_range: 0..16 },
                        ResolvedSet1Entry::TexelBuffer {
                            byte_range: 0..texel_pool.len(),
                        },
                    ],
                    push_constants: Some(16..32),
                }),
            },
            PipelineDraw {
                slot: 0,
                stage: 4,
                modifier_stack: 0,
                draw: MeshDraw::vertices(0..4),
                source: PipelineDrawSource::Transient {
                    vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                    index_range: None,
                },
                bindings: Some(ResolvedDrawBindings {
                    entries: vec![
                        ResolvedSet1Entry::UniformBuffer {
                            id: ubo_id,
                            buffer: ubo_buf,
                            byte_len: ubo_len,
                        },
                        ResolvedSet1Entry::InlineUniform { byte_range: 32..48 },
                        ResolvedSet1Entry::TexelBuffer {
                            byte_range: 0..texel_pool.len(),
                        },
                    ],
                    push_constants: Some(48..64),
                }),
            },
        ];

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: &draws,
            transient_vertices: vbytes,
            inline_uniforms: &inline_pool,
            texel_data: texel_pool,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(16, 16), [255.0, 128.0, 0.0, 255.0], 2.0, "draw 1");
        assert_near(at(48, 16), [0.0, 128.0, 255.0, 255.0], 2.0, "draw 2");
        uniforms.destroy(None, ubo).unwrap();
        assert_eq!(
            uniforms.resolve(None, ubo, 16).unwrap_err(),
            mod_api::PipelineDrawError::Buffer(mod_api::BufferError::Stale)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn set1_dynamic_and_standalone_textures_sample_with_repeat_addressing() {
        let _guard = crate::gfx::test_gpu::lock();
        let (device, queue) = gpu();
        let id = Identifier::parse("mod_c3:repeat_tex").unwrap();
        let vsh = "#version 330\n\
            #include <minecraft:projection.glsl>\n\
            #include <minecraft:dynamictransforms.glsl>\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec2 vUv;\n\
            void main() {\n\
                gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);\n\
                vUv = UV0;\n\
            }\n";
        let fsh = "#version 330\n\
            uniform sampler2D CustomTex;\n\
            in vec2 vUv;\n\
            out vec4 fragColor;\n\
            void main() {\n\
                fragColor = texture(CustomTex, vUv);\n\
            }\n";
        let p = RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .sampler("CustomTex", SamplerFilter::Nearest, SamplerAddress::Repeat);
        let set = Pipelines::build(&[p], 1, |file| {
            if file.ends_with(".vsh") {
                Ok(Some(vsh.to_owned()))
            } else if file.ends_with(".fsh") {
                Ok(Some(fsh.to_owned()))
            } else {
                Ok(None)
            }
        })
        .expect("textured set-1 pipeline must translate");

        let dyn_id = Identifier::parse("mod_c3:dyn_stripe").unwrap();
        let rgba: [u8; 8] = [255, 0, 0, 255, 0, 0, 255, 255];
        let mut uniforms = crate::gfx::mod_uniforms::ModUniformBuffers::default();
        let (view_id, view) = uniforms
            .resolve_dynamic_texture(&device, &queue, &dyn_id, [2, 1], &rgba, 1)
            .unwrap();

        let quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 64.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 64.0, 0.0],
                uv: [2.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 0.0, 0.0],
                uv: [2.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let vbytes = bytemuck::cast_slice(&quad);
        let draw_cmd = PipelineDraw {
            slot: 0,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                entries: vec![ResolvedSet1Entry::Texture { view_id, view }],
                push_constants: None,
            }),
        };

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&draw_cmd),
            transient_vertices: vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(at(8, 32), [255.0, 0.0, 0.0, 255.0], 1.0, "repeat 0 red");
        assert_near(at(24, 32), [0.0, 0.0, 255.0, 255.0], 1.0, "repeat 0 blue");
        assert_near(at(40, 32), [255.0, 0.0, 0.0, 255.0], 1.0, "repeat 1 red");
        assert_near(at(56, 32), [0.0, 0.0, 255.0, 255.0], 1.0, "repeat 1 blue");
    }

    #[test]
    #[ignore = "daily"]
    fn offscreen_render_target_draw_sample_persist_resize_and_hazard_check() {
        let _gpu_lock = crate::gfx::test_gpu::lock();
        let (device, queue) = gpu_device();

        // Pipeline 0: solid red quad into offscreen target (no set-1 bindings).
        let write_id = Identifier::parse("test:offscreen_write").unwrap();
        let write_vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec4 vColor;\n\
            layout(std140) uniform Projection { mat4 ProjMat; };\n\
            layout(std140) uniform DynamicTransforms {\n\
                mat4 ModelViewMat; mat4 TextureMat; vec4 ColorModulator; vec3 ModelOffset;\n\
            };\n\
            void main() {\n\
                gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);\n\
                vColor = Color;\n\
            }\n";
        let write_fsh = "#version 330\n\
            in vec4 vColor;\n\
            out vec4 fragColor;\n\
            void main() { fragColor = vColor; }\n";

        // Pipeline 1: samples the offscreen target's colour attachment via set-1 sampler.
        let read_id = Identifier::parse("test:offscreen_read").unwrap();
        let read_vsh = "#version 330\n\
            in vec3 Position;\n\
            in vec2 UV0;\n\
            in vec4 Color;\n\
            out vec2 vUv;\n\
            layout(std140) uniform Projection { mat4 ProjMat; };\n\
            layout(std140) uniform DynamicTransforms {\n\
                mat4 ModelViewMat; mat4 TextureMat; vec4 ColorModulator; vec3 ModelOffset;\n\
            };\n\
            void main() {\n\
                gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);\n\
                vUv = UV0;\n\
            }\n";
        let read_fsh = "#version 330\n\
            in vec2 vUv;\n\
            uniform sampler2D OffscreenTex;\n\
            out vec4 fragColor;\n\
            void main() { fragColor = texture(OffscreenTex, vUv); }\n";

        let p_write = RenderPipeline::new(
            write_id.clone(),
            write_id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        );
        let p_read = RenderPipeline::new(
            read_id.clone(),
            read_id,
            PipelineBlend::Opaque,
            mod_api::WorldDepth::ThroughWalls,
        )
        .sampler(
            "OffscreenTex",
            mod_api::SamplerFilter::Nearest,
            mod_api::SamplerAddress::Clamp,
        );

        let set = Pipelines::build(&[p_write, p_read], 1, |file| {
            if file.contains("offscreen_write") && file.ends_with(".vsh") {
                Ok(Some(write_vsh.to_owned()))
            } else if file.contains("offscreen_write") && file.ends_with(".fsh") {
                Ok(Some(write_fsh.to_owned()))
            } else if file.contains("offscreen_read") && file.ends_with(".vsh") {
                Ok(Some(read_vsh.to_owned()))
            } else if file.contains("offscreen_read") && file.ends_with(".fsh") {
                Ok(Some(read_fsh.to_owned()))
            } else {
                Ok(None)
            }
        })
        .expect("offscreen write/read pipelines must translate");

        let mut targets = crate::gfx::mod_render_targets::ModRenderTargets::default();
        let rt = targets
            .create(
                &device,
                Some("test_mod"),
                &mod_api::RenderTargetDescriptor::rgba8(
                    [32, 32],
                    mod_api::RenderTargetDepth::Depth32Float,
                ),
            )
            .expect("create 32x32 RGBA8+D32F target");
        assert_eq!(targets.size(Some("test_mod"), rt), Ok([32, 32]));
        assert_eq!(
            targets.size(Some("other_mod"), rt),
            Err(mod_api::RenderTargetError::NotOwned)
        );

        // Self-sampling hazard check: binding `rt` while `active_target == Some(rt)` is refused.
        assert_eq!(
            targets
                .resolve_color(Some("test_mod"), Some(rt), rt, 0, false, false)
                .map(|(id, _)| id),
            Err(mod_api::PipelineDrawError::TargetReadWriteHazard)
        );
        assert_eq!(
            targets
                .resolve_depth(Some("test_mod"), Some(rt), rt, false)
                .map(|(id, _)| id),
            Err(mod_api::PipelineDrawError::TargetReadWriteHazard)
        );
        // Depth aspect refuses linear (filterable) sampling, allows nearest.
        assert_eq!(
            targets
                .resolve_depth(Some("test_mod"), None, rt, true)
                .map(|(id, _)| id),
            Err(mod_api::PipelineDrawError::InvalidBinding)
        );
        assert!(
            targets
                .resolve_depth(Some("test_mod"), None, rt, false)
                .is_ok()
        );

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut primitives = WorldPrimitives::new(&device, (format, false));
        primitives
            .pipelines
            .build(&device, &set, None)
            .expect("build offscreen pipelines");

        // Orthographic projection mapping [0..32, 0..32] into clip space [-1..1].
        let ortho32 = Mat4::orthographic_rh(0.0, 32.0, 32.0, 0.0, -1.0, 1.0).to_cols_array();
        // `PipelineVertex.color` packs red in the low byte (0xAABBGGRR).
        let red_quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFF00_00FF,
            },
            PipelineVertex {
                pos: [0.0, 32.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFF00_00FF,
            },
            PipelineVertex {
                pos: [32.0, 32.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFF00_00FF,
            },
            PipelineVertex {
                pos: [32.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFF00_00FF,
            },
        ];
        let red_vbytes = bytemuck::cast_slice(&red_quad);
        let write_draw = PipelineDraw {
            slot: 0,
            stage: 0,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..red_vbytes.len() as u64)],
                index_range: None,
            },
            bindings: None,
        };

        // Pass 1: clear to black, draw red quad into offscreen target.
        let resolved_rt = targets.resolve_target(Some("test_mod"), rt).unwrap();
        let pass1 = mod_api::OffscreenPass::new(rt)
            .clear_color_0([0.0, 0.0, 0.0, 1.0])
            .clear_depth(0.0)
            .projection(ortho32);
        let mut enc = device.create_command_encoder(&Default::default());
        primitives.execute_offscreen_pass(
            &device,
            &queue,
            &mut enc,
            &OffscreenPassData {
                pass: &pass1,
                target: &resolved_rt,
                draws: std::slice::from_ref(&write_draw),
                transient_vertices: red_vbytes,
                transient_indices: &[],
                inline_uniforms: &[],
                texel_data: &[],
            },
        );
        // Pass 2: LoadOp::Load (no clear) with zero draws — pixels must persist!
        let pass2 = mod_api::OffscreenPass::new(rt);
        primitives.execute_offscreen_pass(
            &device,
            &queue,
            &mut enc,
            &OffscreenPassData {
                pass: &pass2,
                target: &resolved_rt,
                draws: &[],
                transient_vertices: &[],
                transient_indices: &[],
                inline_uniforms: &[],
                texel_data: &[],
            },
        );
        queue.submit([enc.finish()]);

        // Same-size resize is a no-op that preserves existing contents and view_id.
        let (view_id_before, _) = targets
            .resolve_color(Some("test_mod"), None, rt, 0, false, false)
            .unwrap();
        targets
            .resize(&device, Some("test_mod"), rt, [32, 32])
            .unwrap();
        let (view_id_after_noop, rt_view) = targets
            .resolve_color(Some("test_mod"), None, rt, 0, false, false)
            .unwrap();
        assert_eq!(view_id_before, view_id_after_noop);

        // Sample the offscreen target in a world pass and verify the red pixels survived!
        let sample_quad = [
            PipelineVertex {
                pos: [0.0, 0.0, 0.0],
                uv: [0.0, 0.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [0.0, 64.0, 0.0],
                uv: [0.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 64.0, 0.0],
                uv: [1.0, 1.0],
                color: 0xFFFF_FFFF,
            },
            PipelineVertex {
                pos: [64.0, 0.0, 0.0],
                uv: [1.0, 0.0],
                color: 0xFFFF_FFFF,
            },
        ];
        let sample_vbytes = bytemuck::cast_slice(&sample_quad);
        let read_draw = PipelineDraw {
            slot: 1,
            stage: 4,
            modifier_stack: 0,
            draw: MeshDraw::vertices(0..4),
            source: PipelineDrawSource::Transient {
                vertex_ranges: vec![(0, 0..sample_vbytes.len() as u64)],
                index_range: None,
            },
            bindings: Some(ResolvedDrawBindings {
                entries: vec![ResolvedSet1Entry::Texture {
                    view_id: view_id_after_noop,
                    view: rt_view,
                }],
                push_constants: None,
            }),
        };

        // Resize to 64x64 changes dimensions and view_id; destroy retires handle.
        let gpu = (device, queue);
        let batch = Batch {
            set: Some(&set),
            draws: std::slice::from_ref(&read_draw),
            transient_vertices: sample_vbytes,
            time: 0.0,
            ..Default::default()
        };
        let at = render(&gpu, &mut primitives, format, batch, STAGES - 1);
        assert_near(
            at(32, 32),
            [255.0, 0.0, 0.0, 255.0],
            1.0,
            "sampled offscreen red pixel",
        );

        targets
            .resize(&gpu.0, Some("test_mod"), rt, [64, 64])
            .unwrap();
        assert_eq!(targets.size(Some("test_mod"), rt), Ok([64, 64]));
        let (view_id_resized, _) = targets
            .resolve_color(Some("test_mod"), None, rt, 0, false, false)
            .unwrap();
        assert_ne!(view_id_before, view_id_resized);
        targets.destroy(Some("test_mod"), rt).unwrap();
        assert_eq!(
            targets.size(Some("test_mod"), rt),
            Err(mod_api::RenderTargetError::Stale)
        );
    }
}
