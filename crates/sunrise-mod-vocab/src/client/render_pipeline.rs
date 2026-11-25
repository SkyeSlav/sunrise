//! Render pipelines a client mod registers as plain data, and the quads it draws with them.
use crate::{Identifier, WorldDepth};

/// A pipeline registered by `Hooks::register_render_pipelines`: a GLSL core shader program
/// (`assets/<ns>/shaders/core/<path>.vsh` and `.fsh`, optionally reusing another program's
/// vertex stage), its preprocessor defines, optional sprite texture, and fixed blend/depth
/// state. The engine translates the GLSL via `sunrise-glsl` and builds every GPU object.
///
/// ## Colour space
/// Mod pipeline GLSL programs work in vanilla's gamma space, matching vanilla core shaders
/// (`texture * vertexColor * ColorModulator`). On frames storing vanilla's encoding the output is
/// stored as written; on frames holding linear light the engine decodes the fragment shader's
/// location-0 output after `main()` (sanitizing negative/NaN RGB to 0 and clamping alpha
/// coverage to `[0, 1]`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline registration descriptor.
pub struct RenderPipeline {
    owner: String,
    /// The pipeline's own name (`mymod:glow`), unique among mod pipelines.
    pub id: Identifier,
    /// The core shader program (`mymod:core/glow` resolves to
    /// `assets/mymod/shaders/core/glow.vsh` and `assets/mymod/shaders/core/glow.fsh`, or
    /// `mymod:glow` resolves to `assets/mymod/shaders/glow.vsh` and `.fsh`), as the mod's assets
    /// or a resource pack supply it. When [`Self::vertex_shader`] is set, this id supplies only
    /// the `.fsh` fragment stage.
    pub fragment: Identifier,
    /// Optional separate vertex shader program id (`withVertexShader`): defaults to
    /// [`Self::fragment`] when `None`. Lets a mod reuse another program's vertex stage such as
    /// `minecraft:core/position_tex_color` or a library mod's vertex shader.
    pub vertex_shader: Option<Identifier>,
    /// Preprocessor defines compiled into both stages (`withShaderDefine`): `(name, value)`.
    /// Each `name` must be a valid non-`GL_` ASCII GLSL identifier, and each `value` (when
    /// present) a non-empty single-line token without comments, backslashes, or `#`.
    pub defines: Vec<(String, Option<String>)>,
    /// What the pipeline's draws are made of.
    pub vertex_format: PipelineVertexFormat,
    /// Custom per-binding vertex formats (`withVertexBinding(bindingIndex, format)`). Empty by
    /// default, meaning binding 0 uses [`VertexFormat::position_tex_color`] (stride 24:
    /// `Position: Float32x3`, `UV0: Float32x2`, `Color: Unorm8x4`).
    pub vertex_bindings: Vec<(u8, VertexFormat)>,
    /// Fixed-function blend, depth, depth-write, depth-bias, culling, colour-write, polygon-mode,
    /// and primitive-topology state.
    pub state: PipelineState,
    /// Per-target colour blend and write-mask overrides for multi-target (MRT) passes, indexed
    /// `0..MAX_COLOR_TARGETS` (`ColorTargetState.MAX_COLOR_TARGETS = 8`). Empty by default,
    /// meaning a single target 0 derived from [`PipelineState::blend`] and
    /// [`PipelineState::color_write`]. Configured via [`Self::color_target`].
    pub color_targets: Vec<Option<ColorTargetState>>,
    /// First out-of-range index passed to [`Self::color_target`], kept so registration refuses
    /// `index >= MAX_COLOR_TARGETS` without panicking or over-allocating.
    invalid_color_target_index: Option<usize>,
    /// Whether this pipeline opts in to order-independent transparency
    /// (`RenderLevelStage::OitTranslucent`, NeoForge's `registerOitPipelineSet`). When `true`
    /// and `state.blend != PipelineBlend::Opaque`, the engine translates three OIT variants at
    /// load (`OIT_DEPTH_BOUNDS`, `OIT_TRANSMITTANCE`, `OIT_ACCUMULATE`, plus `OIT_ADDITIVE` for
    /// [`PipelineBlend::Additive`]).
    pub order_independent: bool,
    /// Whether a resource load may go on without this pipeline (NeoForge's
    /// `registerOptionalPipeline`). A required pipeline (NeoForge's `registerPipeline`, the
    /// default) that a load cannot build fails the load: the game does not start, and a reload
    /// fails and keeps the previous resources. An optional one is logged once by id and resolves
    /// to [`RenderPipelineError::Invalid`] while the other pipelines draw.
    pub optional: bool,
    /// What the shader samples via `Sampler0` / `<sunrise:pipeline_sprite.glsl>`'s
    /// `sampleSprite(uv)`, if anything ([`Self::textured`]). `None`, the default, binds no
    /// texture: referencing `Sampler0` or `SpriteRect` refuses the pipeline.
    pub texture: Option<PipelineTexture>,
    /// What kind of world geometry this pipeline draws ([`Self::geometry_class`]), so an active
    /// Iris/OptiFine shader pack can route it through the matching standard `gbuffers_*`
    /// fallback chain. `None`, the default, draws only with the pipeline's own shader.
    pub geometry_class: Option<PipelineGeometryClass>,
    /// Optional shader-pack integration extras ([`Self::pack_binding`]): prepended pack program
    /// names, pass slot override, auxiliary depth target, per-frame uniforms, preprocessor
    /// defines, and GLSL compatibility bindings. All names are supplied by the mod; the engine
    /// hardcodes none.
    pub pack_binding: Option<PackBinding>,
}
impl RenderPipeline {
    /// Describe a required pipeline that draws [`PipelineVertexFormat::PositionTexColor`]
    /// geometry with the GLSL core program `shader` (`ns:core/path` ->
    /// `assets/<ns>/shaders/core/<path>.vsh` and `.fsh`).
    pub fn new(
        id: Identifier,
        shader: Identifier,
        blend: PipelineBlend,
        depth: WorldDepth,
    ) -> Self {
        Self {
            owner: String::new(),
            id,
            fragment: shader,
            vertex_shader: None,
            defines: Vec::new(),
            vertex_format: PipelineVertexFormat::PositionTexColor,
            vertex_bindings: Vec::new(),
            state: PipelineState::new(blend, depth),
            color_targets: Vec::new(),
            invalid_color_target_index: None,
            order_independent: false,
            optional: false,
            texture: None,
            geometry_class: None,
            pack_binding: None,
        }
    }
    /// The owning mod, assigned by the host after registration.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Use a separate GLSL vertex shader program (vanilla's `withVertexShader`), such as
    /// `minecraft:core/position_tex_color` or another mod's vertex shader. Defaults to
    /// [`Self::fragment`] when omitted.
    pub fn vertex_shader(mut self, id: Identifier) -> Self {
        self.vertex_shader = Some(id);
        self
    }

    /// Define a preprocessor flag `#define NAME` for both stages of this pipeline
    /// (vanilla's `withShaderDefine(key)`). Validated at registration startup.
    pub fn define(mut self, name: impl Into<String>) -> Self {
        self.defines.push((name.into(), None));
        self
    }

