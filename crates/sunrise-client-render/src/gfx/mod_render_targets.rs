//! Mod-owned offscreen GPU render targets backing [`mod_api::RenderTargetRef`].
//!
//! Target dimensions are bounded strictly by `device.limits().max_texture_dimension_2d` and
//! colour attachment counts by [`mod_api::MAX_COLOR_TARGETS`] (zero uncited per-mod cap).
//! Targets always retain their contents across frames until cleared by an
//! [`mod_api::OffscreenPass`] or reallocated by [`mod_api::RenderTargetRef::resize`]. Calling
//! `resize` with the current dimensions is a no-op that preserves existing pixels.
//!
//! Handles are generational, owner-checked, and retired on mod disable, world close, resource
//! reload, or explicit [`mod_api::RenderTargetRef::destroy`].

use mod_api::__private::game::Fields;
use mod_api::{
    OffscreenPass, PipelineDrawError, RenderTargetDepth, RenderTargetDescriptor, RenderTargetError,
    RenderTargetFormat, RenderTargetRef,
};
use std::collections::BTreeSet;

/// Starting view ID for mod render target views, disjoint from [`super::mod_uniforms`] view IDs
/// so `ModPipelines`' set-1 bind-group cache keys never collide.
const FIRST_TARGET_VIEW_ID: u64 = 1 << 32;

/// Convert a mod API colour attachment format to its `wgpu::TextureFormat`.
pub fn wgpu_color_format(format: RenderTargetFormat) -> wgpu::TextureFormat {
    match format {
        RenderTargetFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        RenderTargetFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        RenderTargetFormat::R32Float => wgpu::TextureFormat::R32Float,
        _ => wgpu::TextureFormat::Rgba8Unorm,
    }
}

/// Convert a mod API depth/stencil format to its `wgpu::TextureFormat`.
pub fn wgpu_depth_format(depth: RenderTargetDepth) -> Option<wgpu::TextureFormat> {
    match depth {
        RenderTargetDepth::None => None,
        RenderTargetDepth::Depth32Float => Some(wgpu::TextureFormat::Depth32Float),
        RenderTargetDepth::Depth24PlusStencil8 => Some(wgpu::TextureFormat::Depth24PlusStencil8),
        _ => None,
    }
}

/// Cloned texture views and format metadata for executing an [`OffscreenPass`] without holding
/// the [`ModRenderTargets`] mutex across GPU pass recording.
pub struct ResolvedOffscreenTarget {
    pub handle: RenderTargetRef,
    pub size: [u32; 2],
    pub color_views: Vec<wgpu::TextureView>,
    pub color_formats: Vec<wgpu::TextureFormat>,
    pub depth_view: Option<wgpu::TextureView>,
    pub depth_format: Option<wgpu::TextureFormat>,
    pub has_stencil: bool,
}

struct ColorAttachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    view_id: u64,
    format: wgpu::TextureFormat,
}

struct DepthAttachment {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    attachment_view: wgpu::TextureView,
    sample_view: wgpu::TextureView,
    view_id: u64,
    format: wgpu::TextureFormat,
    has_stencil: bool,
}

struct ModRenderTarget {
    generation: u64,
    owner: Option<&'static str>,
    size: [u32; 2],
    color_formats: Vec<RenderTargetFormat>,
    depth_format: RenderTargetDepth,
    colors: Vec<ColorAttachment>,
    depth: Option<DepthAttachment>,
}

/// Generational store of mod-owned offscreen GPU render targets (`RenderTargetRef`).
pub struct ModRenderTargets {
    next_generation: u64,
    slots: Vec<Option<ModRenderTarget>>,
    next_view_id: u64,
}

impl Default for ModRenderTargets {
    fn default() -> Self {
        Self {
            next_generation: 0,
            slots: Vec::new(),
            next_view_id: FIRST_TARGET_VIEW_ID,
        }
    }
}

impl ModRenderTargets {
    fn alloc_view_id(&mut self) -> u64 {
        let id = self.next_view_id;
        self.next_view_id = self
            .next_view_id
            .checked_add(1)
            .expect("mod render target view id exhausted");
        id
    }

