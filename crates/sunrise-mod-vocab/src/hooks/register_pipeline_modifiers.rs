//! Client pipeline modifier registration.

pub trait Hooks {
    /// Register pipeline modifiers once at client startup (NeoForge's
    /// `RegisterPipelineModifiersEvent`): call `event.register(modifier)` with each
    /// [`PipelineModifier`]. Modifiers are plain data state overrides (blend, depth, depth write,
    /// face culling, colour write mask, polygon mode) applied during world drawing via
    /// [`WorldDrawContext::with_pipeline_modifier`]. Duplicate modifier IDs fail startup naming
    /// both mods.
    #[notes = r#"## Pipeline modifiers

`Hooks::register_pipeline_modifiers` is the client registration-phase counterpart of NeoForge's
`RegisterPipelineModifiersEvent`. A modifier is plain data (`PipelineModifier`): each field is an
optional override of `PipelineState` (`blend`, `depth`, `depth_write`, `cull`, `color_write`,
`polygon`). Because modifiers are data rather than closures, applying them is idempotent by
construction and the renderer caches each `(pipeline, modifier stack)` variant lazily.

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

Resolve a modifier once in `client_setup` (or `client_resources_reloaded`) with
`host.render_assets().pipeline_modifier(&id)` and apply it around draws with
`WorldDrawContext::with_pipeline_modifier(modifier, &mut |draw| { ... })`. Scopes nest: the most
recently pushed modifier applies first, then older ones, so for any field set by two modifiers
the outermost wins (`PipelineModifierStack.apply`). Modifiers apply to `lines`, `quads`,
`pipeline_quads`, and `draw_model_instances` (not `name_tag` labels). On OIT variants, `blend`
and `depth_write` overrides are ignored because OIT owns target blending and read-only depth.
Variants the GPU refuses (such as `PolygonMode::Line` on a device without wireframe support) are
logged once with both IDs and skipped while the rest of the frame draws."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "modifiers",
        registration_append = "attributed",
        context = "registration",
        neoforge = "RegisterPipelineModifiersEvent"
    )]
    fn register_pipeline_modifiers(modifiers: &mut Vec<mod_api::PipelineModifier>);
}