    /// Define a preprocessor macro `#define NAME VALUE` for both stages of this pipeline
    /// (vanilla's `withShaderDefine(key, value)`). Validated at registration startup.
    pub fn define_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.push((name.into(), Some(value.into())));
        self
    }

    /// Validate GLSL define names and values declared on this pipeline. Called by the engine
    /// during pipeline registration freeze.
    pub fn validate_defines(&self) -> Result<(), String> {
        for (name, value) in &self.defines {
            if !is_valid_glsl_identifier(name) || name.starts_with("GL_") {
                return Err(format!(
                    "invalid shader define name {name:?}: expected a valid non-GL_ GLSL identifier"
                ));
            }
            if let Some(val) = value
                && !is_valid_glsl_define_value(val)
            {
                return Err(format!(
                    "invalid shader define value for {name}: {val:?} must be a non-empty single-line token without comments, backslashes, or '#'"
                ));
            }
        }
        Ok(())
    }

    /// Validate fixed-function pipeline state declared on this pipeline: finite depth-bias
    /// slope scale, colour-target indices `< MAX_COLOR_TARGETS` (8), and identical blend
    /// functions across all active blended colour targets (`RenderPipeline.java:385-397`).
    /// Called by the engine during pipeline registration freeze.
    pub fn validate_state(&self) -> Result<(), String> {
        if let Some(ds) = self.state.depth_stencil
            && !ds.depth_bias_slope_scale.is_finite()
        {
            return Err(format!(
                "invalid depth_bias_slope_scale {:?}: must be finite",
                ds.depth_bias_slope_scale
            ));
        }
        if let Some(index) = self.invalid_color_target_index {
            return Err(format!(
                "color target index {index} exceeds MAX_COLOR_TARGETS ({MAX_COLOR_TARGETS})"
            ));
        }
        if self.color_targets.len() > MAX_COLOR_TARGETS {
            return Err(format!(
                "color target count {} exceeds MAX_COLOR_TARGETS ({MAX_COLOR_TARGETS})",
                self.color_targets.len()
            ));
        }
        let mut last_blend: Option<BlendFunction> = None;
        for target in self.color_targets.iter().flatten() {
            if let Some(blend_fn) = target.blend.to_blend_function() {
                match last_blend {
                    None => last_blend = Some(blend_fn),
                    Some(prev) if prev != blend_fn => {
                        return Err(
                            "blend functions must be the same for all active color targets (RenderPipeline.java:392)"
                                .to_owned(),
                        );
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }

    /// Declare a vertex buffer binding at `binding` (`0..MAX_VERTEX_BINDINGS`, vanilla's
    /// `RenderPipeline.Builder.withVertexBinding(int bindingIndex, VertexFormat)`).
    ///
    /// When no binding is declared, the pipeline uses the default binding 0
    /// ([`VertexFormat::position_tex_color`]). Registration refuses `binding >= 16`, duplicate
    /// binding indices, invalid formats, duplicate attribute names across bindings, or more than
    /// `MAX_VERTEX_ELEMENTS` (16) total attributes across all bindings (`RenderPipeline.java:408`).
    pub fn vertex_binding(mut self, binding: u8, format: VertexFormat) -> Self {
        self.vertex_bindings.push((binding, format));
        self
    }

    /// The effective vertex buffer bindings sorted by binding index: [`Self::vertex_bindings`]
    /// when non-empty, otherwise `[(0, VertexFormat::position_tex_color())]`.
    pub fn effective_vertex_bindings(&self) -> Vec<(u8, VertexFormat)> {
        if self.vertex_bindings.is_empty() {
            vec![(0, VertexFormat::position_tex_color())]
        } else {
            let mut bindings = self.vertex_bindings.clone();
            bindings.sort_by_key(|(idx, _)| *idx);
            bindings
        }
    }

    /// Whether this pipeline uses the default single `PositionTexColor` vertex binding
    /// (compatible with the [`crate::WorldDrawContext::pipeline_quads`] shorthand).
    pub fn uses_default_vertex_format(&self) -> bool {
        self.vertex_bindings.is_empty()
            || (self.vertex_bindings.len() == 1
                && self.vertex_bindings[0].0 == 0
                && self.vertex_bindings[0].1 == VertexFormat::position_tex_color())
    }

    /// Validate all declared vertex buffer bindings on this pipeline: binding indices `< 16`,
    /// unique binding indices, valid per-binding [`VertexFormat`], unique GLSL attribute names
    /// across all bindings, and at most `MAX_VERTEX_ELEMENTS` (16) total attributes
    /// (`RenderPipeline.java:159,408`).
    pub fn validate_vertex_bindings(&self) -> Result<(), String> {
        let mut seen_bindings = std::collections::HashSet::new();
        let mut seen_attrs = std::collections::HashSet::new();
        let mut total_attrs = 0usize;
        for (binding, format) in &self.vertex_bindings {
            if usize::from(*binding) >= MAX_VERTEX_BINDINGS {
                return Err(format!(
                    "vertex binding index {binding} exceeds MAX_VERTEX_BINDINGS ({MAX_VERTEX_BINDINGS})"
                ));
            }
            if !seen_bindings.insert(*binding) {
                return Err(format!("duplicate vertex binding index {binding}"));
            }
            format
                .validate()
                .map_err(|e| format!("vertex binding {binding}: {e}"))?;
            total_attrs += format.attributes.len();
            if total_attrs > MAX_VERTEX_ELEMENTS {
                return Err(format!(
                    "total vertex attributes ({total_attrs}) exceeds MAX_VERTEX_ELEMENTS ({MAX_VERTEX_ELEMENTS})"
                ));
            }
            for attr in &format.attributes {
                if !seen_attrs.insert(attr.name.as_str()) {
                    return Err(format!(
                        "duplicate vertex attribute name {:?} across vertex bindings",
                        attr.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// Replace all fixed-function state fields at once ([`PipelineState`]).
    pub fn state(mut self, state: PipelineState) -> Self {
        self.state = state;
        self
    }

    /// Set how fragments join the colour target ([`PipelineState::blend`]).
    pub fn blend(mut self, blend: PipelineBlend) -> Self {
        self.state.blend = blend;
        if let Some(Some(slot0)) = self.color_targets.first_mut() {
            slot0.blend = blend;
        }
        self
    }

    /// Set depth testing behaviour from a coarse [`WorldDepth`] (`Tested` ->
    /// [`DepthCompare::GreaterEqual`], `ThroughWalls` -> [`DepthCompare::Always`]).
    pub fn depth(mut self, depth: WorldDepth) -> Self {
        let compare = DepthCompare::from_world_depth(depth);
        match &mut self.state.depth_stencil {
            Some(ds) => ds.compare = compare,
            None => self.state.depth_stencil = Some(DepthState::new(compare, false)),
        }
        self
    }

    /// Set the depth comparison function (`DepthStencilState.depthTest`).
    pub fn depth_compare(mut self, compare: DepthCompare) -> Self {
        match &mut self.state.depth_stencil {
            Some(ds) => ds.compare = compare,
            None => self.state.depth_stencil = Some(DepthState::new(compare, false)),
        }
        self
    }

    /// Set whether fragments write to the depth buffer (`DepthStencilState.writeDepth`).
    /// Default: `false`.
    pub fn depth_write(mut self, write: bool) -> Self {
        match &mut self.state.depth_stencil {
            Some(ds) => ds.write = write,
            None => {
                self.state.depth_stencil = Some(DepthState::new(DepthCompare::GreaterEqual, write))
            }
        }
        self
    }

    /// Set depth bias (`DepthStencilState.depthBiasConstant` and `depthBiasScaleFactor`).
    /// Positive values lift fragments toward the near plane in Sunrise's reversed-Z world pass
    /// (and are negated automatically on standard-Z shader-pack targets). Non-finite
    /// `slope_scale` is refused at registration.
    pub fn depth_bias(mut self, constant: i32, slope_scale: f32) -> Self {
        let ds = self
            .state
            .depth_stencil
            .get_or_insert_with(|| DepthState::new(DepthCompare::GreaterEqual, false));
        ds.depth_bias_constant = constant;
        ds.depth_bias_slope_scale = slope_scale;
        self
    }

    /// Replace the full depth-stencil state (`withDepthStencilState`). Passing `None` declares a
    /// depthless pipeline (`RenderPipeline.wantsDepthTexture() == false`) for offscreen, post, or
    /// GUI passes with no depth attachment.
    pub fn depth_stencil(mut self, depth_stencil: Option<DepthState>) -> Self {
        self.state.depth_stencil = depth_stencil;
        self
    }

    /// Set whether back faces are culled (`withCull`). Default: `false` (two-sided).
    pub fn cull(mut self, cull: bool) -> Self {
        self.state.cull = cull;
        self
    }

    /// Set which colour channels fragments write (`ColorTargetState` write mask). Default:
    /// [`ColorWrite::ALL`].
    pub fn color_write(mut self, write: ColorWrite) -> Self {
        self.state.color_write = write;
        if let Some(Some(slot0)) = self.color_targets.first_mut() {
            slot0.write = write;
        }
        self
    }

    /// Configure colour target `index` (`0..MAX_COLOR_TARGETS`, vanilla's
    /// `withColorTargetState(index, state)` / `withUnusedColorTargetState(index)`). Passing
    /// `index >= MAX_COLOR_TARGETS` (8) or differing blend functions across active targets is
    /// refused at registration (`RenderPipeline.java:385-397`).
    pub fn color_target(mut self, index: usize, state: Option<ColorTargetState>) -> Self {
        if index >= MAX_COLOR_TARGETS {
            if self.invalid_color_target_index.is_none() {
                self.invalid_color_target_index = Some(index);
            }
            return self;
        }
        if self.color_targets.len() <= index {
            self.color_targets.resize(index + 1, None);
        }
        self.color_targets[index] = state;
        if index == 0
            && let Some(ct) = state
        {
            self.state.blend = ct.blend;
            self.state.color_write = ct.write;
        }
        self
    }

    /// The effective colour target slots: [`Self::color_targets`] when explicitly configured,
    /// otherwise a single slot 0 built from [`PipelineState::blend`] and
    /// [`PipelineState::color_write`].
    pub fn effective_color_targets(&self) -> Vec<Option<ColorTargetState>> {
        if self.color_targets.is_empty() {
            vec![Some(ColorTargetState::new(
                self.state.blend,
                self.state.color_write,
            ))]
        } else {
            self.color_targets.clone()
        }
    }

    /// Set triangle fill or wireframe mode (`PolygonMode`). Default: [`PolygonMode::Fill`].
    /// [`PolygonMode::Line`] requires the device's wireframe feature.
    pub fn polygon(mut self, polygon: PolygonMode) -> Self {
        self.state.polygon = polygon;
        self
    }

    /// Set how vertices are assembled into primitives (`withPrimitiveTopology`). Default:
    /// [`PrimitiveTopology::Quads`].
    pub fn topology(mut self, topology: PrimitiveTopology) -> Self {
        self.state.topology = topology;
        self
    }

    /// Opt this pipeline in to order-independent transparency (`RenderLevelStage::OitTranslucent`,
    /// NeoForge's `registerOitPipelineSet`). The fragment shader must include
    /// `<minecraft:oit.glsl>`, declare no colour output under `OIT_ALPHA_ONLY` while calling
    /// `executeAlphaOnlyPhase(gl_FragCoord.z, alpha)`, and pass its output colour through
    /// `sampleColorForAccumulation(color)` under `OIT_ACCUMULATE`.
    pub fn order_independent(mut self) -> Self {
        self.order_independent = true;
        self
    }

    /// Let a resource load that cannot build this pipeline go on without it (NeoForge's
    /// `registerOptionalPipeline`).
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Let the shader sample `texture` via `Sampler0` or `<sunrise:pipeline_sprite.glsl>`'s
    /// `sampleSprite(uv)` (vanilla's `RenderPipeline.withSampler("Sampler0")`). A texture the
    /// load refuses ([`PipelineTextureError`]) refuses the pipeline as a missing shader does: a
    /// required one fails the load, an optional one is logged with the named cause and resolves
    /// to [`RenderPipelineError::Invalid`].
    pub fn textured(mut self, texture: PipelineTexture) -> Self {
        self.texture = Some(texture);
        self
    }

    /// Declare the world-geometry class this pipeline draws, so an active shader pack routes
    /// its draws through the matching standard `gbuffers_*` program chain before falling back
    /// to the pipeline's own shader.
    pub fn geometry_class(mut self, class: PipelineGeometryClass) -> Self {
        self.geometry_class = Some(class);
        self
    }

    /// Attach optional shader-pack integration extras (prepended program names, auxiliary
    /// depth target, frame uniforms, preprocessor defines, GLSL input bindings).
    pub fn pack_binding(mut self, binding: PackBinding) -> Self {
        self.pack_binding = Some(binding);
        self
    }

    /// The effective pack-program fallback chain in priority order: any names prepended in
    /// [`PackBinding::programs`], followed by [`PipelineGeometryClass::default_programs`],
    /// deduplicated while keeping the first occurrence. If none exist in the active pack or
    /// fail to translate, the pipeline falls back to its own shader.
    pub fn effective_pack_programs(&self) -> Vec<&str> {
        let mut chain = Vec::new();
        if let Some(binding) = &self.pack_binding {
            for name in &binding.programs {
                if !chain.contains(&name.as_str()) {
                    chain.push(name.as_str());
                }
            }
        }
        if let Some(class) = self.geometry_class {
            for &name in class.default_programs() {
                if !chain.contains(&name) {
                    chain.push(name);
                }
            }
        }
        chain
    }

    /// The effective shader-pack pass slot: [`PackBinding::slot`] when set, otherwise
    /// [`PipelineGeometryClass::default_slot`], or `None` when neither is declared.
    pub fn effective_pack_slot(&self) -> Option<PackPassSlot> {
        self.pack_binding
            .as_ref()
            .and_then(|binding| binding.slot)
            .or_else(|| self.geometry_class.map(PipelineGeometryClass::default_slot))
    }
}

/// What kind of world geometry a [`RenderPipeline`] draws, matching how Iris routes mod render
/// types through an active shader pack's standard `gbuffers_*` fallback chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline geometry classification.
pub enum PipelineGeometryClass {
    /// Opaque or cutout block-like terrain (`gbuffers_terrain` fallback chain).
    Terrain,
    /// Translucent terrain or fluid geometry (`gbuffers_water` fallback chain).
    Translucent,
    /// Entity, block-entity, or contraption geometry (`gbuffers_entities` fallback chain).
    Entity,
    /// Textured lit particle geometry (`gbuffers_particles` fallback chain).
    Particles,
    /// Sky dome or horizon geometry (`gbuffers_skybasic` fallback chain).
    Sky,
    /// Cloud geometry (`gbuffers_clouds` fallback chain).
    Clouds,
    /// Rain, snow, or weather geometry (`gbuffers_weather` fallback chain).
    Weather,
    /// Wireframe, outline, or line geometry (`gbuffers_line` fallback chain).
    Lines,
    /// First-person held item or hand geometry (`gbuffers_hand` fallback chain).
    Hand,
}

impl PipelineGeometryClass {
    /// The standard Iris/OptiFine pack-program fallback chain for this geometry class, in
    /// priority order. When none exist in the active pack or fail to translate, the pipeline
    /// falls back to its own shader.
    pub const fn default_programs(self) -> &'static [&'static str] {
        match self {
            Self::Terrain => &[
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Translucent => &[
                "gbuffers_water",
                "gbuffers_terrain",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Entity => &[
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Particles => &[
                "gbuffers_particles",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Sky => &["gbuffers_skybasic", "gbuffers_basic"],
            Self::Clouds => &["gbuffers_clouds", "gbuffers_textured", "gbuffers_basic"],
            Self::Weather => &[
                "gbuffers_weather",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
            Self::Lines => &["gbuffers_line", "gbuffers_basic"],
            Self::Hand => &[
                "gbuffers_hand",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ],
        }
    }

    /// The default shader-pack execution slot for this geometry class.
    pub const fn default_slot(self) -> PackPassSlot {
        match self {
            Self::Sky => PackPassSlot::AfterSky,
            Self::Terrain | Self::Entity | Self::Lines | Self::Hand => {
                PackPassSlot::AfterOpaqueGbuffers
            }
            Self::Translucent | Self::Particles | Self::Clouds => {
                PackPassSlot::AfterTranslucentGbuffers
            }
            Self::Weather => PackPassSlot::AfterWeather,
        }
    }
}

/// Execution slot inside an Iris/OptiFine shader pack's frame, ordered from earliest to latest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
// no wire: local client shader-pack execution slot.
pub enum PackPassSlot {
    /// Inside the pack's shadow pass, after vanilla shadow geometry and before `shadowcomp*`.
    Shadow,
    /// After the pack's sky pass (`gbuffers_skybasic`), before opaque terrain.
    AfterSky,
    /// After vanilla opaque gbuffers and before `deferred*`.
    AfterOpaqueGbuffers,
    /// After vanilla translucent gbuffers and before `composite*`.
    AfterTranslucentGbuffers,
    /// After weather gbuffers and before `composite*` and `final`.
    AfterWeather,
}

/// Optional Iris/OptiFine shader-pack extras attached to a [`RenderPipeline`]
/// ([`RenderPipeline::pack_binding`]). All identifiers are supplied by the mod; the engine
/// hardcodes none.
///
/// Startup refuses invalid GLSL identifiers, multi-line or comment-bearing define values,
/// collisions with reserved built-in pack samplers or uniforms, and cross-mod collisions on
/// pack samplers, uniforms, defines, or depth-target IDs (naming both mods).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline shader-pack binding descriptor.
pub struct PackBinding {
    /// Pack program stems prepended ahead of [`PipelineGeometryClass::default_programs`].
    pub programs: Vec<String>,
    /// Optional pass slot override; defaults to [`PipelineGeometryClass::default_slot`].
    pub slot: Option<PackPassSlot>,
    /// Preprocessor defines exposed while compiling the active shader pack when this pipeline
    /// is active: `(name, optional_value)`. `name` must be a valid GLSL identifier (not
    /// `GL_`-prefixed) and `value`, when present, must be a non-empty single-line token.
    /// Toggling active defines triggers at most one shader-pack reload.
    pub defines: Vec<(String, Option<String>)>,
    /// Optional auxiliary depth-plane target, allocated and populated only while an active
    /// pack is bound and samples one of its declared samplers.
    pub depth_target: Option<PackDepthTarget>,
    /// Per-frame pack uniforms supplied by the mod under pack-side GLSL names. If a pack
    /// declares a uniform with a different GLSL type, the engine leaves it unbound and logs
    /// once without panicking.
    pub frame_uniforms: Vec<PackUniformDecl>,
    /// Mapping from the pipeline's vertex or per-draw inputs to pack-side GLSL names.
    pub glsl_bindings: Vec<PackGlslInput>,
}

impl PackBinding {
    /// An empty shader-pack binding with no extra programs, defines, depth target, or uniforms.
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepend one pack program stem in front of the geometry class's default chain.
    pub fn program(mut self, name: impl Into<String>) -> Self {
        self.programs.push(name.into());
        self
    }

    /// Override the pack execution slot instead of using the geometry class default.
    pub fn slot(mut self, slot: PackPassSlot) -> Self {
        self.slot = Some(slot);
        self
    }

    /// Declare a preprocessor define `#define NAME` (no value) while this pipeline is active.
    pub fn define(mut self, name: impl Into<String>) -> Self {
        self.defines.push((name.into(), None));
        self
    }

    /// Declare a preprocessor define `#define NAME VALUE` while this pipeline is active.
    pub fn define_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.push((name.into(), Some(value.into())));
        self
    }

    /// Attach an auxiliary depth-plane target shared by pipelines using the same target ID.
    pub fn depth_target(mut self, target: PackDepthTarget) -> Self {
        self.depth_target = Some(target);
        self
    }

    /// Declare a per-frame pack uniform supplied by this pipeline.
    pub fn frame_uniform(mut self, uniform: PackUniformDecl) -> Self {
        self.frame_uniforms.push(uniform);
        self
    }

    /// Declare one mapping from a pipeline input to a pack-side GLSL input or uniform name.
    pub fn glsl_binding(mut self, input: PackGlslInput) -> Self {
        self.glsl_bindings.push(input);
        self
    }

    /// Validate GLSL identifier syntax and single-line define token rules on all declared
    /// entries. Called by the engine during pipeline registration freeze.
    pub fn validate_syntax(&self) -> Result<(), String> {
        for program in &self.programs {
            if !is_valid_pack_program_name(program) {
                return Err(format!(
                    "invalid pack program name {program:?}: expected ASCII [A-Za-z0-9_]+"
                ));
            }
        }
        for (name, value) in &self.defines {
            if !is_valid_glsl_identifier(name) || name.starts_with("GL_") {
                return Err(format!(
                    "invalid pack define name {name:?}: expected a valid non-GL_ GLSL identifier"
                ));
            }
            if let Some(val) = value
                && !is_valid_glsl_define_value(val)
            {
                return Err(format!(
                    "invalid pack define value for {name}: {val:?} must be a non-empty single-line token without comments, backslashes, or '#'"
                ));
            }
        }
        if let Some(target) = &self.depth_target {
            let mut seen_samplers = std::collections::HashSet::new();
            for sampler in &target.depth_samplers {
                if !is_valid_glsl_identifier(sampler) || sampler.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack depth sampler name {sampler:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if !seen_samplers.insert(sampler.as_str()) {
                    return Err(format!(
                        "duplicate pack depth sampler {sampler:?} on target {}",
                        target.id
                    ));
                }
            }
            for sampler in &target.opaque_depth_samplers {
                if !is_valid_glsl_identifier(sampler) || sampler.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack depth sampler name {sampler:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if target.depth_samplers.iter().any(|s| s == sampler) {
                    return Err(format!(
                        "pack depth sampler {sampler:?} on target {} cannot be both a full depth sampler and an opaque depth sampler",
                        target.id
                    ));
                }
                if !seen_samplers.insert(sampler.as_str()) {
                    return Err(format!(
                        "duplicate pack opaque depth sampler {sampler:?} on target {}",
                        target.id
                    ));
                }
            }
        }
        let mut current_uniforms = std::collections::HashSet::new();
        let mut previous_uniforms = std::collections::HashSet::new();
        for uniform in &self.frame_uniforms {
            if !is_valid_glsl_identifier(&uniform.name) || uniform.name.starts_with("gl_") {
                return Err(format!(
                    "invalid pack uniform name {:?}: expected a valid non-gl_ GLSL identifier",
                    uniform.name
                ));
            }
            if previous_uniforms.contains(uniform.name.as_str()) {
                return Err(format!(
                    "pack uniform {:?} collides with another uniform's previous_name",
                    uniform.name
                ));
            }
            current_uniforms.insert(uniform.name.as_str());
            if let Some(prev) = &uniform.previous_name {
                if !is_valid_glsl_identifier(prev) || prev.starts_with("gl_") {
                    return Err(format!(
                        "invalid pack previous uniform name {prev:?}: expected a valid non-gl_ GLSL identifier"
                    ));
                }
                if prev == &uniform.name {
                    return Err(format!(
                        "pack uniform {:?} cannot reuse its own name as previous_name",
                        uniform.name
                    ));
                }
                if current_uniforms.contains(prev.as_str()) {
                    return Err(format!(
                        "pack previous uniform {prev:?} collides with another uniform's current name"
                    ));
                }
                previous_uniforms.insert(prev.as_str());
            }
        }
        for input in &self.glsl_bindings {
            if !is_valid_glsl_identifier(&input.source) {
                return Err(format!(
                    "invalid pack GLSL binding source {:?}: expected a valid GLSL identifier",
                    input.source
                ));
            }
            if !is_valid_glsl_identifier(&input.glsl_name) {
                return Err(format!(
                    "invalid pack GLSL binding target {:?}: expected a valid GLSL identifier",
                    input.glsl_name
                ));
            }
        }
        Ok(())
    }
}

/// An auxiliary depth target declared by a [`PackBinding`]. Multiple pipelines owned by the
/// same mod may reference the same [`Self::id`] to share one `Depth32Float` attachment (cleared
/// to OpenGL depth `1.0`, matching `depthtex0`). Allocated and populated only while an active
/// pack is bound and samples one of its declared samplers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack auxiliary depth target descriptor.
pub struct PackDepthTarget {
    /// Mod-namespaced key identifying the shared depth attachment.
    pub id: Identifier,
    /// Sampler names exposed to pack passes for full depth after draws to this target.
    pub depth_samplers: Vec<String>,
    /// Optional sampler names exposed for an opaque-depth copy taken after
    /// [`PackPassSlot::AfterOpaqueGbuffers`] and before
    /// [`PackPassSlot::AfterTranslucentGbuffers`], populated only while sampled by the pack.
    pub opaque_depth_samplers: Vec<String>,
}

impl PackDepthTarget {
    /// Describe an auxiliary depth target keyed by `id`.
    pub fn new(id: Identifier) -> Self {
        Self {
            id,
            depth_samplers: Vec::new(),
            opaque_depth_samplers: Vec::new(),
        }
    }

    /// Expose the full depth buffer under GLSL sampler name `name`.
    pub fn depth_sampler(mut self, name: impl Into<String>) -> Self {
        self.depth_samplers.push(name.into());
        self
    }

    /// Expose the post-opaque depth snapshot under GLSL sampler name `name`.
    pub fn opaque_depth_sampler(mut self, name: impl Into<String>) -> Self {
        self.opaque_depth_samplers.push(name.into());
        self
    }
}

/// A per-frame pack uniform declared by a [`PackBinding`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack frame uniform declaration.
pub struct PackUniformDecl {
    /// GLSL uniform name read by the active shader pack.
    pub name: String,
    /// GLSL value type supplied by the mod.
    pub ty: PackUniformType,
    /// Optional previous-frame uniform alias auto-tracked by the engine.
    pub previous_name: Option<String>,
}

impl PackUniformDecl {
    /// Declare a per-frame pack uniform of type `ty`.
    pub fn new(name: impl Into<String>, ty: PackUniformType) -> Self {
        Self {
            name: name.into(),
            ty,
            previous_name: None,
        }
    }

    /// Also expose the previous frame's value under `previous_name`.
    pub fn with_previous(mut self, previous_name: impl Into<String>) -> Self {
        self.previous_name = Some(previous_name.into());
        self
    }
}

/// GLSL value type for a [`PackUniformDecl`] or [`PackGlslInput`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client shader-pack uniform GLSL type selector.
pub enum PackUniformType {
    /// `float`
    Float,
    /// `int`
    Int,
    /// `vec2`
    Vec2,
    /// `vec3`
    Vec3,
    /// `vec4`
    Vec4,
    /// `mat3`
    Mat3,
    /// `mat4`
    Mat4,
}

/// How an active shader pack binds a registered [`RenderPipeline`], reported by
/// [`crate::WorldDrawContext::pack_override_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client query value describing active shader-pack override state.
pub enum PackOverrideState {
    /// No translated shader pack is active.
    NoPack,
    /// A shader pack is active, but the pipeline was not bound to a translated pack program
    /// (either it has no [`PackBinding`] or no candidate program compiled); the mod's own shader
    /// runs instead.
    UnboundPack,
    /// The active shader pack translated and bound a pack program for this pipeline.
    Bound,
}

/// Per-frame value for a mod-declared shader-pack uniform ([`PackUniformDecl`]),
/// supplied through [`crate::WorldDrawContext::set_pack_uniform`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client shader-pack per-frame uniform value.
pub enum PackUniformValue {
    /// GLSL `float`
    Float(f32),
    /// GLSL `int`
    Int(i32),
    /// GLSL `vec2`
    Vec2([f32; 2]),
    /// GLSL `vec3`
    Vec3([f32; 3]),
    /// GLSL `vec4`
    Vec4([f32; 4]),
    /// GLSL `mat3` (column-major)
    Mat3([[f32; 3]; 3]),
    /// GLSL `mat4` (column-major)
    Mat4([[f32; 4]; 4]),
}

impl PackUniformValue {
    /// The [`PackUniformType`] matching this value's variant.
    pub const fn ty(&self) -> PackUniformType {
        match self {
            Self::Float(_) => PackUniformType::Float,
            Self::Int(_) => PackUniformType::Int,
            Self::Vec2(_) => PackUniformType::Vec2,
            Self::Vec3(_) => PackUniformType::Vec3,
            Self::Vec4(_) => PackUniformType::Vec4,
            Self::Mat3(_) => PackUniformType::Mat3,
            Self::Mat4(_) => PackUniformType::Mat4,
        }
    }
}

/// Mapping from a pipeline vertex attribute or per-draw value to a pack-side GLSL input name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client shader-pack GLSL input binding descriptor.
pub struct PackGlslInput {
    /// Pipeline-side attribute or per-draw input name.
    pub source: String,
    /// Target GLSL identifier seen by the shader pack program (e.g. `gl_Vertex` or a named
    /// attribute or uniform).
    pub glsl_name: String,
    /// GLSL value type of the bound input.
    pub ty: PackUniformType,
}

impl PackGlslInput {
    /// Map pipeline input `source` to pack-side GLSL name `glsl_name` of type `ty`.
    pub fn new(
        source: impl Into<String>,
        glsl_name: impl Into<String>,
        ty: PackUniformType,
    ) -> Self {
        Self {
            source: source.into(),
            glsl_name: glsl_name.into(),
            ty,
        }
    }
}

/// Whether `name` is a valid ASCII GLSL identifier (`[A-Za-z_][A-Za-z0-9_]*`).
fn is_valid_glsl_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether `value` is a safe, non-empty single-line GLSL `#define` value token sequence:
/// no ASCII control characters (including newlines), no leading/trailing whitespace, no
/// backslash continuations, no preprocessor `#`, and no `//`, `/*`, or `*/` comment tokens.
fn is_valid_glsl_define_value(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.bytes().any(|b| b.is_ascii_control())
        && !value.contains('\\')
        && !value.contains('#')
        && !value.contains("//")
        && !value.contains("/*")
        && !value.contains("*/")
}

/// Whether `name` is a valid pack program stem (`[A-Za-z0-9_]+`, no path separators or dots).
fn is_valid_pack_program_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A texture a pipeline's shader samples ([`RenderPipeline::textured`]): always a rectangle of
/// the atlas the host binds, never a texture of its own. Including `<sunrise:pipeline_sprite.glsl>`
/// exposes `vec4 sampleSprite(vec2 uv)`, which maps `fract(uv)` into the sprite's rectangle and
/// loads one texel by integer coordinates at mip level 0 (nearest, unfiltered, so no neighbouring
/// sprite's texel enters) in vanilla's gamma space.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline texture binding.
pub enum PipelineTexture {
    /// A sprite of the game's block or item atlas, by its texture id: `powah:block/energy_charge`
    /// is `assets/powah/textures/block/energy_charge.png` as the `minecraft:blocks` atlas holds
    /// it, `minecraft:item/diamond` an item sprite. An animated sprite animates. A sprite in
    /// neither atlas refuses the pipeline ([`PipelineTextureError::NoSuchSprite`]).
    Sprite(Identifier),
}

/// Why a pipeline's texture was refused: the named cause its refusal is logged with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline texture load.
pub enum PipelineTextureError {
    /// No sprite of that id in the block or item atlas.
    NoSuchSprite,
    /// No texture file of that id in the mod's assets or a resource pack.
    NoSuchTextureFile,
    /// The id names a file (a `textures/` path or a `.png` name), not a sprite key.
    NotASpriteKey,
    /// A texture file that isn't an image the game can decode.
    Undecodable,
}

impl std::fmt::Display for PipelineTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoSuchSprite => "no such sprite in the block or item atlas",
            Self::NoSuchTextureFile => "no such texture file",
            Self::NotASpriteKey => "the id names a file, not a sprite key",
            Self::Undecodable => "the texture file cannot be decoded",
        })
    }
}

impl std::error::Error for PipelineTextureError {}

/// The vertex data a pipeline's draws carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex format.
pub enum PipelineVertexFormat {
    /// A position, a texture coordinate and a colour: what [`PipelineQuad`] gives.
    PositionTexColor,
}

/// Required byte alignment for vertex strides (`VertexFormat.VERTEX_ALIGNMENT = 4`,
/// `VertexFormat.java:13`).
pub const VERTEX_ALIGNMENT: u32 = 4;

/// Maximum number of vertex attributes across all bindings of a [`RenderPipeline`]
/// (`VertexFormat.MAX_VERTEX_ELEMENTS = 16`, `VertexFormat.java:14`, `RenderPipeline.java:408`).
pub const MAX_VERTEX_ELEMENTS: usize = 16;

/// Maximum number of vertex buffer binding slots on a [`RenderPipeline`]
/// (`RenderPipeline.java:159`). At GPU build time, a pipeline using a binding index `>=`
/// `device.limits().max_vertex_buffers` is refused cleanly.
pub const MAX_VERTEX_BINDINGS: usize = 16;

/// Per-attribute vertex data format (the portable WebGPU/wgpu subset of vanilla's `GpuFormat`,
/// supported across Linux Vulkan, Windows DX12/Vulkan, and Apple-chip macOS Metal without
/// extensions). Each format maps 1:1 to a GLSL vertex `in` type (`float`/`vecN`/`int`/`ivecN`/
/// `uint`/`uvecN`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex attribute format.
pub enum VertexAttributeFormat {
    Float32,
    Float32x2,
    Float32x3,
    Float32x4,
    Float16x2,
    Float16x4,
    Unorm8x2,
    Unorm8x4,
    Snorm8x2,
    Snorm8x4,
    Uint8x2,
    Uint8x4,
    Sint8x2,
    Sint8x4,
    Unorm16x2,
    Unorm16x4,
    Snorm16x2,
    Snorm16x4,
    Uint16x2,
    Uint16x4,
    Sint16x2,
    Sint16x4,
    Uint32,
    Uint32x2,
    Uint32x3,
    Uint32x4,
    Sint32,
    Sint32x2,
    Sint32x3,
    Sint32x4,
    Unorm10_10_10_2,
}

impl VertexAttributeFormat {
    /// Size of one attribute value in bytes.
    pub const fn byte_size(self) -> u32 {
        match self {
            Self::Unorm8x2 | Self::Snorm8x2 | Self::Uint8x2 | Self::Sint8x2 => 2,
            Self::Float32
            | Self::Float16x2
            | Self::Unorm8x4
            | Self::Snorm8x4
            | Self::Uint8x4
            | Self::Sint8x4
            | Self::Unorm16x2
            | Self::Snorm16x2
            | Self::Uint16x2
            | Self::Sint16x2
            | Self::Uint32
            | Self::Sint32
            | Self::Unorm10_10_10_2 => 4,
            Self::Float32x2
            | Self::Float16x4
            | Self::Unorm16x4
            | Self::Snorm16x4
            | Self::Uint16x4
            | Self::Sint16x4
            | Self::Uint32x2
            | Self::Sint32x2 => 8,
            Self::Float32x3 | Self::Uint32x3 | Self::Sint32x3 => 12,
            Self::Float32x4 | Self::Uint32x4 | Self::Sint32x4 => 16,
        }
    }

    /// Required byte alignment of the attribute's offset inside its vertex buffer (`min(4,
    /// byte_size)`, satisfying both `GpuFormat.byteAlignment()` and WebGPU vertex attribute
    /// alignment).
    pub const fn alignment(self) -> u32 {
        let size = self.byte_size();
        if size < 4 { size } else { 4 }
    }

    /// The GLSL vertex `in` type for this attribute (`float`/`vecN` for float and normalized
    /// formats, `int`/`ivecN` for signed integer formats, `uint`/`uvecN` for unsigned integer
    /// formats).
    pub const fn glsl_type(self) -> &'static str {
        match self {
            Self::Float32 => "float",
            Self::Float32x2
            | Self::Float16x2
            | Self::Unorm8x2
            | Self::Snorm8x2
            | Self::Unorm16x2
            | Self::Snorm16x2 => "vec2",
            Self::Float32x3 => "vec3",
            Self::Float32x4
            | Self::Float16x4
            | Self::Unorm8x4
            | Self::Snorm8x4
            | Self::Unorm16x4
            | Self::Snorm16x4
            | Self::Unorm10_10_10_2 => "vec4",
            Self::Uint32 => "uint",
            Self::Uint8x2 | Self::Uint16x2 | Self::Uint32x2 => "uvec2",
            Self::Uint32x3 => "uvec3",
            Self::Uint8x4 | Self::Uint16x4 | Self::Uint32x4 => "uvec4",
            Self::Sint32 => "int",
            Self::Sint8x2 | Self::Sint16x2 | Self::Sint32x2 => "ivec2",
            Self::Sint32x3 => "ivec3",
            Self::Sint8x4 | Self::Sint16x4 | Self::Sint32x4 => "ivec4",
        }
    }

    /// Whether this attribute is read as an integer (`int`/`ivecN`/`uint`/`uvecN`) rather than
    /// floating-point (`float`/`vecN`).
    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            Self::Uint8x2
                | Self::Uint8x4
                | Self::Sint8x2
                | Self::Sint8x4
                | Self::Uint16x2
                | Self::Uint16x4
                | Self::Sint16x2
                | Self::Sint16x4
                | Self::Uint32
                | Self::Uint32x2
                | Self::Uint32x3
                | Self::Uint32x4
                | Self::Sint32
                | Self::Sint32x2
                | Self::Sint32x3
                | Self::Sint32x4
        )
    }
}

/// How a vertex buffer binding advances across vertices or instances (`VertexFormat.stepRate`).
///
/// WebGPU/wgpu supports per-vertex (`0`) and per-instance (`1`) step rates only; arbitrary
/// instance divisors `> 1` (`VulkanRenderPipeline.java:177-182`) are refused with
/// [`VertexFormatError::UnsupportedStepRate`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex step mode.
pub enum VertexStepMode {
    /// Advance once per vertex (`stepRate = 0`).
    #[default]
    Vertex,
    /// Advance once per instance (`stepRate = 1`).
    Instance,
}

impl VertexStepMode {
    /// Convert a vanilla numeric `stepRate` (`0` = per-vertex, `1` = per-instance), refusing
    /// divisors `> 1` with [`VertexFormatError::UnsupportedStepRate`].
    pub const fn from_step_rate(step_rate: u32) -> Result<Self, VertexFormatError> {
        match step_rate {
            0 => Ok(Self::Vertex),
            1 => Ok(Self::Instance),
            rate => Err(VertexFormatError::UnsupportedStepRate(rate)),
        }
    }
}

/// One named attribute inside a [`VertexFormat`] (`VertexFormatElement`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex attribute declaration.
pub struct VertexAttribute {
    /// GLSL vertex `in` identifier (must be a valid non-`gl_` ASCII GLSL identifier).
    pub name: String,
    /// Byte offset from the start of each vertex/instance record.
    pub offset: u32,
    /// Data encoding of this attribute.
    pub format: VertexAttributeFormat,
}

impl VertexAttribute {
    /// Describe one vertex attribute.
    pub fn new(name: impl Into<String>, offset: u32, format: VertexAttributeFormat) -> Self {
        Self {
            name: name.into(),
            offset,
            format,
        }
    }
}

/// Why a [`VertexFormat`] declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex format validation error.
pub enum VertexFormatError {
    /// Attribute name is not a valid non-`gl_` ASCII GLSL identifier.
    InvalidAttributeName(String),
    /// Two attributes in the same format share a name (`VertexFormat.java:98`).
    DuplicateAttributeName(String),
    /// Attribute byte offset is not aligned to `format.alignment()` (`VertexFormat.java:89`).
    UnalignedOffset {
        name: String,
        offset: u32,
        alignment: u32,
    },
    /// Attribute bytes extend past the format's vertex stride.
    AttributeOutOfBounds { name: String, end: u32, stride: u32 },
    /// Vertex stride is zero or not a multiple of [`VERTEX_ALIGNMENT`] (4)
    /// (`VertexFormat.java:152`).
    InvalidStride(u32),
    /// Format declares more than [`MAX_VERTEX_ELEMENTS`] (16) attributes
    /// (`VertexFormat.java:14,80`).
    TooManyAttributes(usize),
    /// Step-rate divisor `> 1` is not supported on WebGPU/wgpu (`VulkanRenderPipeline.java:177-182`).
    UnsupportedStepRate(u32),
}

impl std::fmt::Display for VertexFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAttributeName(name) => write!(
                f,
                "invalid vertex attribute name {name:?}: expected a valid non-gl_ GLSL identifier"
            ),
            Self::DuplicateAttributeName(name) => {
                write!(f, "duplicate vertex attribute name {name:?}")
            }
            Self::UnalignedOffset {
                name,
                offset,
                alignment,
            } => write!(
                f,
                "vertex attribute {name:?} offset {offset} is not aligned to {alignment} bytes"
            ),
            Self::AttributeOutOfBounds { name, end, stride } => write!(
                f,
                "vertex attribute {name:?} ends at byte {end}, exceeding stride {stride}"
            ),
            Self::InvalidStride(stride) => write!(
                f,
                "vertex stride {stride} must be non-zero and a multiple of {VERTEX_ALIGNMENT}"
            ),
            Self::TooManyAttributes(count) => write!(
                f,
                "vertex format has {count} attributes, exceeding MAX_VERTEX_ELEMENTS ({MAX_VERTEX_ELEMENTS})"
            ),
            Self::UnsupportedStepRate(rate) => write!(
                f,
                "vertex step rate {rate} is unsupported: only 0 (Vertex) and 1 (Instance) are supported"
            ),
        }
    }
}

impl std::error::Error for VertexFormatError {}

/// Memory layout of one vertex or instance buffer binding (`com.mojang.renderpearl.api.vertex.VertexFormat`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline vertex buffer format.
pub struct VertexFormat {
    /// Whether this binding advances per vertex or per instance.
    pub step_mode: VertexStepMode,
    /// Size of one record in bytes (`> 0`, multiple of [`VERTEX_ALIGNMENT`] = 4).
    pub stride: u32,
    /// Attributes read from this binding (`<= MAX_VERTEX_ELEMENTS`).
    pub attributes: Vec<VertexAttribute>,
}

impl VertexFormat {
    /// Start building a [`VertexFormat`] with the given [`VertexStepMode`].
    pub fn builder(step_mode: VertexStepMode) -> VertexFormatBuilder {
        VertexFormatBuilder {
            step_mode,
            unsupported_step_rate: None,
            attributes: Vec::new(),
            auto_offset: 0,
            explicit_stride: None,
        }
    }

    /// Start building a [`VertexFormat`] from a numeric vanilla `stepRate` (`0` = per-vertex,
    /// `1` = per-instance). Divisors `> 1` fail [`VertexFormatBuilder::build`] with
    /// [`VertexFormatError::UnsupportedStepRate`].
    pub fn builder_with_step_rate(step_rate: u32) -> VertexFormatBuilder {
        match VertexStepMode::from_step_rate(step_rate) {
            Ok(step_mode) => Self::builder(step_mode),
            Err(_) => VertexFormatBuilder {
                step_mode: VertexStepMode::Vertex,
                unsupported_step_rate: Some(step_rate),
                attributes: Vec::new(),
                auto_offset: 0,
                explicit_stride: None,
            },
        }
    }

    /// Default binding 0 matching [`PipelineVertexFormat::PositionTexColor`] (`Position: Float32x3`
    /// at 0, `UV0: Float32x2` at 12, `Color: Unorm8x4` at 20, stride 24).
    pub fn position_tex_color() -> Self {
        Self {
            step_mode: VertexStepMode::Vertex,
            stride: 24,
            attributes: vec![
                VertexAttribute::new("Position", 0, VertexAttributeFormat::Float32x3),
                VertexAttribute::new("UV0", 12, VertexAttributeFormat::Float32x2),
                VertexAttribute::new("Color", 20, VertexAttributeFormat::Unorm8x4),
            ],
        }
    }

    /// Validate all vertex-format invariants (GLSL identifiers, uniqueness, alignment, stride,
    /// attribute bounds, and attribute count).
    pub fn validate(&self) -> Result<(), VertexFormatError> {
        if self.attributes.len() > MAX_VERTEX_ELEMENTS {
            return Err(VertexFormatError::TooManyAttributes(self.attributes.len()));
        }
        if self.stride == 0 || self.stride % VERTEX_ALIGNMENT != 0 {
            return Err(VertexFormatError::InvalidStride(self.stride));
        }
        let mut seen = std::collections::HashSet::with_capacity(self.attributes.len());
        for attr in &self.attributes {
            if !is_valid_glsl_identifier(&attr.name) || attr.name.starts_with("gl_") {
                return Err(VertexFormatError::InvalidAttributeName(attr.name.clone()));
            }
            if !seen.insert(attr.name.as_str()) {
                return Err(VertexFormatError::DuplicateAttributeName(attr.name.clone()));
            }
            let alignment = attr.format.alignment();
            if attr.offset % alignment != 0 {
                return Err(VertexFormatError::UnalignedOffset {
                    name: attr.name.clone(),
                    offset: attr.offset,
                    alignment,
                });
            }
            let end = attr.offset.saturating_add(attr.format.byte_size());
            if end > self.stride {
                return Err(VertexFormatError::AttributeOutOfBounds {
                    name: attr.name.clone(),
                    end,
                    stride: self.stride,
                });
            }
        }
        Ok(())
    }
}

/// Builder for [`VertexFormat`], created via [`VertexFormat::builder`].
#[derive(Clone, Debug)]
// no wire: local client vertex format builder.
pub struct VertexFormatBuilder {
    step_mode: VertexStepMode,
    unsupported_step_rate: Option<u32>,
    attributes: Vec<VertexAttribute>,
    auto_offset: u32,
    explicit_stride: Option<u32>,
}

impl VertexFormatBuilder {
    /// Append an attribute at the current byte cursor and advance the cursor by
    /// `format.byte_size()`.
    pub fn attribute(mut self, name: impl Into<String>, format: VertexAttributeFormat) -> Self {
        let offset = self.auto_offset;
        self.attributes.push(VertexAttribute {
            name: name.into(),
            offset,
            format,
        });
        self.auto_offset = offset.saturating_add(format.byte_size());
        self
    }

    /// Append an attribute at an explicit byte `offset`, advancing the automatic cursor to at
    /// least `offset + format.byte_size()`.
    pub fn attribute_at(
        mut self,
        name: impl Into<String>,
        offset: u32,
        format: VertexAttributeFormat,
    ) -> Self {
        self.attributes.push(VertexAttribute {
            name: name.into(),
            offset,
            format,
        });
        self.auto_offset = self
            .auto_offset
            .max(offset.saturating_add(format.byte_size()));
        self
    }

    /// Set an explicit vertex stride in bytes (must be non-zero and a multiple of
    /// [`VERTEX_ALIGNMENT`] = 4). When omitted, the byte cursor after the last attribute is used.
    pub fn stride(mut self, bytes: u32) -> Self {
        self.explicit_stride = Some(bytes);
        self
    }

    /// Validate and finish the [`VertexFormat`].
    pub fn build(self) -> Result<VertexFormat, VertexFormatError> {
        if let Some(rate) = self.unsupported_step_rate {
            return Err(VertexFormatError::UnsupportedStepRate(rate));
        }
        let format = VertexFormat {
            step_mode: self.step_mode,
            stride: self.explicit_stride.unwrap_or(self.auto_offset),
            attributes: self.attributes,
        };
        format.validate()?;
        Ok(format)
    }
}

/// Index element type for indexed mesh draws (`com.mojang.renderpearl.api.pipeline.IndexType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client mesh index format.
pub enum IndexFormat {
    /// 16-bit unsigned indices (`IndexType.SHORT`, 2 bytes per index).
    Uint16,
    /// 32-bit unsigned indices (`IndexType.INT`, 4 bytes per index).
    Uint32,
}

impl IndexFormat {
    /// Size of one index in bytes (`2` for `Uint16`, `4` for `Uint32`).
    pub const fn byte_size(self) -> u32 {
        match self {
            Self::Uint16 => 2,
            Self::Uint32 => 4,
        }
    }
}

/// Initial descriptor for one vertex or instance buffer in a [`MeshDescriptor`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client retained mesh vertex buffer descriptor.
pub struct MeshBufferInit<'a> {
    /// Record stride in bytes (`> 0`, multiple of 4).
    pub stride: u32,
    /// Total GPU buffer size in bytes (non-zero, multiple of `stride`, `<= device.limits().max_buffer_size`).
    pub byte_len: u64,
    /// Optional initial bytes written at offset 0 (`len <= byte_len`, multiple of 4).
    pub initial: Option<&'a [u8]>,
}

impl<'a> MeshBufferInit<'a> {
    /// Create a buffer sized exactly to `bytes` and initialized with `bytes`.
    pub const fn from_bytes(stride: u32, bytes: &'a [u8]) -> Self {
        Self {
            stride,
            byte_len: bytes.len() as u64,
            initial: Some(bytes),
        }
    }

    /// Create an uninitialized GPU buffer of `byte_len` bytes to be filled via
    /// [`MeshRef::write_vertices`].
    pub const fn empty(stride: u32, byte_len: u64) -> Self {
        Self {
            stride,
            byte_len,
            initial: None,
        }
    }
}

/// Initial descriptor for an optional index buffer in a [`MeshDescriptor`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client retained mesh index buffer descriptor.
pub struct MeshIndexInit<'a> {
    /// Index element encoding (`Uint16` or `Uint32`).
    pub format: IndexFormat,
    /// Total GPU buffer size in bytes (non-zero, multiple of 4 and `format.byte_size()`,
    /// `<= device.limits().max_buffer_size`).
    pub byte_len: u64,
    /// Optional initial index bytes written at offset 0 (`len <= byte_len`, multiple of 4).
    pub initial: Option<&'a [u8]>,
}

impl<'a> MeshIndexInit<'a> {
    /// Create an index buffer sized exactly to `bytes` and initialized with `bytes`.
    pub const fn from_bytes(format: IndexFormat, bytes: &'a [u8]) -> Self {
        Self {
            format,
            byte_len: bytes.len() as u64,
            initial: Some(bytes),
        }
    }

    /// Create an uninitialized GPU index buffer of `byte_len` bytes to be filled via
    /// [`MeshRef::write_indices`].
    pub const fn empty(format: IndexFormat, byte_len: u64) -> Self {
        Self {
            format,
            byte_len,
            initial: None,
        }
    }
}

/// Descriptor passed to [`crate::prelude::ClientRenderClientHost::create_mesh`] and
/// [`MeshRef::replace`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client retained mesh creation descriptor.
pub struct MeshDescriptor<'a> {
    /// One or more vertex/instance buffer bindings (`1..=MAX_VERTEX_BINDINGS`).
    pub vertex_buffers: &'a [MeshBufferInit<'a>],
    /// Optional index buffer.
    pub indices: Option<MeshIndexInit<'a>>,
}

