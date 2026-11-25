//! The `entity_tick_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The entity `entity` of type `kind` ticked in the level `dimension` (NeoForge's
    /// `EntityTickEvent.Post`): one `entity_tick_pre` was told and did not cancel, including one
    /// that died or was removed in its tick.
    /// Movement phase 6 (tick end), after travel and movement.
    /// Every observed entity is bracketed individually right after its own tick. Told where
    /// `entity_tick_pre` is, on the server and on a client (`host` says which side). Version 5
    /// expands client coverage to every entity the client ticks, the local player included.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_tick_post",
        dispatch = "flat",
        side = "both",
        version = 5,
        contract = "listen",
        neoforge = "EntityTickEvent.Post",
        cancellable = false
    )]
    fn entity_tick_post(dimension: &str, entity: u32, kind: &str);
}
