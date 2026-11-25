//! The renderer's side of mod compute pipelines (`RegisterRenderPipelinesEvent::compute_pipeline`):
//! building each translated compute shader on the device after checking it against the device's
//! limits and features, and recording the dispatches a frame-graph compute node holds.
//!
//! Everything a mod can get wrong is checked before the first GPU call, and creation sits in
//! validation and internal error scopes, so a refusal is an `Err` or a log line and never an
//! uncaptured device error.

use super::mod_pipelines::{
    ResolvedSet1Entry, create_mod_samplers, sampler_address_idx, sampler_filter_idx,
};
use crate::mod_compute::{BuiltCompute, ComputeSet, Refusal};
use mod_api::{BindingKind, SamplerFilter};
use wgpu::util::DeviceExt;

/// One recorded dispatch of a compute node. Inline uniform and texel byte ranges in `entries`
/// index this dispatch's own `inline` and `texel` bytes.
#[derive(Clone, Debug)]
pub struct RecordedDispatch {
    pub slot: u32,
    pub entries: Vec<ResolvedSet1Entry>,
    pub inline: Vec<u8>,
    pub texel: Vec<u8>,
    pub kind: DispatchKind,
}

/// How many workgroups a dispatch runs.
#[derive(Clone, Debug)]
pub enum DispatchKind {
    Direct([u32; 3]),
    /// Three `u32` counts read from `buffer` at `offset` (a multiple of 4, with 12 bytes in
    /// range: checked when the dispatch is recorded).
    Indirect {
        buffer: wgpu::Buffer,
        offset: u64,
    },
}

struct CompiledCompute {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The bind-group binding each declared entry starts at, in declaration order.
    bindings: Vec<(u32, BindingKind)>,
    label: String,
}

/// The compute pipelines built on one device for one load.
#[derive(Default)]
pub struct ModCompute {
    generation: u64,
    pipelines: Vec<Option<CompiledCompute>>,
    samplers: Option<[[wgpu::Sampler; 3]; 2]>,
}

/// The texel format the shader declares for the storage image at `binding`.
fn storage_format(built: &BuiltCompute, binding: u32) -> Option<wgpu::TextureFormat> {
    let module = &built.shader.module;
    module
        .global_variables
        .iter()
        .filter(|(_, var)| {
            var.binding.as_ref().is_some_and(|at| {
                at.group == sunrise_mod_pipeline::COMPUTE_BIND_GROUP && at.binding == binding
            })
        })
        .find_map(|(_, var)| match module.types[var.ty].inner {
            naga::TypeInner::Image {
                class: naga::ImageClass::Storage { format, .. },
                ..
            } => Some(format),
            _ => None,
        })
        .and_then(|format| match format {
            naga::StorageFormat::Rgba8Unorm => Some(wgpu::TextureFormat::Rgba8Unorm),
            naga::StorageFormat::Rgba16Float => Some(wgpu::TextureFormat::Rgba16Float),
            naga::StorageFormat::R32Float => Some(wgpu::TextureFormat::R32Float),
            _ => None,
        })
}