impl<'a> MeshDescriptor<'a> {
    /// Describe a non-indexed mesh with `vertex_buffers`.
    pub const fn new(vertex_buffers: &'a [MeshBufferInit<'a>]) -> Self {
        Self {
            vertex_buffers,
            indices: None,
        }
    }

    /// Attach an index buffer to this descriptor.
    pub const fn with_indices(mut self, indices: MeshIndexInit<'a>) -> Self {
        self.indices = Some(indices);
        self
    }
}

/// Why a retained GPU [`MeshRef`] operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client retained mesh operation.
pub enum MeshError {
    /// No GPU renderer is available in this context.
    Unavailable,
    /// The handle was destroyed, its owner mod was disabled, the world closed, or resources
    /// reloaded. Retired handles never become valid again.
    Stale,
    /// The calling mod does not own this mesh.
    NotOwned,
    /// Empty vertex buffer list, `> MAX_VERTEX_BINDINGS`, or binding index out of range.
    InvalidBinding,
    /// `write_indices` was called on a mesh that has no index buffer.
    NoIndexBuffer,
    /// A vertex buffer stride is zero or not a multiple of 4.
    InvalidStride,
    /// Byte offset, buffer size, or write length violates 4-byte, stride, or index alignment.
    InvalidAlignment,
    /// Buffer length is zero, or `byte_offset + data.len()` exceeds the buffer capacity.
    OutOfRange,
    /// Requested buffer size exceeds `device.limits().max_buffer_size`.
    ExceedsDeviceLimit,
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "GPU renderer is unavailable",
            Self::Stale => "mesh handle is stale or destroyed",
            Self::NotOwned => "mesh handle is owned by another mod",
            Self::InvalidBinding => "invalid vertex buffer binding",
            Self::NoIndexBuffer => "mesh has no index buffer",
            Self::InvalidStride => "vertex buffer stride must be non-zero and a multiple of 4",
            Self::InvalidAlignment => "mesh buffer size, offset, or data length is misaligned",
            Self::OutOfRange => "mesh buffer write or size is out of range",
            Self::ExceedsDeviceLimit => "mesh buffer size exceeds device max_buffer_size",
        })
    }
}

