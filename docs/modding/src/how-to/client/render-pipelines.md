# Drawing with your own render pipeline

Draw world quads with your own GLSL core shader program. Register a `RenderPipeline`, resolve
its handle, and submit quads through a `WorldDrawContext` (in `submit_custom_geometry` or
a debug renderer). This is the counterpart of NeoForge's `RegisterRenderPipelinesEvent`: you
write vanilla-style GLSL core shaders (`.vsh` and `.fsh`), optionally reusing an existing vertex
shader such as `minecraft:core/position_tex_color`, and the game translates them via
`sunrise-glsl` and builds every GPU object.

## 1. Write the shaders

Put GLSL files at `assets/<namespace>/shaders/core/<path>.vsh` and `.fsh` (referred to by the
identifier `<namespace>:core/<path>`, matching vanilla's `withVertexShader` and
`withFragmentShader`). Shared includes live under `assets/<namespace>/shaders/include/` and are
brought in with `#include <namespace:file.glsl>`, `#include "relative.glsl"`, or `#moj_import`
through the active resource stack (vanilla assets, loaded mods, and resource packs).

```glsl
// assets/example/shaders/core/glow.fsh
#version 330
#include <minecraft:globals.glsl>
#include <minecraft:dynamictransforms.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;

void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef PULSE
    color.rgb *= 1.0 + sin(GameTime * 24000.0 * 0.2);
#endif
    fragColor = vec4(color.rgb, color.a * (1.0 - texCoord0.y));
}
```

If you do not need custom vertex math, you can omit `glow.vsh` and call
`.vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())` on registration.
When writing a custom `.vsh`, declare any subset of the vertex attributes by vanilla name:

| Input | Type / Include | Value the game binds |
| --- | --- | --- |
| `Position` | `in vec3 Position;` | Quad corner in blocks, camera-relative. |
| `UV0` | `in vec2 UV0;` | Quad texture coordinate as given on the corners. |
| `Color` | `in vec4 Color;` | Quad colour in vanilla's gamma space. |
| `Globals` | `#include <minecraft:globals.glsl>` | Frame globals (`CameraBlockPos`, `GlintAlpha`, `CameraOffset`, `GameTime`, `ScreenSize`, `MenuBlurRadius`, `UseRgss`). |
| `Projection` | `#include <minecraft:projection.glsl>` | View projection (`ProjMat`, reversed 0..1 depth; `RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` is always defined). |
| `DynamicTransforms` | `#include <minecraft:dynamictransforms.glsl>` | `ModelViewMat` (camera rotation), `TextureMat` (identity), `ColorModulator` (white), `ModelOffset` (zero). |
| `Fog` | `#include <minecraft:fog.glsl>` | Frame fog (`FogColor`, environmental and render-distance starts/ends, sky and cloud ends). |
| `Sampler0` / `sampleSprite` | `#include <sunrise:pipeline_sprite.glsl>` | Sprite atlas and `vec4 sampleSprite(vec2 uv)` when `.textured(...)` is set. |

### Colour space

Mod pipeline GLSL programs work in **vanilla's gamma space**, just like vanilla core shaders:
write `texture * vertexColor * ColorModulator` directly. On frames storing vanilla's encoding the
output is stored as written; on frames holding linear light the engine automatically decodes the
fragment shader's location-0 output after `main()` (clamping negative or NaN RGB to 0 and alpha
coverage to `[0, 1]`).

### Crash and memory containment bounds

Because Sunrise translates untrusted GLSL in-process with naga rather than handing raw text to
the GPU driver, expanded shader stages are checked against crash-containment bounds before naga
parses them: 1 MiB expanded bytes, include depth 16 (1,024 pastes, 8 MiB scanned), at most 200
chained binary/member/index operators in one expression, and at most 200 nested brackets,
consecutive unary prefix operators, or brace-less control statements. Split long expressions
with local variables and add braces where needed.

## 2. Register the pipeline

