//! The `register_feature_renderers` hook: declarations the mod API generator reads (see
//! `hooks/mod.rs`).

pub trait Hooks {
    /// Register custom feature renderers once at client startup, before resources load
    /// (NeoForge 26.3's `RegisterFeatureRenderersEvent`, vanilla 26.3's `FeatureRenderDispatcher`).
    ///
    /// Call `event.renderers.register(&TYPE, renderer)` with a [`FeatureRendererType<S>`] and a
    /// [`FeatureRenderer<S>`]. Duplicate registrations for the same type identifier fail startup
    /// naming both mods. A listener that panics leaves earlier registrations unchanged.
    #[notes = r#"## Custom feature renderers

`Hooks::register_feature_renderers` is the client startup counterpart of NeoForge 26.3's
`RegisterFeatureRenderersEvent`. Declare each [`FeatureRendererType<S>`] as a `const` with a
namespaced identifier (`"namespace:path"`), register its Rust type identity during
`Hooks::register` via `Registrar::register_feature_renderer_type` (which uses the shared startup
`(namespaced id, Rust type)` identity table to refuse cross-type collisions naming both mods and
both types), and bind a [`FeatureRenderer<S>`] here via `event.renderers.register(&TYPE, renderer)`.

During world rendering (`Hooks::submit_custom_geometry` or a `DebugRenderer`), submit typed nodes
into any vanilla [`RenderPhaseKey`] (mirroring NeoForge's `RenderPhaseKeys`: `Solid`, `Shadows`,
`NameTags`, `Texts`, `ShapeOutlines`, `TranslucentBlocksAndItems`, `TranslucentModels`,
`TranslucentCustomGeometry`, `TranslucentGizmos`, `BreakingOverlay`, `WaterMask`, `AfterTerrain`,
`AlwaysOnTop`, `Outline`) using `WorldDrawContext::submit_feature(&TYPE, phase, node)`.

Submitted nodes are batched per `(RenderPhaseKey, FeatureRendererType)` in submission order and
flushed in [`RenderPhaseKey::EXECUTION_ORDER`], passing `&[S]` to `FeatureRenderer::render` along
with `&dyn ClientHost`, the frame's `&WorldRenderView<'_>` (including extracted `RenderDataKey`
values), and `&mut dyn WorldDrawContext`. Lookups verify `TypeId` through `Any` (`TypeMismatch` on
mismatch, never a pointer cast). All queued nodes are cleared between views and dropped when the
view ends so mod-owned values are dropped before any mod library unloads. A panicking feature
renderer is isolated and disabled for the rest of the session with an error log and toast."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "renderers",
        context = "registration",
        neoforge = "RegisterFeatureRenderersEvent",
        version = 1
    )]
    fn register_feature_renderers(renderers: &mut dyn FeatureRenderers);
}