impl std::error::Error for MeshError {}

impl crate::NoBackend for MeshError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A live client context that can operate on retained GPU [`MeshRef`] handles.
pub trait MeshContext {
    /// The scoped engine access used by [`MeshRef`] methods.
    // hidden: internal trait method opening the retained-mesh backend for MeshRef.
    #[doc(hidden)]
    fn mesh_backend(&self) -> Option<MeshBackendRef<'_>> {
        None
    }
}

/// Opaque access to retained-mesh operations, not to the world or other client services.
// hidden: only the handle can open the mesh backend.
// exhaustive: private field; only constructed inside this crate.
// no wire: scoped backend borrow.
#[doc(hidden)]
pub struct MeshBackendRef<'a>(&'a dyn crate::prelude::ClientRenderClientBackend);

impl<H: crate::ClientHost + ?Sized> MeshContext for H {
    fn mesh_backend(&self) -> Option<MeshBackendRef<'_>> {
        self.backend().map(|backend| MeshBackendRef(backend.open()))
    }
}

/// Generational handle to retained GPU vertex and optional index buffers, created via
/// [`crate::prelude::ClientRenderClientHost::create_mesh`].
///
/// Drawing a [`MeshRef`] via [`crate::WorldDrawContext::draw_mesh`] records a small POD draw
/// command with zero CPU vertex copying per frame. Handles are owner-checked and retired
/// automatically on mod disable, world close, resource reload, or [`Self::destroy`].
// no wire: process-local generational handle to retained GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles.
pub struct MeshRef(pub(crate) u64, pub(crate) u32);