Call `event.register(pipeline)` in `register_render_pipelines` at client startup.
The event adds declarations in order and does not expose earlier registrations.
Resource reloads rebuild the registered pipelines from the current shader and include files;
they do not call the registration hook again.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.register(
        RenderPipeline::new(
            Identifier::parse("example:glow").unwrap(),      // the pipeline's id
            Identifier::parse("example:core/glow").unwrap(), // assets/example/shaders/core/glow.{vsh,fsh}
            PipelineBlend::Additive,
            WorldDepth::Tested,
        )
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        .define("PULSE")
        .define_value("INTENSITY", "2"),
    );
}
```

- `.vertex_shader(id)` reuses another program's `.vsh` (for example `minecraft:core/position_tex_color`
  or a library mod's vertex shader). Omit it when your own `<path>.vsh` sits beside `<path>.fsh`.
- `.define("NAME")` and `.define_value("NAME", "VALUE")` compile preprocessor macros into both
  stages (matching vanilla's `withShaderDefine`), letting several pipelines share one GLSL
  program. Define names must be valid non-`GL_` GLSL identifiers and values must be single-line
  tokens without comments, backslashes, or `#`; invalid defines fail startup registration.
- `PipelineBlend::Translucent` lays the result over the frame by its alpha, and `Additive` adds
  it, scaled by its alpha. `WorldDepth::Tested` hides the quads behind nearer world surfaces, and
  `ThroughWalls` draws them over everything. No pipeline writes depth.

Any valid `Identifier` is accepted. Use a unique ID for every mod pipeline, regardless of its
required or optional policy. A mod ID collision fails startup. Collisions with vanilla are checked
in the corresponding required or optional table. If startup reports missing or invalid
`render-pipelines.json`, prepare the client resources again.

### Required or optional

A pipeline is required unless you register it with `.optional()`, as NeoForge's `registerPipeline`
and `registerOptionalPipeline` are:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested).optional(),
);
```

| Policy | When a resource load cannot build it |
| --- | --- |
| Required (`RenderPipeline::new`) | The load fails with a message naming each required pipeline and why. Startup cannot finish with a broken required pipeline. A failed resource reload keeps the previous resources, so handles from before it still draw. |
| Optional (`.optional()`) | The log has one line naming the pipeline and why. It resolves to `RenderPipelineError::Invalid`, and every other pipeline builds and draws. |

Client resource pack recovery only runs if an optional pack was selected. It clears and saves the
failing selection before reloading the required packs and mod content. If recovery fails, the
error is reported and that selection stays cleared. If the player has already committed a newer
selection during startup, the queued load uses it instead. Create World's data-pack editor is
unaffected.

A load cannot build a pipeline when its `.vsh`, `.fsh`, or an included file is missing, the
shader is refused, or the GPU rejects it. The game never panics over a pipeline's shader.

### Checking shaders before you run the game

`cargo sunrise-mod check`, run in the mod's folder, translates and validates every GLSL core
shader program under `assets/<namespace>/shaders/core/` with the same `sunrise-mod-pipeline`
layer the game uses, including SPIR-V (Vulkan), Metal (MSL), and DirectX 12 (HLSL) backend
translation checks.

When `SUNRISE_RESOURCES` points to your prepared 26.3 resources, `minecraft:` includes and
vertex shaders are read directly from them. When prepared resources are absent (such as in CI or
registry checks), the tool falls back to Sunrise-written interface-only stubs for `minecraft:*`
includes and standard vertex shaders so every interface error is still caught offline; the game's
resource load remains the authority. Because the tool does not execute your Rust registration
code, a `.fsh` with no `.vsh` beside it is checked against `minecraft:core/position_tex_color`,
and a program referencing `sampleSprite`, `pipeline_sprite.glsl`, `SpriteRect`, or `Sampler0` is
checked as textured.

## 3. Resolve and draw

```rust,ignore
fn client_setup(&self, context: &mut dyn ClientSetupContext, _event: &mut ClientSetupEvent<'_>) {
    let id = Identifier::parse("example:glow").unwrap();
    *self.glow.lock().unwrap() = context
        .render_assets()
        .render_pipeline(&id)
        .ok();
}