/// Check `built` against the device's limits and features; the error says which and the
/// [`Refusal`] how a handle to it resolves.
fn check_limits(device: &wgpu::Device, built: &BuiltCompute) -> Result<(), (Refusal, String)> {
    let limits = device.limits();
    let shader = &built.shader;
    if shader.uses_subgroup && !device.features().contains(wgpu::Features::SUBGROUP) {
        return Err((
            Refusal::UnsupportedFeature,
            "it uses subgroup operations the adapter lacks".to_owned(),
        ));
    }
    let invalid = |reason: String| Err((Refusal::Invalid, reason));
    let [x, y, z] = shader.workgroup_size;
    for (axis, size, max) in [
        ("x", x, limits.max_compute_workgroup_size_x),
        ("y", y, limits.max_compute_workgroup_size_y),
        ("z", z, limits.max_compute_workgroup_size_z),
    ] {
        if size > max {
            return invalid(format!(
                "workgroup size {axis} ({size}) exceeds device max_compute_workgroup_size_{axis} ({max})"
            ));
        }
    }
    let invocations = u64::from(x) * u64::from(y) * u64::from(z);
    if invocations > u64::from(limits.max_compute_invocations_per_workgroup) {
        return invalid(format!(
            "{invocations} invocations per workgroup exceed device max_compute_invocations_per_workgroup ({})",
            limits.max_compute_invocations_per_workgroup
        ));
    }
    if shader.workgroup_storage_bytes > limits.max_compute_workgroup_storage_size {
        return invalid(format!(
            "workgroup storage ({} bytes) exceeds device max_compute_workgroup_storage_size ({})",
            shader.workgroup_storage_bytes, limits.max_compute_workgroup_storage_size
        ));
    }
    let (mut uniforms, mut textures, mut samplers, mut storage, mut images) = (0, 0, 0, 0, 0);
    for entry in &built.bind_group_layout.entries {
        match entry.kind {
            BindingKind::UniformBuffer { min_size } => {
                if u64::from(min_size) > limits.max_uniform_buffer_binding_size {
                    return invalid(format!(
                        "uniform buffer `{}` min_size ({min_size}) exceeds device max_uniform_buffer_binding_size ({})",
                        entry.name, limits.max_uniform_buffer_binding_size
                    ));
                }
                uniforms += 1;
            }
            BindingKind::CombinedImageSampler { .. } => {
                textures += 1;
                samplers += 1;
            }
            BindingKind::TexelBuffer { .. } | BindingKind::StorageBuffer { .. } => storage += 1,
            BindingKind::StorageImage { .. } => images += 1,
            _ => {
                return invalid(format!(
                    "unsupported bind-group entry kind for `{}`",
                    entry.name
                ));
            }
        }
    }
    for (what, count, max, limit) in [
        (
            "uniform buffer",
            uniforms,
            limits.max_uniform_buffers_per_shader_stage,
            "max_uniform_buffers_per_shader_stage",
        ),
        (
            "sampled texture",
            textures,
            limits.max_sampled_textures_per_shader_stage,
            "max_sampled_textures_per_shader_stage",
        ),
        (
            "sampler",
            samplers,
            limits.max_samplers_per_shader_stage,
            "max_samplers_per_shader_stage",
        ),
        (
            "storage/texel buffer",
            storage,
            limits.max_storage_buffers_per_shader_stage,
            "max_storage_buffers_per_shader_stage",
        ),
        (
            "storage image",
            images,
            limits.max_storage_textures_per_shader_stage,
            "max_storage_textures_per_shader_stage",
        ),
    ] {
        if count > max {
            return invalid(format!(
                "{what} count ({count}) exceeds device {limit} ({max})"
            ));
        }
    }
    Ok(())
}

fn compile(
    device: &wgpu::Device,
    built: &BuiltCompute,
) -> Result<CompiledCompute, (Refusal, String)> {
    check_limits(device, built)?;
    let compute = wgpu::ShaderStages::COMPUTE;
    let mut layout_entries = Vec::new();
    let mut bindings = Vec::new();
    let mut next = 0u32;
    for entry in &built.bind_group_layout.entries {
        bindings.push((next, entry.kind));
        match entry.kind {
            BindingKind::UniformBuffer { min_size } => {
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(u64::from(min_size)),
                    },
                    count: None,
                });
                next += 1;
            }
            BindingKind::CombinedImageSampler { filter, .. } => {
                let filterable = filter == SamplerFilter::Linear;
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next + 1,
                    visibility: compute,
                    ty: wgpu::BindingType::Sampler(if filterable {
                        wgpu::SamplerBindingType::Filtering
                    } else {
                        wgpu::SamplerBindingType::NonFiltering
                    }),
                    count: None,
                });
                next += 2;
            }
            BindingKind::TexelBuffer { .. } | BindingKind::StorageBuffer { .. } => {
                let read_only =
                    !matches!(entry.kind, BindingKind::StorageBuffer { read_only: false });
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(4),
                    },
                    count: None,
                });
                next += 1;
            }
            BindingKind::StorageImage { access } => {
                let Some(format) = storage_format(built, next) else {
                    return Err((
                        Refusal::Invalid,
                        format!(
                            "storage image `{}` needs a `layout(rgba8)`, `layout(rgba16f)` or `layout(r32f)` declaration in its shader",
                            entry.name
                        ),
                    ));
                };
                let access = match access {
                    mod_api::StorageImageAccess::WriteOnly => wgpu::StorageTextureAccess::WriteOnly,
                    mod_api::StorageImageAccess::ReadOnly => wgpu::StorageTextureAccess::ReadOnly,
                    mod_api::StorageImageAccess::ReadWrite => wgpu::StorageTextureAccess::ReadWrite,
                    _ => {
                        return Err((
                            Refusal::Invalid,
                            format!("unsupported storage image access for `{}`", entry.name),
                        ));
                    }
                };
                layout_entries.push(wgpu::BindGroupLayoutEntry {
                    binding: next,
                    visibility: compute,
                    ty: wgpu::BindingType::StorageTexture {
                        access,
                        format,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                });
                next += 1;
            }
            _ => unreachable!("check_limits refuses unknown entry kinds"),
        }
    }
    let entry_point = built.shader.module.entry_points[0].name.clone();
    let label = built.id.to_string();
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&label),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(built.shader.module.clone())),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&label),
        entries: &layout_entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&label),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(&entry_point),
        compilation_options: Default::default(),
        cache: None,
    });
    let validation = pollster::block_on(validation.pop());
    let internal = pollster::block_on(internal.pop());
    if let Some(error) = validation.or(internal) {
        return Err((Refusal::Invalid, error.to_string()));
    }
    Ok(CompiledCompute {
        pipeline,
        layout,
        bindings,
        label,
    })
}