impl MeshRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished mesh generation");
        }
        Ok(Self(generation, slot))
    }

    /// Partially or fully update vertex buffer `binding` starting at `byte_offset` (both
    /// `byte_offset` and `data.len()` must be 4-byte aligned and in bounds).
    pub fn write_vertices(
        self,
        host: &(impl MeshContext + ?Sized),
        binding: u8,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_write_vertices(self, binding, byte_offset, data)
    }

    /// Partially or fully update this mesh's index buffer starting at `byte_offset` (both
    /// `byte_offset` and `data.len()` must be 4-byte aligned and in bounds).
    pub fn write_indices(
        self,
        host: &(impl MeshContext + ?Sized),
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_write_indices(self, byte_offset, data)
    }

    /// Reallocate backing GPU buffer(s) in place without churning this handle. If `desc` is
    /// refused, the previous buffers remain intact.
    pub fn replace(
        self,
        host: &(impl MeshContext + ?Sized),
        desc: &MeshDescriptor<'_>,
    ) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_replace(self, desc)
    }

    /// Immediately free this mesh's GPU buffers and retire the handle ([`MeshError::Stale`]
    /// thereafter).
    pub fn destroy(self, host: &(impl MeshContext + ?Sized)) -> Result<(), MeshError> {
        host.mesh_backend()
            .ok_or(MeshError::Unavailable)?
            .0
            .mesh_destroy(self)
    }
}

impl crate::__private::game::Fields for MeshRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

/// Vertex or index range for [`MeshDraw`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client mesh draw range.
pub enum DrawRange {
    /// Non-indexed draw over vertex indices `start..end`. For [`PrimitiveTopology::Quads`] and
    /// [`PrimitiveTopology::Lines`], `end - start` must be a multiple of 4 and is expanded via
    /// the engine's shared quad index buffer; for [`PrimitiveTopology::TriangleFan`], it is
    /// expanded via the engine's shared fan index buffer.
    Vertices(std::ops::Range<u32>),
    /// Indexed draw over `indices` in the bound index buffer, adding `base_vertex` to each index.
    Indexed {
        indices: std::ops::Range<u32>,
        base_vertex: i32,
    },
}

/// Draw parameters for [`crate::WorldDrawContext::draw_mesh`] and
/// [`crate::WorldDrawContext::draw_vertices`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client mesh draw parameters.
pub struct MeshDraw {
    /// Vertex or index range to draw.
    pub range: DrawRange,
    /// Instance range to draw (`0..1` by default).
    pub instances: std::ops::Range<u32>,
}

impl MeshDraw {
    /// Non-indexed draw of `range` vertices with a single instance (`0..1`).
    pub const fn vertices(range: std::ops::Range<u32>) -> Self {
        Self {
            range: DrawRange::Vertices(range),
            instances: 0..1,
        }
    }

    /// Indexed draw of `indices` with `base_vertex` and a single instance (`0..1`).
    pub const fn indexed(indices: std::ops::Range<u32>, base_vertex: i32) -> Self {
        Self {
            range: DrawRange::Indexed {
                indices,
                base_vertex,
            },
            instances: 0..1,
        }
    }

    /// Set the instance range (`0..count`).
    pub const fn with_instances(mut self, instances: std::ops::Range<u32>) -> Self {
        self.instances = instances;
        self
    }
}

/// Per-draw set-1 resource bindings passed to [`crate::WorldDrawContext::draw_mesh`] and
/// [`crate::WorldDrawContext::draw_vertices`] (extended with custom uniform buffers, samplers,
/// and texel buffers in `#5428`). Pass `&DrawBindings::EMPTY` for pipelines with no custom
/// bind-group entries.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
// no wire: local client per-draw resource bindings.
pub struct DrawBindings<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

impl DrawBindings<'_> {
    /// Empty bind-group bindings for pipelines that use only vanilla set-0 bindings.
    pub const EMPTY: Self = Self {
        _marker: std::marker::PhantomData,
    };
}

/// Maximum number of colour targets a [`RenderPipeline`] can declare
/// (`ColorTargetState.MAX_COLOR_TARGETS`).
pub const MAX_COLOR_TARGETS: usize = 8;

/// Depth comparison operator (`com.mojang.renderpearl.api.pipeline.CompareOp`).
///
/// Sunrise uses reversed-Z in world passes (`1.0` near, `0.0` far, matching vanilla 26.3's
/// `DepthStencilState.DEFAULT = GREATER_THAN_OR_EQUAL`). On standard-Z shader-pack targets
/// (`0.0` near, `1.0` far), the renderer flips `Less*` ↔ `Greater*` automatically via
/// [`Self::flip_for_standard_z`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline depth comparison operator.
pub enum DepthCompare {
    /// Always pass (`CompareOp.ALWAYS_PASS`).
    Always,
    /// Pass when incoming depth `<` stored depth (`CompareOp.LESS_THAN`).
    Less,
    /// Pass when incoming depth `<=` stored depth (`CompareOp.LESS_THAN_OR_EQUAL`).
    LessEqual,
    /// Pass when incoming depth `==` stored depth (`CompareOp.EQUAL`).
    Equal,
    /// Pass when incoming depth `!=` stored depth (`CompareOp.NOT_EQUAL`).
    NotEqual,
    /// Pass when incoming depth `>=` stored depth (`CompareOp.GREATER_THAN_OR_EQUAL`, default
    /// for reversed-Z world passes).
    GreaterEqual,
    /// Pass when incoming depth `>` stored depth (`CompareOp.GREATER_THAN`).
    Greater,
    /// Never pass (`CompareOp.NEVER_PASS`).
    Never,
}

impl DepthCompare {
    /// Map a coarse [`WorldDepth`] to Sunrise's reversed-Z [`DepthCompare`].
    pub const fn from_world_depth(depth: WorldDepth) -> Self {
        match depth {
            WorldDepth::ThroughWalls => Self::Always,
            WorldDepth::Tested => Self::GreaterEqual,
            _ => Self::GreaterEqual,
        }
    }

    /// Flip `Less*` ↔ `Greater*` when targeting a standard-Z (`0.0` near, `1.0` far) render
    /// target such as a translated shader pack's `depthtex0`.
    pub const fn flip_for_standard_z(self) -> Self {
        match self {
            Self::Less => Self::Greater,
            Self::LessEqual => Self::GreaterEqual,
            Self::GreaterEqual => Self::LessEqual,
            Self::Greater => Self::Less,
            Self::Always => Self::Always,
            Self::Equal => Self::Equal,
            Self::NotEqual => Self::NotEqual,
            Self::Never => Self::Never,
        }
    }
}

/// Canonical bit representation of `f32` normalizing `-0.0` to `+0.0` so `DepthState` and
/// `PipelineState` satisfy `Eq` and `Hash`.
const fn canonical_f32_bits(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

/// Depth test, depth write, and depth bias state
/// (`com.mojang.renderpearl.api.pipeline.DepthStencilState`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
// no wire: local client render-pipeline depth-stencil state.
pub struct DepthState {
    /// Depth comparison function in Sunrise's reversed-Z world space.
    pub compare: DepthCompare,
    /// Whether fragments write to the depth buffer (`DepthStencilState.writeDepth`).
    pub write: bool,
    /// Constant depth bias in depth-buffer units (`wgpu::DepthBiasState::constant`,
    /// `DepthStencilState.depthBiasConstant`). Default: `0`.
    pub depth_bias_constant: i32,
    /// Slope-scaled depth bias factor (`wgpu::DepthBiasState::slope_scale`,
    /// `DepthStencilState.depthBiasScaleFactor`). Must be finite (refused at registration
    /// otherwise). Default: `0.0`.
    pub depth_bias_slope_scale: f32,
}

impl DepthState {
    /// Vanilla's `DepthStencilState.DEFAULT`: `GreaterEqual`, depth write enabled, zero bias.
    pub const DEFAULT: Self = Self {
        compare: DepthCompare::GreaterEqual,
        write: true,
        depth_bias_constant: 0,
        depth_bias_slope_scale: 0.0,
    };

    /// Construct a depth state with zero depth bias.
    pub const fn new(compare: DepthCompare, write: bool) -> Self {
        Self {
            compare,
            write,
            depth_bias_constant: 0,
            depth_bias_slope_scale: 0.0,
        }
    }

    /// Set constant and slope-scaled depth bias.
    pub const fn depth_bias(mut self, constant: i32, slope_scale: f32) -> Self {
        self.depth_bias_constant = constant;
        self.depth_bias_slope_scale = slope_scale;
        self
    }
}

impl PartialEq for DepthState {
    fn eq(&self, other: &Self) -> bool {
        self.compare == other.compare
            && self.write == other.write
            && self.depth_bias_constant == other.depth_bias_constant
            && canonical_f32_bits(self.depth_bias_slope_scale)
                == canonical_f32_bits(other.depth_bias_slope_scale)
    }
}

impl Eq for DepthState {}

impl std::hash::Hash for DepthState {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.compare.hash(state);
        self.write.hash(state);
        self.depth_bias_constant.hash(state);
        canonical_f32_bits(self.depth_bias_slope_scale).hash(state);
    }
}

/// Blending factor (`com.mojang.renderpearl.api.pipeline.BlendFactor`).
///
/// Note: WebGPU/wgpu maps both `ConstantColor`/`ConstantAlpha` to its `Constant` factor (and
/// `OneMinusConstantColor`/`OneMinusConstantAlpha` to `OneMinusConstant`), selecting RGB for the
/// colour equation and A for the alpha equation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend factor.
pub enum BlendFactor {
    ConstantAlpha,
    ConstantColor,
    DstAlpha,
    DstColor,
    One,
    OneMinusConstantAlpha,
    OneMinusConstantColor,
    OneMinusDstAlpha,
    OneMinusDstColor,
    OneMinusSrcAlpha,
    OneMinusSrcColor,
    SrcAlpha,
    SrcAlphaSaturate,
    SrcColor,
    Zero,
}

/// Blending operation (`com.mojang.renderpearl.api.pipeline.BlendOp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend operation.
pub enum BlendOp {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

/// One channel's blend equation (`com.mojang.renderpearl.api.pipeline.BlendEquation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend equation.
pub struct BlendEquation {
    pub src: BlendFactor,
    pub dst: BlendFactor,
    pub op: BlendOp,
}

impl BlendEquation {
    /// Construct a blend equation with explicit source factor, destination factor, and op.
    pub const fn new(src: BlendFactor, dst: BlendFactor, op: BlendOp) -> Self {
        Self { src, dst, op }
    }