fn submit_custom_geometry(&self, _host: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
    let Some(glow) = *self.glow.lock().unwrap() else { return };
    let quad = PipelineQuad::spanning(corners, 0xFFFF_C040);
    if let Err(error) = event.draw.pipeline_quads(glow, &[quad]) {
        log::warn!("glow not drawn: {error:?}");
    }
}
```

Resolve again in `client_resources_reloaded`: every resource reload builds the pipelines again
from the new resources, so a resource pack can replace your shaders or includes, and a handle
from before the reload answers `Stale`.

| Error | When |
| --- | --- |
| `RenderPipelineError::Unknown` | No mod registered that id. |
| `RenderPipelineError::Invalid` | The pipeline is optional and its shader file is missing, was refused or was rejected by the GPU. The log has a line `render pipeline <id> from mod <owner> refused: <file>: <reason>` or `render pipeline <id> from mod <owner> refused by the GPU: <reason>`. |
| `RenderPipelineError::Stale` | The handle is from before a resource reload. |

`pipeline_quads` returns `PipelineDrawError::Pipeline(...)` when the handle is invalid or stale,
drawing nothing for that call. Any quad with a non-finite corner, camera-relative `f32` overflow,
or non-finite texture coordinate is ignored while the valid quads in the batch still draw, and
quad storage grows dynamically into the GPU buffers without a 4,096 cap. An `Invalid` pipeline
never stops the game from starting; a required pipeline that cannot be built does.

## What a pipeline draws, and when

`WorldDrawContext::pipeline_quads` in `submit_custom_geometry` or a debug renderer submits quads
for the deferred world pass. Quads with non-finite coordinates or UVs are skipped individually
without rejecting valid quads in the same slice; accepted pipeline quads draw after the pass's
plain quads and lines, in submission order.

## Sampling a texture

A pipeline can sample one texture (vanilla's `Sampler0`). Name it when you register the pipeline:

```rust,ignore
event.register(
    RenderPipeline::new(
        id,
        Identifier::parse("example:core/charge").unwrap(),
        PipelineBlend::Additive,
        WorldDepth::Tested,
    )
    .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
    .textured(PipelineTexture::Sprite(
        Identifier::parse("example:block/charge").unwrap(),
    )),
);
```

`PipelineTexture::Sprite(id)` names a sprite of the block or item atlas by its texture id:
`example:block/charge` is `assets/example/textures/block/charge.png` as the `minecraft:blocks`
atlas holds it, and `minecraft:item/diamond` names an item sprite. Animated sprites keep their
animation. A textured pipeline samples a rectangle of the atlas; it does not sample a standalone
texture file.

In the fragment shader, include `<sunrise:pipeline_sprite.glsl>` and call `sampleSprite(uv)`:

```glsl
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <sunrise:pipeline_sprite.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
out vec4 fragColor;