impl ModCompute {
    /// Builds the pipelines of `set` unless they are built already. A pipeline the device
    /// refuses is logged once, marked in `set` (so its handle resolves `Invalid` or
    /// `UnsupportedFeature`) and left out; the others still build.
    pub(super) fn build(&mut self, device: &wgpu::Device, set: &ComputeSet) {
        if set.generation() == self.generation {
            return;
        }
        self.pipelines = set
            .built()
            .iter()
            .enumerate()
            .map(|(slot, built)| {
                let built = built.as_ref()?;
                match compile(device, built) {
                    Ok(compiled) => Some(compiled),
                    Err((reason, error)) => {
                        if set.refuse(slot, reason) {
                            log::warn!(
                                "Sunrise: mod {} compute pipeline {} refused by the device: {error}",
                                built.owner,
                                built.id
                            );
                        }
                        None
                    }
                }
            })
            .collect();
        self.generation = set.generation();
    }

    /// Record `dispatches` on `encoder`, each in its own compute pass. A dispatch that cannot
    /// run (an unbuilt slot, a count over the device's limit, an indirect range out of its
    /// buffer, a bind group the device refuses) is logged and skipped.
    pub(super) fn record(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        dispatches: &[RecordedDispatch],
    ) {
        let max_groups = device.limits().max_compute_workgroups_per_dimension;
        let samplers = self
            .samplers
            .get_or_insert_with(|| create_mod_samplers(device));
        for dispatch in dispatches {
            let Some(Some(compiled)) = self.pipelines.get(dispatch.slot as usize) else {
                continue;
            };
            match &dispatch.kind {
                DispatchKind::Direct(counts) => {
                    if counts.iter().any(|&n| n > max_groups) {
                        log::warn!(
                            "Sunrise: compute dispatch of {} skipped: {counts:?} exceeds device max_compute_workgroups_per_dimension ({max_groups})",
                            compiled.label
                        );
                        continue;
                    }
                    if counts.contains(&0) {
                        continue;
                    }
                }
                DispatchKind::Indirect { buffer, offset } => {
                    if offset % 4 != 0 || offset.saturating_add(12) > buffer.size() {
                        log::warn!(
                            "Sunrise: indirect compute dispatch of {} skipped: offset {offset} is not a 4-aligned range of 12 bytes in its buffer",
                            compiled.label
                        );
                        continue;
                    }
                }
            }
            let mut owned = Vec::new();
            let mut entries = Vec::new();
            let mut ok = true;
            for (resolved, (binding, kind)) in dispatch.entries.iter().zip(&compiled.bindings) {
                match (resolved, kind) {
                    (
                        ResolvedSet1Entry::UniformBuffer {
                            buffer, byte_len, ..
                        },
                        BindingKind::UniformBuffer { .. },
                    ) => entries.push(wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(*byte_len),
                        }),
                    }),
                    (
                        ResolvedSet1Entry::InlineUniform { byte_range },
                        BindingKind::UniformBuffer { .. },
                    ) => {
                        let Some(bytes) = dispatch.inline.get(byte_range.clone()) else {
                            ok = false;
                            break;
                        };
                        owned.push((*binding, bytes, wgpu::BufferUsages::UNIFORM));
                    }
                    (
                        ResolvedSet1Entry::TexelBuffer { byte_range },
                        BindingKind::TexelBuffer { .. },
                    ) => {
                        let Some(bytes) = dispatch.texel.get(byte_range.clone()) else {
                            ok = false;
                            break;
                        };
                        owned.push((*binding, bytes, wgpu::BufferUsages::STORAGE));
                    }
                    (
                        ResolvedSet1Entry::Texture { view, .. },
                        BindingKind::CombinedImageSampler { filter, address },
                    ) => {
                        entries.push(wgpu::BindGroupEntry {
                            binding: *binding,
                            resource: wgpu::BindingResource::TextureView(view),
                        });
                        entries.push(wgpu::BindGroupEntry {
                            binding: binding + 1,
                            resource: wgpu::BindingResource::Sampler(
                                &samplers[sampler_filter_idx(*filter)]
                                    [sampler_address_idx(*address)],
                            ),
                        });
                    }
                    (
                        ResolvedSet1Entry::StorageBuffer {
                            buffer,
                            offset,
                            size,
                            ..
                        },
                        BindingKind::StorageBuffer { .. },
                    ) => entries.push(wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: *offset,
                            size: wgpu::BufferSize::new(*size),
                        }),
                    }),
                    (
                        ResolvedSet1Entry::StorageImage { view, .. },
                        BindingKind::StorageImage { .. },
                    ) => {
                        entries.push(wgpu::BindGroupEntry {
                            binding: *binding,
                            resource: wgpu::BindingResource::TextureView(view),
                        });
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || dispatch.entries.len() != compiled.bindings.len() {
                log::warn!(
                    "Sunrise: compute dispatch of {} skipped: its bindings do not match its layout",
                    compiled.label
                );
                continue;
            }
            let buffers: Vec<(u32, wgpu::Buffer)> = owned
                .iter()
                .map(|(binding, bytes, usage)| {
                    (
                        *binding,
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Mod compute inline data"),
                            contents: bytes,
                            usage: *usage,
                        }),
                    )
                })
                .collect();
            for (binding, buffer) in &buffers {
                entries.push(wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: buffer.as_entire_binding(),
                });
            }
            let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&compiled.label),
                layout: &compiled.layout,
                entries: &entries,
            });
            if let Some(error) = pollster::block_on(validation.pop()) {
                log::warn!(
                    "Sunrise: compute dispatch of {} skipped: its bindings were refused: {error}",
                    compiled.label
                );
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&compiled.label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&compiled.pipeline);
            pass.set_bind_group(0, &group, &[]);
            match &dispatch.kind {
                DispatchKind::Direct([x, y, z]) => pass.dispatch_workgroups(*x, *y, *z),
                DispatchKind::Indirect { buffer, offset } => {
                    pass.dispatch_workgroups_indirect(buffer, *offset)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{BindEntry, BindGroupLayout, ComputePipeline, Identifier};

    const COUNT: &str = "#version 450\nlayout(local_size_x = 8) in;\nlayout(std430) buffer Out { uint values[]; };\nvoid main() { values[gl_GlobalInvocationID.x] = gl_GlobalInvocationID.x * 3u + 1u; }\n";
    const SUBGROUP: &str = "#version 450\n#extension GL_KHR_shader_subgroup_basic : enable\nlayout(local_size_x = 8) in;\nlayout(std430) buffer Out { uint values[]; };\nvoid main() { values[gl_GlobalInvocationID.x] = gl_SubgroupSize; }\n";

    fn pipeline(name: &str) -> ComputePipeline {
        let layout = BindGroupLayout::builder()
            .entry(BindEntry::new(
                "Out",
                BindingKind::StorageBuffer { read_only: false },
            ))
            .build()
            .unwrap();
        ComputePipeline::builder(
            Identifier::parse(name).unwrap(),
            Identifier::parse(name).unwrap(),
        )
        .bind_group_layout(layout)
        .build()
    }

    #[test]
    #[ignore = "daily"]
    fn a_dispatch_writes_its_storage_buffer_and_a_subgroup_shader_follows_the_adapter() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((adapter, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let set = ComputeSet::build(&[pipeline("t:count"), pipeline("t:sub")], 1, |file| {
            Ok(match file {
                "assets/t/shaders/compute/count.csh" => Some(COUNT.to_owned()),
                "assets/t/shaders/compute/sub.csh" => Some(SUBGROUP.to_owned()),
                _ => None,
            })
        });
        let mut compute = ModCompute::default();
        compute.build(&device, &set);
        let subgroup = device.features().contains(wgpu::Features::SUBGROUP);
        assert_eq!(
            set.resolve(&Identifier::parse("t:sub").unwrap()).is_ok(),
            set.built()[1].is_some() && subgroup,
            "adapter subgroup support: {}",
            adapter.features().contains(wgpu::Features::SUBGROUP)
        );
        let handle = set.resolve(&Identifier::parse("t:count").unwrap()).unwrap();
        assert_eq!(set.slot(handle), Ok(0));

        let storage = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: 32,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: 32,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dispatch = |counts| RecordedDispatch {
            slot: 0,
            entries: vec![ResolvedSet1Entry::StorageBuffer {
                id: 1,
                buffer: storage.clone(),
                offset: 0,
                size: 32,
            }],
            inline: Vec::new(),
            texel: Vec::new(),
            kind: DispatchKind::Direct(counts),
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        // Over the device limit and zero-sized dispatches are skipped; the third one runs.
        compute.record(
            &device,
            &mut encoder,
            &[
                dispatch([u32::MAX, 1, 1]),
                dispatch([0, 1, 1]),
                dispatch([1, 1, 1]),
            ],
        );
        encoder.copy_buffer_to_buffer(&storage, 0, &readback, 0, 32);
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = readback.slice(..).get_mapped_range();
        let values: &[u32] = bytemuck::cast_slice(&data);
        assert_eq!(values, [1, 4, 7, 10, 13, 16, 19, 22]);
    }
}