    /// Construct an additive (`BlendOp::Add`) blend equation.
    pub const fn add(src: BlendFactor, dst: BlendFactor) -> Self {
        Self {
            src,
            dst,
            op: BlendOp::Add,
        }
    }
}

/// Separate colour and alpha blend equations (`com.mojang.renderpearl.api.pipeline.BlendFunction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline custom blend function.
pub struct BlendFunction {
    pub color: BlendEquation,
    pub alpha: BlendEquation,
}

impl BlendFunction {
    /// `BlendFunction.LIGHTNING`: `SrcAlpha`, `One` (both colour and alpha).
    pub const LIGHTNING: Self = Self::symmetric(BlendFactor::SrcAlpha, BlendFactor::One);
    /// `BlendFunction.GLINT`: colour `(SrcColor, One)`, alpha `(Zero, One)`.
    pub const GLINT: Self = Self::separate(
        BlendFactor::SrcColor,
        BlendFactor::One,
        BlendFactor::Zero,
        BlendFactor::One,
    );
    /// `BlendFunction.OVERLAY`: colour `(SrcAlpha, One)`, alpha `(One, Zero)`.
    pub const OVERLAY: Self = Self::separate(
        BlendFactor::SrcAlpha,
        BlendFactor::One,
        BlendFactor::One,
        BlendFactor::Zero,
    );
    /// `BlendFunction.TRANSLUCENT`: colour `(SrcAlpha, OneMinusSrcAlpha)`, alpha `(One, OneMinusSrcAlpha)`.
    pub const TRANSLUCENT: Self = Self::separate(
        BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha,
        BlendFactor::One,
        BlendFactor::OneMinusSrcAlpha,
    );
    /// `BlendFunction.TRANSLUCENT_PREMULTIPLIED_ALPHA`: `(One, OneMinusSrcAlpha)` on colour and alpha.
    pub const TRANSLUCENT_PREMULTIPLIED_ALPHA: Self = Self::separate(
        BlendFactor::One,
        BlendFactor::OneMinusSrcAlpha,
        BlendFactor::One,
        BlendFactor::OneMinusSrcAlpha,
    );
    /// `BlendFunction.ADDITIVE`: `(One, One)` on colour and alpha.
    pub const ADDITIVE: Self = Self::symmetric(BlendFactor::One, BlendFactor::One);
    /// `BlendFunction.ENTITY_OUTLINE_BLIT`: colour `(SrcAlpha, OneMinusSrcAlpha)`, alpha `(Zero, One)`.
    pub const ENTITY_OUTLINE_BLIT: Self = Self::separate(
        BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha,
        BlendFactor::Zero,
        BlendFactor::One,
    );
    /// `BlendFunction.INVERT`: colour `(OneMinusDstColor, OneMinusSrcColor)`, alpha `(One, Zero)`.
    pub const INVERT: Self = Self::separate(
        BlendFactor::OneMinusDstColor,
        BlendFactor::OneMinusSrcColor,
        BlendFactor::One,
        BlendFactor::Zero,
    );
    /// `BlendFunction.MAX`: `(One, One, BlendOp::Max)` on colour and alpha.
    pub const MAX: Self = Self::symmetric_with_op(BlendFactor::One, BlendFactor::One, BlendOp::Max);

    /// Construct from explicit colour and alpha equations.
    pub const fn new(color: BlendEquation, alpha: BlendEquation) -> Self {
        Self { color, alpha }
    }

    /// Construct with separate colour and alpha additive equations.
    pub const fn separate(
        src_color: BlendFactor,
        dst_color: BlendFactor,
        src_alpha: BlendFactor,
        dst_alpha: BlendFactor,
    ) -> Self {
        Self {
            color: BlendEquation::add(src_color, dst_color),
            alpha: BlendEquation::add(src_alpha, dst_alpha),
        }
    }

    /// Construct with separate colour and alpha equations and explicit operations.
    pub const fn separate_with_ops(
        src_color: BlendFactor,
        dst_color: BlendFactor,
        color_op: BlendOp,
        src_alpha: BlendFactor,
        dst_alpha: BlendFactor,
        alpha_op: BlendOp,
    ) -> Self {
        Self {
            color: BlendEquation::new(src_color, dst_color, color_op),
            alpha: BlendEquation::new(src_alpha, dst_alpha, alpha_op),
        }
    }

    /// Construct with identical additive colour and alpha equations.
    pub const fn symmetric(src: BlendFactor, dst: BlendFactor) -> Self {
        let eq = BlendEquation::add(src, dst);
        Self {
            color: eq,
            alpha: eq,
        }
    }

    /// Construct with identical colour and alpha equations and an explicit operation.
    pub const fn symmetric_with_op(src: BlendFactor, dst: BlendFactor, op: BlendOp) -> Self {
        let eq = BlendEquation::new(src, dst, op);
        Self {
            color: eq,
            alpha: eq,
        }
    }
}

/// How a pipeline's fragments join the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline blend mode.
pub enum PipelineBlend {
    /// Solid colour written directly without alpha blending (vanilla's no-blend target state).
    Opaque,
    /// Laid over the frame by the fragment's alpha, as [`crate::WorldQuad`] is
    /// ([`BlendFunction::TRANSLUCENT`]).
    Translucent,
    /// Added to the frame, scaled by the fragment's alpha (colour `SrcAlpha + One`, alpha
    /// `Zero + One`).
    Additive,
    /// Explicit colour and alpha blend equations (`BlendFunction.java`).
    Custom(BlendFunction),
}

impl PipelineBlend {
    /// Convert this blend mode into its underlying [`BlendFunction`], or `None` for
    /// [`Self::Opaque`].
    pub const fn to_blend_function(self) -> Option<BlendFunction> {
        match self {
            Self::Opaque => None,
            Self::Translucent => Some(BlendFunction::TRANSLUCENT),
            Self::Additive => Some(BlendFunction::separate(
                BlendFactor::SrcAlpha,
                BlendFactor::One,
                BlendFactor::Zero,
                BlendFactor::One,
            )),
            Self::Custom(func) => Some(func),
        }
    }
}

/// Per-target blending and channel write mask (`ColorTargetState.java`). The target texture
/// format comes from the active render pass rather than the pipeline descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline colour target state.
pub struct ColorTargetState {
    /// How fragments join this colour target.
    pub blend: PipelineBlend,
    /// Which colour channels fragments write to this target.
    pub write: ColorWrite,
}

impl ColorTargetState {
    /// `ColorTargetState.DEFAULT`: opaque (no blend), write all RGBA channels.
    pub const DEFAULT: Self = Self {
        blend: PipelineBlend::Opaque,
        write: ColorWrite::ALL,
    };

    /// Construct a colour target state with explicit blend mode and write mask.
    pub const fn new(blend: PipelineBlend, write: ColorWrite) -> Self {
        Self { blend, write }
    }