void main() {
    fragColor = sampleSprite(texCoord0) * vertexColor * ColorModulator;
}
```

`sampleSprite(uv)` maps the fraction of `uv` into the sprite's own rectangle (`uv` 0 to 1 spans
it once; beyond that, negative values too, it repeats) and loads one texel by integer coordinates
at mip level 0: nearest, unfiltered, so no texel of a neighbouring sprite can enter, even at the
far edge. It returns the texel in vanilla's gamma space with its alpha. `texCoord0` is still what
the quad's corners gave, so existing quad UVs work unchanged.

A texture is part of the pipeline, so it is checked when the pipeline is built, at startup and on
each reload. A refused texture is named by `PipelineTextureError` and refuses the pipeline as a
missing shader does: a required one fails the load, and an optional one is logged with the named
cause and resolves to `Invalid`:

| Cause | When |
| --- | --- |
| `NoSuchSprite` | Neither the block nor the item atlas has the sprite (`… refused by the GPU: its texture sprite <name>: NoSuchSprite: …`) |
| `NotASpriteKey` | The id names a file (a `textures/` path or a `.png` name), not a sprite key |

A pipeline without `.textured(…)` binds neither `Sampler0` nor `SpriteRect`: referencing either
(or including `<sunrise:pipeline_sprite.glsl>`) refuses the shader with a note naming
`RenderPipeline::textured`.

## Storage buffers

A pipeline reads large, sparsely updated data (compact geometry records, per-instance tables) from
a std430 storage buffer. Create it once with `host.create_storage_buffer(&StorageBufferDescriptor::new(size))`
(a multiple of 4 bytes, up to the device's `max_buffer_size`), update ranges with
`buffer.write_range(host, offset, bytes)` (queued and applied in call order before the next frame),
and free it with `buffer.release(host)`. It survives a resource reload and a world close, and is
freed when the mod unloads.

Declare it in the layout with `BindEntry::new("Tints", BindingKind::StorageBuffer { read_only: true })`
and write the block in GLSL with the same access (`layout(std430) readonly buffer Tints { vec4 tint[]; };`;
a layout that says read-write needs a block without `readonly`). Bind a range of the buffer per draw
with `DrawBindings::new().storage_buffer("Tints", buffer.slice(offset, length))`. A range is at most
the device's `max_storage_buffer_binding_size` bytes with its offset a multiple of
`min_storage_buffer_offset_alignment`; the buffer itself may be larger. A writable block in the
vertex stage needs `VERTEX_WRITABLE_STORAGE`; a device without it refuses such a pipeline. Every
refusal is a `BufferError` or `PipelineDrawError`, never a panic.

## Mip chains and storage images on render targets

A render target made with `host.create_render_target(&desc)` can carry a mip chain, the way a
vanilla `GpuTexture` has `getMipLevels()`: `RenderTargetDescriptor::new(size)
.color_target(RenderTargetFormat::R32Float).mip_levels(n)`, with `n` from 1 to
`RenderTargetDescriptor::max_mip_levels(size)` (`32 - max(width, height).leading_zeros()`). A chain
has colour attachments only. `OffscreenPass::new(target).mip_level(k)` renders into level `k` at that
level's size, and `TextureSource::RenderTargetColorMip(RenderTargetMipView::new(target, attachment,
base_mip, mip_levels))` samples a range of levels, as vanilla's `GpuTextureView` has `baseMipLevel()`
and `mipLevels()`. Reading level `k` while rendering level `k + 1` is allowed; the same level, or a
range holding it, is `PipelineDrawError::TargetReadWriteHazard`.

For a shader that writes a level directly (a depth pyramid, a blur chain), create the target with
`.storage(StorageImageAccess::WriteOnly)`, declare the image in the layout with
`BindEntry::new("Next", BindingKind::StorageImage { access: StorageImageAccess::WriteOnly })`, write
`layout(r32f) writeonly uniform image2D Next;` in GLSL (`rgba8`, `rgba16f` and `r32f` are the
formats a target can have; the shader's `readonly`/`writeonly` qualifiers must agree with the
access), and bind one level with `DrawBindings::new().storage_image("Next",
StorageImageBinding::new(target, 0, level))`. A target can be used as a storage image only in the
access it was created for (`ReadWrite` covers both others). A format or access the adapter lacks
is refused when the target is created, with `RenderTargetError::UnsupportedStorage`: ask
`host.render_capabilities()` first. Write-only and read-only storage work for every format;
`read_write_storage` lists the formats with read-write storage, and `float32_filterable` says
whether an `R32Float` target can be sampled with `SamplerFilter::Linear`.

A render target survives a resource reload and a world close, and is freed by
`target.release(host)` or when its mod unloads.

## Participating in translated shader packs

When an Iris/OptiFine shader pack is active and owns terrain rendering, a mod pipeline without a
pack override draws its own GLSL program into the pack's world targets before the pack's
composite chain runs, so composite and `final` passes shade it alongside the world.

To let an active Iris/OptiFine shader pack shade a pipeline's geometry with its own program, declare
its `PipelineGeometryClass` on registration via `.geometry_class(...)`:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested)
        .geometry_class(PipelineGeometryClass::Terrain),
);
```

Each geometry class selects a standard Iris/OptiFine program fallback chain (for example,
`Terrain` resolves `gbuffers_terrain -> gbuffers_textured_lit -> gbuffers_textured -> gbuffers_basic`,
and `Particles` resolves `gbuffers_particles -> gbuffers_textured_lit -> gbuffers_textured -> gbuffers_basic`).
Pipelines that need pack-defined programs, preprocessor defines, an auxiliary depth target, or
per-frame pack uniforms attach an optional `.pack_binding(PackBinding::new()...)`. Startup refuses
invalid GLSL identifiers, reserved Iris/OptiFine sampler/image/uniform names, and cross-mod or
namespace collisions across samplers, uniforms, defines, and depth-target IDs.

During a world draw hook, feed declared per-frame pack uniforms with
`ctx.set_pack_uniform(name, PackUniformValue::...)` (returns `false` without panicking when `name`
or its `PackUniformType` is not registered; any `previous_name` declared on the uniform is tracked
automatically across frames). To inspect whether an active shader pack bound a specific pipeline to
one of its translated programs (`PackOverrideState::Bound`), left it on the mod's own fallback
shader (`PackOverrideState::UnboundPack`), or no pack is active (`PackOverrideState::NoPack`), call
`ctx.pack_override_state(pipeline)`. To check generally whether a shader pack is currently loaded
and rendering the world (always `false` on the title screen, matching `IrisApi.isShaderPackInUse()`),
call `host.shader_pack_active()`.