    fn allocate_attachments(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        color_formats: &[RenderTargetFormat],
        depth_format: RenderTargetDepth,
    ) -> (Vec<ColorAttachment>, Option<DepthAttachment>) {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let mut colors = Vec::with_capacity(color_formats.len());
        for (idx, &fmt) in color_formats.iter().enumerate() {
            let format = wgpu_color_format(fmt);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Mod render target color {idx}")),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let view_id = self.alloc_view_id();
            colors.push(ColorAttachment {
                texture,
                view,
                view_id,
                format,
            });
        }

        let depth = wgpu_depth_format(depth_format).map(|format| {
            let has_stencil = matches!(format, wgpu::TextureFormat::Depth24PlusStencil8);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Mod render target depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let attachment_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..Default::default()
            });
            let view_id = self.alloc_view_id();
            DepthAttachment {
                texture,
                attachment_view,
                sample_view,
                view_id,
                format,
                has_stencil,
            }
        });

        (colors, depth)
    }

    /// Allocate a mod-owned offscreen GPU render target.
    pub fn create(
        &mut self,
        device: &wgpu::Device,
        owner: Option<&'static str>,
        desc: &RenderTargetDescriptor,
    ) -> Result<RenderTargetRef, RenderTargetError> {
        desc.validate(device.limits().max_texture_dimension_2d)?;
        let (colors, depth) =
            self.allocate_attachments(device, desc.size, &desc.color_targets, desc.depth);
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("mod render target generation exhausted");
        let generation = self.next_generation;
        let entry = ModRenderTarget {
            generation,
            owner,
            size: desc.size,
            color_formats: desc.color_targets.clone(),
            depth_format: desc.depth,
            colors,
            depth,
        };
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.slots.len());
        if slot == self.slots.len() {
            self.slots.push(Some(entry));
        } else {
            self.slots[slot] = Some(entry);
        }
        Ok(mod_api::__private::built::render_target_ref(
            generation,
            slot as u32,
        ))
    }

    /// Resize `handle` to `size`. Calling with the target's current dimensions is a no-op that
    /// preserves existing attachment contents.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
        size: [u32; 2],
    ) -> Result<(), RenderTargetError> {
        let max_dim = device.limits().max_texture_dimension_2d;
        if size[0] == 0 || size[1] == 0 || size[0] > max_dim || size[1] > max_dim {
            return Err(RenderTargetError::InvalidSize);
        }
        let (current_size, color_formats, depth_format) = {
            let entry = self.get_owned(owner, handle)?;
            (entry.size, entry.color_formats.clone(), entry.depth_format)
        };
        if current_size == size {
            return Ok(());
        }
        let (colors, depth) = self.allocate_attachments(device, size, &color_formats, depth_format);
        let slot = handle.fields().slot as usize;
        if let Some(entry) = self.slots.get_mut(slot).and_then(Option::as_mut) {
            entry.size = size;
            entry.colors = colors;
            entry.depth = depth;
        }
        Ok(())
    }

    /// Current `[width, height]` pixel dimensions of `handle`.
    pub fn size(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<[u32; 2], RenderTargetError> {
        Ok(self.get_owned(owner, handle)?.size)
    }

    /// Free `handle`'s GPU textures and retire it (`RenderTargetError::Stale` thereafter).
    pub fn destroy(
        &mut self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<(), RenderTargetError> {
        self.get_owned(owner, handle)?;
        let slot = handle.fields().slot as usize;
        self.slots[slot] = None;
        Ok(())
    }

    /// Retire all mod render targets (called on world close and resource reload).
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Retire all mod render targets owned by any mod in `disabled`.
    pub fn retire_disabled(&mut self, disabled: &BTreeSet<String>) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|rt| rt.owner.is_some_and(|id| disabled.contains(id)))
            {
                *slot = None;
            }
        }
    }

    /// Resolve a colour attachment for shader sampling (`TextureSource::RenderTargetColor`),
    /// enforcing the self-sampling hazard check (`PipelineDrawError::TargetReadWriteHazard`).
    pub fn resolve_color(
        &self,
        owner: Option<&'static str>,
        active_target: Option<RenderTargetRef>,
        handle: RenderTargetRef,
        attachment: u8,
        filterable: bool,
        float32_filterable: bool,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        if active_target == Some(handle) {
            return Err(PipelineDrawError::TargetReadWriteHazard);
        }
        let entry = self
            .get_owned(owner, handle)
            .map_err(|_| PipelineDrawError::InvalidBinding)?;
        let color = entry
            .colors
            .get(attachment as usize)
            .ok_or(PipelineDrawError::InvalidBinding)?;
        if filterable && color.format == wgpu::TextureFormat::R32Float && !float32_filterable {
            return Err(PipelineDrawError::InvalidBinding);
        }
        Ok((color.view_id, color.view.clone()))
    }

    /// Resolve the depth aspect for shader sampling (`TextureSource::RenderTargetDepth`),
    /// enforcing the self-sampling hazard check (`PipelineDrawError::TargetReadWriteHazard`).
    pub fn resolve_depth(
        &self,
        owner: Option<&'static str>,
        active_target: Option<RenderTargetRef>,
        handle: RenderTargetRef,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        if active_target == Some(handle) {
            return Err(PipelineDrawError::TargetReadWriteHazard);
        }
        if filterable {
            return Err(PipelineDrawError::InvalidBinding);
        }
        let entry = self
            .get_owned(owner, handle)
            .map_err(|_| PipelineDrawError::InvalidBinding)?;
        let depth = entry
            .depth
            .as_ref()
            .ok_or(PipelineDrawError::InvalidBinding)?;
        Ok((depth.view_id, depth.sample_view.clone()))
    }

    /// Snapshot attachment views and format metadata for executing an [`OffscreenPass`].
    pub fn resolve_target(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<ResolvedOffscreenTarget, RenderTargetError> {
        let entry = self.get_owned(owner, handle)?;
        Ok(ResolvedOffscreenTarget {
            handle,
            size: entry.size,
            color_views: entry.colors.iter().map(|c| c.view.clone()).collect(),
            color_formats: entry.colors.iter().map(|c| c.format).collect(),
            depth_view: entry.depth.as_ref().map(|d| d.attachment_view.clone()),
            depth_format: entry.depth.as_ref().map(|d| d.format),
            has_stencil: entry.depth.as_ref().is_some_and(|d| d.has_stencil),
        })
    }

    /// Access the underlying colour texture of `handle` (used by GPU readback tests).
    #[cfg(test)]
    pub(crate) fn color_texture(
        &self,
        handle: RenderTargetRef,
        attachment: u8,
    ) -> Option<&wgpu::Texture> {
        let fields = handle.fields();
        let entry = self.slots.get(fields.slot as usize)?.as_ref()?;
        if entry.generation != fields.generation {
            return None;
        }
        entry.colors.get(attachment as usize).map(|c| &c.texture)
    }

    fn get_owned(
        &self,
        owner: Option<&'static str>,
        handle: RenderTargetRef,
    ) -> Result<&ModRenderTarget, RenderTargetError> {
        let fields = handle.fields();
        let Some(entry) = self
            .slots
            .get(fields.slot as usize)
            .and_then(Option::as_ref)
        else {
            return Err(RenderTargetError::Stale);
        };
        if entry.generation != fields.generation {
            return Err(RenderTargetError::Stale);
        }
        if entry.owner.is_some() && entry.owner != owner {
            return Err(RenderTargetError::NotOwned);
        }
        Ok(entry)
    }
}