    /// Construct a colour target state with `blend` and [`ColorWrite::ALL`].
    pub const fn blended(blend: PipelineBlend) -> Self {
        Self {
            blend,
            write: ColorWrite::ALL,
        }
    }
}

impl Default for ColorTargetState {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How vertices are assembled into primitives (`com.mojang.renderpearl.api.pipeline.PrimitiveTopology`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline primitive topology.
pub enum PrimitiveTopology {
    /// Vanilla's quad-expanded wide lines (`PrimitiveTopology.LINES`: 4 vertices -> 6 indices
    /// per line quad via the engine's shared quad index buffer).
    Lines,
    /// 1-pixel hardware line list (`PrimitiveTopology.DEBUG_LINES`).
    DebugLines,
    /// 1-pixel hardware line strip (`PrimitiveTopology.DEBUG_LINE_STRIP`).
    DebugLineStrip,
    /// Individual points (`PrimitiveTopology.POINTS`).
    Points,
    /// Independent triangles (`PrimitiveTopology.TRIANGLES`).
    Triangles,
    /// Connected triangle strip (`PrimitiveTopology.TRIANGLE_STRIP`).
    TriangleStrip,
    /// Triangle fan (`PrimitiveTopology.TRIANGLE_FAN`, e.g. `SkyRenderer.java:88,95,338`),
    /// emulated on WebGPU/wgpu via an engine-generated fan index buffer (`0, i, i + 1` for
    /// `i` in `1..vertex_count - 1`).
    TriangleFan,
    /// Quads expanded 4 vertices -> 6 indices via the engine's shared quad index buffer
    /// (`PrimitiveTopology.QUADS`). Default for [`RenderPipeline::new`] / `pipeline_quads`.
    #[default]
    Quads,
}

impl PrimitiveTopology {
    /// Vertices per primitive (`PrimitiveTopology.primitiveLength`).
    pub const fn primitive_length(self) -> u32 {
        match self {
            Self::Lines | Self::DebugLines | Self::DebugLineStrip => 2,
            Self::Points => 1,
            Self::Triangles | Self::TriangleStrip | Self::TriangleFan => 3,
            Self::Quads => 4,
        }
    }

    /// Vertex advance per additional primitive (`PrimitiveTopology.primitiveStride`).
    pub const fn primitive_stride(self) -> u32 {
        match self {
            Self::Lines | Self::DebugLines => 2,
            Self::DebugLineStrip | Self::Points | Self::TriangleStrip | Self::TriangleFan => 1,
            Self::Triangles => 3,
            Self::Quads => 4,
        }
    }

    /// Whether consecutive primitives share vertices (`PrimitiveTopology.connectedPrimitives`).
    pub const fn connected_primitives(self) -> bool {
        matches!(
            self,
            Self::DebugLineStrip | Self::TriangleStrip | Self::TriangleFan
        )
    }

    /// Number of indices produced for `vertex_count` vertices when drawing with engine-generated
    /// index buffers (`Quads` and `Lines`: `(vertex_count / 4) * 6`; `TriangleFan`:
    /// `(vertex_count - 2) * 3`; all native topologies: `vertex_count`).
    pub const fn index_count(self, vertex_count: u32) -> u32 {
        match self {
            Self::Lines | Self::Quads => (vertex_count / 4) * 6,
            Self::TriangleFan => vertex_count.saturating_sub(2).saturating_mul(3),
            Self::DebugLines
            | Self::DebugLineStrip
            | Self::Points
            | Self::Triangles
            | Self::TriangleStrip => vertex_count,
        }
    }
}

/// Which colour channels a pipeline writes (`ColorTargetState` write mask).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline colour write mask.
pub struct ColorWrite {
    /// Write red, green and blue channels.
    pub color: bool,
    /// Write the alpha channel.
    pub alpha: bool,
}

impl ColorWrite {
    /// Write RGB and alpha (the default).
    pub const ALL: Self = Self {
        color: true,
        alpha: true,
    };
    /// Write only RGB, leaving alpha unchanged.
    pub const COLOR: Self = Self {
        color: true,
        alpha: false,
    };
    /// Write only alpha, leaving RGB unchanged.
    pub const ALPHA: Self = Self {
        color: false,
        alpha: true,
    };
    /// Write no colour channels (depth-only pass).
    pub const NONE: Self = Self {
        color: false,
        alpha: false,
    };

    /// Construct a write mask with explicit `color` (RGB) and `alpha` toggles.
    pub const fn new(color: bool, alpha: bool) -> Self {
        Self { color, alpha }
    }
}

impl Default for ColorWrite {
    fn default() -> Self {
        Self::ALL
    }
}

/// How polygons are rasterized (vanilla's `PolygonMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline polygon mode.
pub enum PolygonMode {
    /// Fill triangles (the default).
    #[default]
    Fill,
    /// Wireframe edges; requires the device's wireframe feature, so pipelines or modifier
    /// variants using `Line` are refused when the GPU lacks it.
    Line,
}

/// Fixed-function rasterization, depth, blend, colour-write, and topology state of a
/// [`RenderPipeline`] (vanilla's `RenderPipeline` state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client render-pipeline fixed state.
pub struct PipelineState {
    /// How fragments join colour target 0 (and default MRT targets).
    pub blend: PipelineBlend,
    /// Depth test, depth write, and depth bias state (`DepthStencilState.java`). `None`
    /// indicates a depthless pass (`RenderPipeline.wantsDepthTexture() == false`).
    pub depth_stencil: Option<DepthState>,
    /// Whether back faces are culled (`withCull`). Default: `false` (two-sided).
    pub cull: bool,
    /// Which colour channels fragments write (`ColorTargetState` write mask). Default:
    /// [`ColorWrite::ALL`].
    pub color_write: ColorWrite,
    /// Triangle fill or wireframe (`PolygonMode`). Default: [`PolygonMode::Fill`].
    pub polygon: PolygonMode,
    /// How vertices are assembled into primitives (`PrimitiveTopology.java`). Default:
    /// [`PrimitiveTopology::Quads`].
    pub topology: PrimitiveTopology,
}

impl PipelineState {
    /// Default state for `blend` and `depth` (`depth_stencil: Some(DepthState::new(compare,
    /// false))`, `cull: false`, `color_write: ColorWrite::ALL`, `polygon: PolygonMode::Fill`,
    /// `topology: PrimitiveTopology::Quads`).
    pub const fn new(blend: PipelineBlend, depth: WorldDepth) -> Self {
        Self {
            blend,
            depth_stencil: Some(DepthState::new(
                DepthCompare::from_world_depth(depth),
                false,
            )),
            cull: false,
            color_write: ColorWrite::ALL,
            polygon: PolygonMode::Fill,
            topology: PrimitiveTopology::Quads,
        }
    }
}

/// Data-only state overrides registered by `Hooks::register_pipeline_modifiers` (NeoForge's
/// `RegisterPipelineModifiersEvent` / `PipelineModifier`).
///
/// Every field is optional: `None` leaves the underlying pipeline's value unchanged. Because a
/// modifier is pure data, applying it is idempotent by construction and the renderer caches each
/// `(pipeline, modifier stack)` variant lazily.
#[derive(Clone, Debug)]
#[non_exhaustive]
// no wire: local client pipeline-modifier registration descriptor.
pub struct PipelineModifier {
    owner: String,
    /// The modifier's own name (`mymod:xray`), unique among registered modifiers.
    pub id: Identifier,
    /// Override [`PipelineState::blend`]. Ignored for OIT variants where OIT supplies blending.
    pub blend: Option<PipelineBlend>,
    /// Override [`DepthState::compare`] on depth-attached pipelines (and `WorldDepth` on plain
    /// world primitives).
    pub depth_compare: Option<DepthCompare>,
    /// Override [`DepthState::write`] on depth-attached pipelines. Ignored for OIT variants
    /// where depth is read-only.
    pub depth_write: Option<bool>,
    /// Override `(depth_bias_constant, depth_bias_slope_scale)` on depth-attached pipelines.
    /// Non-finite slope scale is refused at registration.
    pub depth_bias: Option<(i32, f32)>,
    /// Override [`PipelineState::cull`].
    pub cull: Option<bool>,
    /// Override [`PipelineState::color_write`].
    pub color_write: Option<ColorWrite>,
    /// Override [`PipelineState::polygon`].
    pub polygon: Option<PolygonMode>,
    /// Override [`PipelineState::topology`].
    pub topology: Option<PrimitiveTopology>,
}

impl PartialEq for PipelineModifier {
    fn eq(&self, other: &Self) -> bool {
        let bias_eq = match (self.depth_bias, other.depth_bias) {
            (None, None) => true,
            (Some((ac, as_)), Some((bc, bs))) => {
                ac == bc && canonical_f32_bits(as_) == canonical_f32_bits(bs)
            }
            _ => false,
        };
        self.owner == other.owner
            && self.id == other.id
            && self.blend == other.blend
            && self.depth_compare == other.depth_compare
            && self.depth_write == other.depth_write
            && bias_eq
            && self.cull == other.cull
            && self.color_write == other.color_write
            && self.polygon == other.polygon
            && self.topology == other.topology
    }
}

impl Eq for PipelineModifier {}

impl PipelineModifier {
    /// Describe a modifier with no overrides set.
    pub fn new(id: Identifier) -> Self {
        Self {
            owner: String::new(),
            id,
            blend: None,
            depth_compare: None,
            depth_write: None,
            depth_bias: None,
            cull: None,
            color_write: None,
            polygon: None,
            topology: None,
        }
    }

    /// The owning mod, assigned by the host after registration.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Override [`PipelineState::blend`].
    pub fn blend(mut self, blend: PipelineBlend) -> Self {
        self.blend = Some(blend);
        self
    }

    /// Override depth testing from a coarse [`WorldDepth`] (`Tested` ->
    /// [`DepthCompare::GreaterEqual`], `ThroughWalls` -> [`DepthCompare::Always`]).
    pub fn depth(mut self, depth: WorldDepth) -> Self {
        self.depth_compare = Some(DepthCompare::from_world_depth(depth));
        self
    }

    /// Override [`DepthState::compare`].
    pub fn depth_compare(mut self, compare: DepthCompare) -> Self {
        self.depth_compare = Some(compare);
        self
    }

    /// Override [`DepthState::write`].
    pub fn depth_write(mut self, write: bool) -> Self {
        self.depth_write = Some(write);
        self
    }

    /// Override [`DepthState::depth_bias_constant`] and [`DepthState::depth_bias_slope_scale`].
    /// Non-finite `slope_scale` is refused at registration.
    pub fn depth_bias(mut self, constant: i32, slope_scale: f32) -> Self {
        self.depth_bias = Some((constant, slope_scale));
        self
    }

    /// Override [`PipelineState::cull`].
    pub fn cull(mut self, cull: bool) -> Self {
        self.cull = Some(cull);
        self
    }

    /// Override [`PipelineState::color_write`].
    pub fn color_write(mut self, write: ColorWrite) -> Self {
        self.color_write = Some(write);
        self
    }

    /// Override [`PipelineState::polygon`].
    pub fn polygon(mut self, polygon: PolygonMode) -> Self {
        self.polygon = Some(polygon);
        self
    }

    /// Override [`PipelineState::topology`].
    pub fn topology(mut self, topology: PrimitiveTopology) -> Self {
        self.topology = Some(topology);
        self
    }

    /// Validate modifier fields at registration freeze (e.g. finite `depth_bias` slope scale).
    pub fn validate(&self) -> Result<(), String> {
        if let Some((_, slope_scale)) = self.depth_bias
            && !slope_scale.is_finite()
        {
            return Err(format!(
                "invalid depth_bias slope_scale {slope_scale:?}: must be finite"
            ));
        }
        Ok(())
    }

    /// Apply this modifier's non-`None` overrides to `state`. Depth overrides (`depth_compare`,
    /// `depth_write`, `depth_bias`) apply only when `state.depth_stencil` is `Some`.
    pub const fn apply(&self, mut state: PipelineState) -> PipelineState {
        if let Some(blend) = self.blend {
            state.blend = blend;
        }
        if let Some(mut ds) = state.depth_stencil {
            if let Some(compare) = self.depth_compare {
                ds.compare = compare;
            }
            if let Some(depth_write) = self.depth_write {
                ds.write = depth_write;
            }
            if let Some((constant, slope_scale)) = self.depth_bias {
                ds.depth_bias_constant = constant;
                ds.depth_bias_slope_scale = slope_scale;
            }
            state.depth_stencil = Some(ds);
        }
        if let Some(cull) = self.cull {
            state.cull = cull;
        }
        if let Some(color_write) = self.color_write {
            state.color_write = color_write;
        }
        if let Some(polygon) = self.polygon {
            state.polygon = polygon;
        }
        if let Some(topology) = self.topology {
            state.topology = topology;
        }
        state
    }

    /// Whether this modifier overrides no state fields.
    pub const fn is_noop(&self) -> bool {
        self.blend.is_none()
            && self.depth_compare.is_none()
            && self.depth_write.is_none()
            && self.depth_bias.is_none()
            && self.cull.is_none()
            && self.color_write.is_none()
            && self.polygon.is_none()
            && self.topology.is_none()
    }
}

/// A registered pipeline modifier, resolved through
/// [`crate::RenderAssets::pipeline_modifier`]. Retain it between frames; modifiers are static
/// across resource reloads, though re-resolving in `client_resources_reloaded` is valid.
// no wire: process-local handle to a registered client pipeline modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private field; only the engine resolves modifier handles
pub struct PipelineModifierRef(pub(crate) u32);

impl PipelineModifierRef {
    pub(crate) fn from_slot(slot: u32) -> Self {
        Self(slot)
    }
}

/// A registered pipeline as the current resources built it, resolved through
/// [`crate::RenderAssets::render_pipeline`]. Retain it between frames; resolve it again after a
/// successful resource reload.
// no wire: process-local handle to a client render pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves pipeline handles
pub struct RenderPipelineRef(pub(crate) u64, pub(crate) u32);

impl RenderPipelineRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished pipeline generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a pipeline could not be resolved or drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline lookup or draw.
pub enum RenderPipelineError {
    /// No mod registered a pipeline of this id.
    Unknown,
    /// The pipeline is registered, but the current resources give it no usable shader. The log
    /// names the pipeline and the reason.
    Invalid,
    /// A successful resource reload superseded the handle's generation.
    Stale,
}

/// A two-sided quad drawn by a mod pipeline. Corners run top left, bottom left, bottom right,
/// top right as seen from the front.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client render-pipeline quad primitive.
pub struct PipelineQuad {
    /// Four world-space corners in blocks, before camera subtraction.
    pub corners: [[f64; 3]; 4],
    /// Each corner's texture coordinate, handed to the shader as given.
    pub uv: [[f32; 2]; 4],
    /// Packed 0xAARRGGBB colour in vanilla's gamma space, passed straight to the GLSL `Color`
    /// attribute.
    pub color: u32,
}
impl PipelineQuad {
    /// Describe a quad; the draw context validates it when submitting the batch.
    pub fn new(corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], color: u32) -> Self {
        Self { corners, uv, color }
    }
    /// A quad whose texture coordinates span 0 to 1: (0, 0) at the top left corner and (1, 1)
    /// at the bottom right.
    pub fn spanning(corners: [[f64; 3]; 4], color: u32) -> Self {
        Self::new(
            corners,
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            color,
        )
    }
}

/// Why a pipeline draw call was refused; nothing from that call was appended.
///
/// Individual quads with non-finite coordinates or UVs are skipped without refusing valid quads
/// in the same slice, and quad storage grows dynamically into the GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local client render-pipeline draw submission.
pub enum PipelineDrawError {
    /// The handle is stale, or names no usable pipeline.
    Pipeline(RenderPipelineError),
    /// The pipeline's effective [`PrimitiveTopology`] does not match the draw call (for example,
    /// submitting a non-[`PrimitiveTopology::Quads`] pipeline to `pipeline_quads`).
    TopologyMismatch,
    /// The retained [`MeshRef`] handle was stale, not owned, or unavailable.
    Mesh(MeshError),
    /// The provided vertex buffer count or per-binding stride does not match the pipeline's
    /// declared [`VertexFormat`] bindings.
    VertexFormatMismatch,
    /// [`DrawRange::Indexed`] was requested without an index buffer.
    MissingIndexBuffer,
    /// The vertex, index, or instance range is inverted, out of bounds, or (for `Quads`/`Lines`
    /// non-indexed draws) not a multiple of 4 vertices.
    InvalidRange,
    /// Transient vertex or index bytes are not aligned to 4 bytes, the binding stride, or the
    /// index element size.
    InvalidAlignment,
}

impl crate::RegisterRenderPipelinesEvent<'_> {
    /// Register one render pipeline without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Any valid identifier is accepted
    /// here; duplicate IDs and vanilla collisions are refused when the engine freezes the
    /// registrations at startup.
    pub fn register(&mut self, mut entry: RenderPipeline) {
        entry.owner = self.mod_id.to_owned();
        self.pipelines.push(entry);
    }
}

impl crate::RegisterPipelineModifiersEvent<'_> {
    /// Register one pipeline modifier without exposing earlier registrations.
    ///
    /// The entry is attributed to this listener immediately. Duplicate modifier IDs fail
    /// client startup naming both mods when the engine freezes registrations.
    pub fn register(&mut self, mut entry: PipelineModifier) {
        entry.owner = self.mod_id.to_owned();
        self.modifiers.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(id: &str) -> RenderPipeline {
        let id = Identifier::parse(id).unwrap();
        RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
    }
    fn ids(pipelines: &[RenderPipeline]) -> Vec<String> {
        pipelines.iter().map(|p| p.id.to_string()).collect()
    }

    #[test]
    fn stamps_owner_on_new_pipelines_across_namespaces_and_preserves_duplicates_for_freeze() {
        let mut pipelines = Vec::new();
        let mut event = crate::RegisterRenderPipelinesEvent {
            mod_id: "first_mod",
            pipelines: &mut pipelines,
        };
        event.register(pipeline("other:glow"));
        event.mod_id = "owning_mod";
        for id in ["minecraft:glow", "other:beam", "other:glow", "mine:beam"] {
            event.register(pipeline(id));
        }
        assert_eq!(
            ids(&pipelines),
            [
                "other:glow".to_owned(),
                "minecraft:glow".to_owned(),
                "other:beam".to_owned(),
                "other:glow".to_owned(),
                "mine:beam".to_owned(),
            ]
        );
        assert_eq!(pipelines[0].owner(), "first_mod");
        assert_eq!(pipelines[1].owner(), "owning_mod");
        assert_eq!(pipelines[2].owner(), "owning_mod");
        assert_eq!(pipelines[3].owner(), "owning_mod");
        assert_eq!(pipelines[4].owner(), "owning_mod");
    }

    #[test]
    fn a_mod_can_register_more_than_vanillas_pipeline_count() {
        let mut pipelines = Vec::new();
        {
            let mut event = crate::RegisterRenderPipelinesEvent {
                mod_id: "mine",
                pipelines: &mut pipelines,
            };
            for i in 0..131 {
                event.register(pipeline(&format!("mine:p{i}")));
            }
        }
        assert_eq!(pipelines.len(), 131);
        assert_eq!(pipelines.last().unwrap().id.to_string(), "mine:p130");

        crate::RegisterRenderPipelinesEvent {
            mod_id: "next",
            pipelines: &mut pipelines,
        }
        .register(pipeline("next:glow"));
        assert_eq!(pipelines.len(), 132);
    }

    #[test]
    fn effective_pack_programs_prepends_custom_names_ahead_of_geometry_class_chain() {
        let plain = pipeline("example:plain");
        assert!(plain.effective_pack_programs().is_empty());
        assert_eq!(plain.effective_pack_slot(), None);

        let entity = pipeline("example:entity").geometry_class(PipelineGeometryClass::Entity);
        assert_eq!(
            entity.effective_pack_programs(),
            [
                "gbuffers_entities",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            PipelineGeometryClass::Particles.default_programs(),
            [
                "gbuffers_particles",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            entity.effective_pack_slot(),
            Some(PackPassSlot::AfterOpaqueGbuffers)
        );

        let custom = pipeline("example:custom_water")
            .geometry_class(PipelineGeometryClass::Translucent)
            .pack_binding(
                PackBinding::new()
                    .program("custom_water")
                    .program("gbuffers_terrain")
                    .slot(PackPassSlot::AfterTranslucentGbuffers),
            );
        assert_eq!(
            custom.effective_pack_programs(),
            [
                "custom_water",
                "gbuffers_terrain",
                "gbuffers_water",
                "gbuffers_textured_lit",
                "gbuffers_textured",
                "gbuffers_basic",
            ]
        );
        assert_eq!(
            custom.effective_pack_slot(),
            Some(PackPassSlot::AfterTranslucentGbuffers)
        );
    }

    #[test]
    fn pack_pass_slots_follow_shader_pack_frame_order() {
        let slots = [
            PackPassSlot::Shadow,
            PackPassSlot::AfterSky,
            PackPassSlot::AfterOpaqueGbuffers,
            PackPassSlot::AfterTranslucentGbuffers,
            PackPassSlot::AfterWeather,
        ];
        assert!(slots.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            PipelineGeometryClass::Sky.default_slot(),
            PackPassSlot::AfterSky
        );
        assert_eq!(
            PipelineGeometryClass::Terrain.default_slot(),
            PackPassSlot::AfterOpaqueGbuffers
        );
        assert_eq!(
            PipelineGeometryClass::Translucent.default_slot(),
            PackPassSlot::AfterTranslucentGbuffers
        );
        assert_eq!(
            PipelineGeometryClass::Weather.default_slot(),
            PackPassSlot::AfterWeather
        );
    }

    #[test]
    fn pack_binding_syntax_rejects_bad_identifiers_and_multiline_defines() {
        let valid = PackBinding::new()
            .program("custom_terrain")
            .define("MOD_ACTIVE")
            .define_value("MOD_MATERIAL_LEAVES", "1")
            .depth_target(
                PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                    .depth_sampler("modDepthTex0")
                    .opaque_depth_sampler("modDepthTex1"),
            )
            .frame_uniform(
                PackUniformDecl::new("modProjection", PackUniformType::Mat4)
                    .with_previous("modPreviousProjection"),
            )
            .glsl_binding(PackGlslInput::new(
                "position",
                "gl_Vertex",
                PackUniformType::Vec4,
            ));
        assert_eq!(valid.validate_syntax(), Ok(()));

        for bad_define in ["", "1BAD", "BAD-NAME", "GL_RESERVED", "has space"] {
            assert!(
                PackBinding::new()
                    .define(bad_define)
                    .validate_syntax()
                    .is_err(),
                "expected invalid define name {bad_define:?} to fail"
            );
        }
        for bad_val in [
            "",
            "   ",
            "1\n#define INJECT 1",
            "1\r0",
            "1 // comment",
            "1 /* comment */",
            "1 \\",
            "#include",
            " leading",
        ] {
            assert!(
                PackBinding::new()
                    .define_value("GOOD_NAME", bad_val)
                    .validate_syntax()
                    .is_err(),
                "expected invalid define value {bad_val:?} to fail"
            );
        }
        for bad_program in ["", "../escape", "gbuffers_terrain.fsh", "has space"] {
            assert!(
                PackBinding::new()
                    .program(bad_program)
                    .validate_syntax()
                    .is_err(),
                "expected invalid program {bad_program:?} to fail"
            );
        }
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("gl_FragDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("sharedDepth")
                        .opaque_depth_sampler("sharedDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .depth_sampler("dupDepth")
                        .depth_sampler("dupDepth")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .depth_target(
                    PackDepthTarget::new(Identifier::parse("example:aux_depth").unwrap())
                        .opaque_depth_sampler("dupOpaque")
                        .opaque_depth_sampler("dupOpaque")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .frame_uniform(
                    PackUniformDecl::new("modProj", PackUniformType::Mat4).with_previous("modProj")
                )
                .validate_syntax()
                .is_err()
        );
        assert!(
            PackBinding::new()
                .frame_uniform(
                    PackUniformDecl::new("projA", PackUniformType::Mat4).with_previous("projB")
                )
                .frame_uniform(PackUniformDecl::new("projB", PackUniformType::Mat4))
                .validate_syntax()
                .is_err()
        );
    }

    #[test]
    fn shader_defines_and_vertex_shader_builder_validate_syntax() {
        let p = pipeline("example:glow")
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .define("PULSE")
            .define_value("STEPS", "4");
        assert_eq!(
            p.vertex_shader,
            Some(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        );
        assert_eq!(
            p.defines,
            vec![
                ("PULSE".to_owned(), None),
                ("STEPS".to_owned(), Some("4".to_owned())),
            ]
        );
        assert_eq!(p.validate_defines(), Ok(()));

        for bad_name in ["", "1BAD", "BAD-NAME", "GL_RESERVED", "has space"] {
            assert!(
                pipeline("example:bad")
                    .define(bad_name)
                    .validate_defines()
                    .is_err(),
                "expected invalid define name {bad_name:?} to fail"
            );
        }
        for bad_val in [
            "",
            "   ",
            "1\n#define INJECT 1",
            "1 // comment",
            "1 /* comment */",
            "1 \\",
            "#include",
        ] {
            assert!(
                pipeline("example:bad")
                    .define_value("GOOD", bad_val)
                    .validate_defines()
                    .is_err(),
                "expected invalid define value {bad_val:?} to fail"
            );
        }
    }

    #[test]
    fn pipeline_state_builders_and_modifier_overrides_apply_cleanly() {
        let p = pipeline("example:solid")
            .blend(PipelineBlend::Opaque)
            .depth(WorldDepth::Tested)
            .depth_write(true)
            .depth_bias(10, 1.5)
            .cull(true)
            .color_write(ColorWrite::COLOR)
            .polygon(PolygonMode::Line)
            .topology(PrimitiveTopology::Triangles)
            .order_independent();
        assert_eq!(
            p.state,
            PipelineState {
                blend: PipelineBlend::Opaque,
                depth_stencil: Some(DepthState {
                    compare: DepthCompare::GreaterEqual,
                    write: true,
                    depth_bias_constant: 10,
                    depth_bias_slope_scale: 1.5,
                }),
                cull: true,
                color_write: ColorWrite::COLOR,
                polygon: PolygonMode::Line,
                topology: PrimitiveTopology::Triangles,
            }
        );
        assert!(p.order_independent);
        assert_eq!(p.validate_state(), Ok(()));

        let noop = PipelineModifier::new(Identifier::parse("example:noop").unwrap());
        assert!(noop.is_noop());
        assert_eq!(noop.apply(p.state), p.state);

        let xray = PipelineModifier::new(Identifier::parse("example:xray").unwrap())
            .depth_compare(DepthCompare::LessEqual)
            .depth_write(false)
            .depth_bias(4, 0.5)
            .polygon(PolygonMode::Fill)
            .topology(PrimitiveTopology::DebugLines);
        assert!(!xray.is_noop());
        let modified = xray.apply(p.state);
        assert_eq!(
            modified.depth_stencil,
            Some(DepthState {
                compare: DepthCompare::LessEqual,
                write: false,
                depth_bias_constant: 4,
                depth_bias_slope_scale: 0.5,
            })
        );
        assert_eq!(modified.polygon, PolygonMode::Fill);
        assert_eq!(modified.topology, PrimitiveTopology::DebugLines);
        assert_eq!(modified.blend, PipelineBlend::Opaque);
        assert!(modified.cull);
        assert_eq!(modified.color_write, ColorWrite::COLOR);

        // Depthless pipeline stays depthless even under depth-overriding modifiers.
        let depthless = pipeline("example:post").depth_stencil(None);
        assert_eq!(depthless.state.depth_stencil, None);
        assert_eq!(xray.apply(depthless.state).depth_stencil, None);

        let mut modifiers = Vec::new();
        let mut event = crate::RegisterPipelineModifiersEvent {
            mod_id: "mod_b",
            modifiers: &mut modifiers,
        };
        event.register(xray);
        assert_eq!(modifiers.len(), 1);
        assert_eq!(modifiers[0].owner(), "mod_b");
    }

    #[test]
    fn validate_state_refuses_bad_depth_bias_out_of_range_target_and_mixed_mrt_blends() {
        // Non-finite depth bias slope scale refused.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let err = pipeline("example:bad_bias")
                .depth_bias(0, bad)
                .validate_state()
                .unwrap_err();
            assert!(err.contains("depth_bias_slope_scale"), "{err}");
            let mod_err = PipelineModifier::new(Identifier::parse("example:bad_mod").unwrap())
                .depth_bias(0, bad)
                .validate()
                .unwrap_err();
            assert!(mod_err.contains("depth_bias"), "{mod_err}");
        }

        // Color target index >= MAX_COLOR_TARGETS (8) refused.
        let out_of_range = pipeline("example:bad_slot")
            .color_target(MAX_COLOR_TARGETS, Some(ColorTargetState::DEFAULT));
        assert!(out_of_range.validate_state().is_err());

        // Differing blend functions across active MRT targets refused (RenderPipeline.java:392).
        let mixed = pipeline("example:mixed_mrt")
            .color_target(
                0,
                Some(ColorTargetState::blended(PipelineBlend::Translucent)),
            )
            .color_target(
                1,
                Some(ColorTargetState::blended(PipelineBlend::Custom(
                    BlendFunction::INVERT,
                ))),
            );
        assert!(mixed.validate_state().is_err());

        // Same blend function (including Opaque + unused slots) accepted across 8 targets.
        let valid_mrt = pipeline("example:valid_mrt")
            .color_target(
                0,
                Some(ColorTargetState::blended(PipelineBlend::Translucent)),
            )
            .color_target(1, None)
            .color_target(2, Some(ColorTargetState::DEFAULT))
            .color_target(
                7,
                Some(ColorTargetState::blended(PipelineBlend::Custom(
                    BlendFunction::TRANSLUCENT,
                ))),
            );
        assert_eq!(valid_mrt.validate_state(), Ok(()));
        assert_eq!(valid_mrt.effective_color_targets().len(), 8);
    }

    #[test]
    fn depth_compare_flips_for_standard_z_and_topology_index_counts_match_vanilla() {
        assert_eq!(
            DepthCompare::GreaterEqual.flip_for_standard_z(),
            DepthCompare::LessEqual
        );
        assert_eq!(
            DepthCompare::Greater.flip_for_standard_z(),
            DepthCompare::Less
        );
        assert_eq!(
            DepthCompare::LessEqual.flip_for_standard_z(),
            DepthCompare::GreaterEqual
        );
        assert_eq!(
            DepthCompare::Less.flip_for_standard_z(),
            DepthCompare::Greater
        );
        assert_eq!(
            DepthCompare::Always.flip_for_standard_z(),
            DepthCompare::Always
        );
        assert_eq!(
            DepthCompare::Equal.flip_for_standard_z(),
            DepthCompare::Equal
        );
        assert_eq!(
            DepthCompare::NotEqual.flip_for_standard_z(),
            DepthCompare::NotEqual
        );
        assert_eq!(
            DepthCompare::Never.flip_for_standard_z(),
            DepthCompare::Never
        );

        assert_eq!(PrimitiveTopology::Quads.index_count(8), 12);
        assert_eq!(PrimitiveTopology::Lines.index_count(8), 12);
        assert_eq!(PrimitiveTopology::TriangleFan.index_count(5), 9);
        assert_eq!(PrimitiveTopology::TriangleFan.index_count(2), 0);
        assert_eq!(PrimitiveTopology::Triangles.index_count(6), 6);
        assert_eq!(PrimitiveTopology::DebugLines.index_count(4), 4);
        assert_eq!(PrimitiveTopology::DebugLineStrip.index_count(4), 4);
        assert_eq!(PrimitiveTopology::Points.index_count(3), 3);
    }

    #[test]
    fn vertex_format_builder_validates_alignment_stride_names_count_and_step_rate() {
        // Valid compact 8-byte vertex format (4B uvec4 + 4B unorm8x4).
        let compact = VertexFormat::builder(VertexStepMode::Vertex)
            .attribute("PackedPos", VertexAttributeFormat::Uint8x4)
            .attribute("PackedColor", VertexAttributeFormat::Unorm8x4)
            .build()
            .unwrap();
        assert_eq!(compact.stride, 8);
        assert_eq!(compact.attributes.len(), 2);
        assert_eq!(compact.attributes[0].offset, 0);
        assert_eq!(compact.attributes[1].offset, 4);
        assert_eq!(compact.attributes[0].format.glsl_type(), "uvec4");
        assert_eq!(compact.attributes[1].format.glsl_type(), "vec4");

        // Unaligned offset refused.
        assert!(matches!(
            VertexFormat::builder(VertexStepMode::Vertex)
                .attribute_at("Pos", 2, VertexAttributeFormat::Float32x3)
                .stride(16)
                .build(),
            Err(VertexFormatError::UnalignedOffset {
                offset: 2,
                alignment: 4,
                ..
            })
        ));

        // Non-multiple-of-4 stride refused.
        assert_eq!(
            VertexFormat::builder(VertexStepMode::Vertex)
                .attribute("Uv", VertexAttributeFormat::Unorm8x2)
                .build(),
            Err(VertexFormatError::InvalidStride(2))
        );
        // Zero stride refused.
        assert_eq!(
            VertexFormat::builder(VertexStepMode::Vertex).build(),
            Err(VertexFormatError::InvalidStride(0))
        );

        // Duplicate attribute name refused.
        assert_eq!(
            VertexFormat::builder(VertexStepMode::Vertex)
                .attribute("Pos", VertexAttributeFormat::Float32x3)
                .attribute("Pos", VertexAttributeFormat::Float32)
                .build(),
            Err(VertexFormatError::DuplicateAttributeName("Pos".to_owned()))
        );

        // Invalid and gl_-prefixed names refused.
        for bad in ["", "1bad", "bad-name", "gl_Position"] {
            assert!(matches!(
                VertexFormat::builder(VertexStepMode::Vertex)
                    .attribute(bad, VertexAttributeFormat::Float32)
                    .build(),
                Err(VertexFormatError::InvalidAttributeName(_))
            ));
        }

        // More than 16 attributes refused.
        let mut too_many = VertexFormat::builder(VertexStepMode::Vertex);
        for i in 0..=MAX_VERTEX_ELEMENTS {
            too_many = too_many.attribute(format!("a{i}"), VertexAttributeFormat::Float32);
        }
        assert_eq!(
            too_many.build(),
            Err(VertexFormatError::TooManyAttributes(
                MAX_VERTEX_ELEMENTS + 1
            ))
        );

        // Step rate divisor > 1 refused (VulkanRenderPipeline.java:177-182).
        assert_eq!(
            VertexFormat::builder_with_step_rate(2)
                .attribute("InstOffset", VertexAttributeFormat::Float32x3)
                .build(),
            Err(VertexFormatError::UnsupportedStepRate(2))
        );
        assert_eq!(
            VertexFormat::builder_with_step_rate(1)
                .attribute("InstOffset", VertexAttributeFormat::Float32x3)
                .build()
                .unwrap()
                .step_mode,
            VertexStepMode::Instance
        );

        // Pipeline-level validation: binding >= 16, duplicate binding, cross-binding duplicate name,
        // and > 16 total attributes across bindings all refused.
        assert!(
            pipeline("example:bad_binding")
                .vertex_binding(MAX_VERTEX_BINDINGS as u8, compact.clone())
                .validate_vertex_bindings()
                .is_err()
        );
        assert!(
            pipeline("example:dup_binding")
                .vertex_binding(0, compact.clone())
                .vertex_binding(0, compact.clone())
                .validate_vertex_bindings()
                .is_err()
        );
        assert!(
            pipeline("example:dup_attr")
                .vertex_binding(0, compact.clone())
                .vertex_binding(1, compact.clone())
                .validate_vertex_bindings()
                .is_err()
        );
        let mut nine_a = VertexFormat::builder(VertexStepMode::Vertex);
        let mut eight_b = VertexFormat::builder(VertexStepMode::Instance);
        for i in 0..9 {
            nine_a = nine_a.attribute(format!("v{i}"), VertexAttributeFormat::Float32);
        }
        for i in 0..8 {
            eight_b = eight_b.attribute(format!("i{i}"), VertexAttributeFormat::Float32);
        }
        assert!(
            pipeline("example:too_many_total")
                .vertex_binding(0, nine_a.build().unwrap())
                .vertex_binding(1, eight_b.build().unwrap())
                .validate_vertex_bindings()
                .is_err()
        );
    }

    #[test]
    fn mesh_ref_routes_through_backend_and_refuses_without_one() {
        struct Bare;
        impl crate::Host for Bare {}
        crate::impl_host_areas!(Bare);
        impl crate::ClientHost for Bare {}
        crate::impl_client_host_areas!(Bare);

        let mesh = MeshRef::from_parts(1, 0).unwrap();
        let init = [MeshBufferInit::empty(8, 32)];
        let desc = MeshDescriptor::new(&init);
        assert_eq!(
            mesh.write_vertices(&Bare, 0, 0, &[0; 8]),
            Err(MeshError::Unavailable)
        );
        assert_eq!(
            mesh.write_indices(&Bare, 0, &[0; 4]),
            Err(MeshError::Unavailable)
        );
        assert_eq!(mesh.replace(&Bare, &desc), Err(MeshError::Unavailable));
        assert_eq!(mesh.destroy(&Bare), Err(MeshError::Unavailable));

        struct Recording {
            calls: std::cell::RefCell<Vec<&'static str>>,
        }
        impl crate::Host for Recording {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(Recording);
        impl crate::ClientHost for Recording {}
        crate::impl_client_host_areas!(Recording);
        impl crate::Backend for Recording {}
        crate::impl_backend_areas!(Recording, except(client_render));
        impl crate::prelude::ClientRenderClientBackend for Recording {
            fn mesh_write_vertices(
                &self,
                _: MeshRef,
                _: u8,
                _: u64,
                _: &[u8],
            ) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("write_vertices");
                Ok(())
            }
            fn mesh_write_indices(&self, _: MeshRef, _: u64, _: &[u8]) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("write_indices");
                Ok(())
            }
            fn mesh_replace(&self, _: MeshRef, _: &MeshDescriptor<'_>) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("replace");
                Ok(())
            }
            fn mesh_destroy(&self, _: MeshRef) -> Result<(), MeshError> {
                self.calls.borrow_mut().push("destroy");
                Ok(())
            }
        }

        let host = Recording {
            calls: std::cell::RefCell::new(Vec::new()),
        };
        mesh.write_vertices(&host, 0, 0, &[0; 8]).unwrap();
        mesh.write_indices(&host, 0, &[0; 4]).unwrap();
        mesh.replace(&host, &desc).unwrap();
        mesh.destroy(&host).unwrap();
        assert_eq!(
            *host.calls.borrow(),
            vec!["write_vertices", "write_indices", "replace", "destroy"]
        );
    }
}