## Fixed-function state (`PipelineState`)

Every `RenderPipeline` carries a `PipelineState` with vanilla's core rasterization, depth, blend,
and colour-write settings. `RenderPipeline::new` sets `blend` and `depth` and defaults the rest to
`depth_write: false`, `cull: false` (two-sided), `color_write: ColorWrite::ALL`, and
`polygon: PolygonMode::Fill`. Override any field with builder methods:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Opaque, WorldDepth::Tested)
        .depth_write(true)
        .cull(true)
        .color_write(ColorWrite::COLOR)
        .polygon(PolygonMode::Fill),
);
```

- `PipelineBlend::Opaque` writes solid colour directly without alpha blending (letting a mod draw
  solid geometry with `depth_write(true)`).
- `PolygonMode::Line` requires the device's wireframe feature (`POLYGON_MODE_LINE`); if the GPU
  lacks it, the pipeline (or modifier variant) is refused.

## Pipeline modifiers (`register_pipeline_modifiers`)

Register reusable data-only state overrides in `Hooks::register_pipeline_modifiers` (NeoForge's
`RegisterPipelineModifiersEvent`):

```rust,ignore
fn register_pipeline_modifiers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterPipelineModifiersEvent<'_>,
) {
    event.register(
        PipelineModifier::new(Identifier::parse("example:xray").unwrap())
            .depth(WorldDepth::ThroughWalls)
            .depth_write(false),
    );
}
```

Resolve the modifier once in `client_setup` (or `client_resources_reloaded`) with
`host.render_assets().pipeline_modifier(&id)` and wrap world draws in
`WorldDrawContext::with_pipeline_modifier(modifier, &mut |draw| { ... })`. Scopes nest: the most
recently pushed modifier applies first, then older ones, so the outermost modifier wins on any
field both set (`PipelineModifierStack.apply`). Modifiers apply to `lines`, `quads`,
`pipeline_quads`, and `draw_model_instances` (not `name_tag` labels). Modified GPU pipeline
variants are compiled lazily and cached per `(pipeline, modifier stack)` until the next reload;
a variant the GPU refuses is logged once with both IDs and skipped while the rest of the frame
draws.

## Order-independent transparency (`OitTranslucent`)

To draw a translucent or additive pipeline in vanilla 26.3's improved-transparency passes, opt in
with `.order_independent()` and include `<minecraft:oit.glsl>` in the fragment shader:

```rust,ignore
event.register(
    RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested)
        .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
        .order_independent(),
);
```

```glsl
#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:oit.glsl>

in vec2 texCoord0;
in vec4 vertexColor;
#ifndef OIT_ALPHA_ONLY
out vec4 fragColor;
#endif

void main() {
    vec4 color = vertexColor * ColorModulator;
#ifdef OIT_ALPHA_ONLY
    executeAlphaOnlyPhase(gl_FragCoord.z, color.a);
#else
#ifdef OIT_ACCUMULATE
    color = sampleColorForAccumulation(color);
#endif
    fragColor = color;
#endif
}
```

At resource load the engine translates three OIT variants (`OIT_ALPHA_ONLY` + `OIT_DEPTH_BOUNDS`,
`OIT_ALPHA_ONLY` + `OIT_TRANSMITTANCE`, and `OIT_ACCUMULATE`; plus `OIT_ADDITIVE` for
`PipelineBlend::Additive`) and binds `DepthBoundsSampler`, `Coeff0`, and `Coeff1`. Prepare quads
for `RenderLevelStage::OitTranslucent(stage)` via `PrepareRenderBuffersContext::pipeline_quads`
and draw the returned `PreparedWorldDraw` in `render_level_stage`. Submitting an `Opaque` pipeline
or one without `.order_independent()` to `OitTranslucent` returns
`PipelineDrawError::Pipeline(RenderPipelineError::Invalid)` and logs once per mod and pipeline.
`cargo sunrise-mod check` validates all three OIT variants using prepared resources or Sunrise's
interface-only `oit.glsl` stub.

