//! The `extract_level_render_state` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait DrawContexts {
    #[draw_context(surface = "mod_api::LevelRenderState", name = "LevelRenderStateWrite")]
    fn level_render_state_write();
}

pub trait Hooks {
    /// Extract custom per-view render state after vanilla level state extraction
    /// (NeoForge 26.3's `ExtractLevelRenderStateEvent`, vanilla `LevelExtractor.extract`).
    /// Runs once per world view before `submit_custom_geometry`, `prepare_render_buffers`, and
    /// `render_level_stage`. Client-only, noncancellable. `view` lends the frame's camera, world
    /// partial tick, game time, improved-transparency state and visible sections; `state` stores
    /// typed values under [`RenderDataKey`]s registered at startup, cleared automatically every
    /// view and readable via [`WorldRenderView::render_data`] during world render stages.
    #[notes = r#"## Level render state extraction

`Hooks::extract_level_render_state` is the client counterpart of NeoForge 26.3's
`ExtractLevelRenderStateEvent`. It fires once per world view before
`Hooks::submit_custom_geometry`, `Hooks::prepare_render_buffers`, and `Hooks::render_level_stage`,
and never fires on title or connect screens.

Register each [`RenderDataKey<T>`] during `Hooks::register` via
`Registrar::register_render_data_key`; startup refuses two keys sharing one namespaced ID with
different Rust types, naming both mods and both types. Inside `extract_level_render_state`, store
extracted values with `event.state.set_render_data(&KEY, value)` and read values written by
earlier listeners (in dependency order) with `event.state.get_render_data(&KEY)`. Subsequent world
render hooks (`submit_custom_geometry`, debug renderers, `prepare_render_buffers`,
`render_level_stage`) read the frozen view values through `event.view.render_data(&KEY)`.

The per-view store is cleared before every view's extraction and dropped when the view finishes,
so values from frame N never leak into frame N+1 and mod-owned values are dropped cleanly. Lookups
verify `TypeId` through `Any` and return `None` on mismatch. A panic disables that mod's listener
for this hook for the session, with an error log and toast."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ExtractLevelRenderStateEvent",
        cancellable = false
    )]
    fn extract_level_render_state(
        view: &mod_api::WorldRenderView<'_>,
        state: &mut dyn LevelRenderStateWrite,
    );
}
