//! The `entity_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The entity `entity` of type `kind` (`minecraft:zombie`, `minecraft:item`) is about to tick
    /// in the level `dimension` (NeoForge's `EntityTickEvent.Pre`, posted right before each
    /// entity's tick in `tickNonPassenger` and `Entity.rideTick`).
    /// Movement phase 1 (tick start), before travel and movement.
    /// Told on the server for the mobs, players, item entities, minecarts, boats, general entity
    /// rows and projectiles that tick in an entity-ticking chunk, and the entities the server
    /// keeps without simulating them (displays, markers, interactions), whose tick is only told
    /// (not a player's own boat or cart, which ticks with the player); and on a client for every
    /// entity the client ticks (`ClientLevel.tickNonPassenger` / `Entity.rideTick`), including
    /// the local player (in addition to `player_tick_pre` / `player_tick_post`), items,
    /// experience orbs, primed TNT, falling blocks, driven and observed vehicles, armor stands,
    /// general entity rows and other tracked entities; on a frozen client level, for players and
    /// entities in a player-bearing passenger tree (`TickRateManager.isEntityFrozen`); `host`
    /// says which side.
    /// Every observed entity is bracketed individually (`Pre(A)`, tick `A`, `Post(A)`, `Pre(B)`,
    /// ...), matching NeoForge's `EntityTickEvent.Pre` / `.Post` timing. On the server, player
    /// Pre runs before the observed parts of `ServerPlayer.tick`, not the connection body's
    /// `player_tick_pre`. For a passenger, `Entity.commonTick` counts its tick before
    /// `entity_tick_pre` runs inside `Entity.rideTick`, and its vehicle still positions it when
    /// canceled.
    /// Return [`Flow::Cancel`] to skip this entity's observed tick work and suppress its Post.
    /// Removing a general entity during its own Pre skips its tick and Post and dismounts its
    /// passengers unless dismount is vetoed. An eligible former passenger can still tick as a
    /// root in this pass if it has not ticked and its join-order slot is still ahead. An
    /// already-passed slot is not revisited.
    /// Version 5 expands client coverage to every entity the client ticks, the local player
    /// included.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_tick_pre",
        dispatch = "flat",
        side = "both",
        version = 5,
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityTickEvent.Pre",
        cancellable = true
    )]
    fn entity_tick_pre(dimension: &str, entity: u32, kind: &str, canceled: &mut bool);
}