impl super::Renderer {
    /// Allocate a mod-owned offscreen [`RenderTargetRef`] on this renderer's device.
    pub fn create_render_target(
        &self,
        owner: Option<&'static str>,
        desc: &RenderTargetDescriptor,
    ) -> Result<RenderTargetRef, RenderTargetError> {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .create(&self.device, owner, desc)
    }

    /// Resize a mod-owned offscreen [`RenderTargetRef`] (no-op if unchanged).
    pub fn render_target_resize(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
        size: [u32; 2],
    ) -> Result<(), RenderTargetError> {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resize(&self.device, owner, target, size)
    }

    /// Query the current pixel dimensions of a mod-owned offscreen [`RenderTargetRef`].
    pub fn render_target_size(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
    ) -> Result<[u32; 2], RenderTargetError> {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .size(owner, target)
    }

    /// Free a mod-owned offscreen [`RenderTargetRef`] and retire its handle.
    pub fn render_target_destroy(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
    ) -> Result<(), RenderTargetError> {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .destroy(owner, target)
    }

    /// Retire all mod-owned offscreen render targets and pooled transient targets (world close,
    /// resource reload).
    pub fn clear_mod_render_targets(&self) {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.transient_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.warned_frame_graph_cycles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Maximum 2D texture dimension supported by this renderer's GPU device.
    pub fn max_texture_dimension_2d(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Whether this renderer's GPU device supports linear filtering on `R32Float` textures.
    pub fn float32_filterable(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    }

    /// Record a distinct frame-graph cycle warning once per reload/world session.
    pub fn warn_frame_graph_cycle_once(&self, passes: &[String]) {
        let mut warned = self
            .warned_frame_graph_cycles
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if warned.insert(passes.to_vec()) {
            log::warn!(
                "Sunrise: frame graph cycle detected between {}",
                passes.join(", ")
            );
        }
    }

    /// Retire all mod-owned offscreen render targets owned by any mod in `disabled`.
    pub fn retire_disabled_mod_render_targets(&self, disabled: &BTreeSet<String>) {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retire_disabled(disabled);
    }

    /// Resolve a mod render target colour attachment for set-1 sampling.
    pub fn resolve_mod_render_target_color(
        &self,
        owner: Option<&'static str>,
        active_target: Option<RenderTargetRef>,
        target: RenderTargetRef,
        attachment: u8,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        let float32_filterable = self
            .device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE);
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_color(
                owner,
                active_target,
                target,
                attachment,
                filterable,
                float32_filterable,
            )
    }

    /// Resolve a mod render target depth aspect for set-1 sampling.
    pub fn resolve_mod_render_target_depth(
        &self,
        owner: Option<&'static str>,
        active_target: Option<RenderTargetRef>,
        target: RenderTargetRef,
        filterable: bool,
    ) -> Result<(u64, wgpu::TextureView), PipelineDrawError> {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_depth(owner, active_target, target, filterable)
    }

    /// Snapshot attachment views for executing an [`OffscreenPass`].
    pub fn resolve_mod_render_target(
        &self,
        owner: Option<&'static str>,
        target: RenderTargetRef,
    ) -> Result<ResolvedOffscreenTarget, RenderTargetError> {
        self.mod_render_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve_target(owner, target)
    }

    /// Record an offscreen pass into an active `CommandEncoder` (shared seam for
    /// `frame_graph_setup` and picture-in-picture GUI renderers).
    #[allow(clippy::too_many_arguments)]
    pub fn record_mod_offscreen_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        owner: Option<&'static str>,
        pass: &OffscreenPass,
        draws: &[super::mod_pipelines::PipelineDraw],
        transient_vertices: &[u8],
        transient_indices: &[u8],
        inline_uniforms: &[u8],
        texel_data: &[u8],
    ) -> Result<(), RenderTargetError> {
        let target = self.resolve_mod_render_target(owner, pass.target)?;
        let Some(primitives) = self.world_primitives.as_mut() else {
            return Err(RenderTargetError::Unavailable);
        };
        primitives.execute_offscreen_pass(
            &self.device,
            &self.queue,
            encoder,
            &super::mod_pipelines::OffscreenPassData {
                pass,
                target: &target,
                draws,
                transient_vertices,
                transient_indices,
                inline_uniforms,
                texel_data,
            },
        );
        Ok(())
    }

    /// Execute and submit a standalone offscreen pass onto a mod-owned [`RenderTargetRef`]
    /// (for picture-in-picture GUI renderers running outside the world frame encoder).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_mod_offscreen_pass(
        &mut self,
        owner: Option<&'static str>,
        pass: &OffscreenPass,
        draws: &[super::mod_pipelines::PipelineDraw],
        transient_vertices: &[u8],
        transient_indices: &[u8],
        inline_uniforms: &[u8],
        texel_data: &[u8],
    ) -> Result<(), RenderTargetError> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Mod offscreen render pass"),
            });
        self.record_mod_offscreen_pass(
            &mut encoder,
            owner,
            pass,
            draws,
            transient_vertices,
            transient_indices,
            inline_uniforms,
            texel_data,
        )?;
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}
