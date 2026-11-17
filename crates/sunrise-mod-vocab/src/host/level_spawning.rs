//! The `level_spawning` area: natural spawning limits and flags, and spawning a mob. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_spawning` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSpawningBackend {
    /// Side: server.
    /// The level's natural spawning limits by category ([`SpawnLimits`]):
    /// [`SpawnLimits::VANILLA`] until a mod sets others. Reads parked levels.
    /// None on a client and for a stale handle.
    #[access(WorldOnly)]
    fn level_spawn_limits(&self, level: LevelRef) -> Option<SpawnLimits>;
    /// Side: server.
    /// Which categories the level's natural spawning may spawn ([`SpawnFlags`]):
    /// both on until a mod turns one off. Reads parked levels. None on a
    /// client and for a stale handle.
    #[access(WorldOnly)]
    fn level_spawn_flags(&self, level: LevelRef) -> Option<SpawnFlags>;
    /// Side: server.
    /// `ServerLevel.canSpawnEntitiesInChunk`: whether natural spawning may place
    /// a mob in chunk `pos` (`[x, z]`), the question it asks of every spawn
    /// position outside the chunk it started in. Sunrise's answer is the one
    /// its spawner uses: the chunk is loaded. Reads parked levels and loads
    /// nothing. None on a client and for a stale handle.
    #[access(WorldOnly)]
    fn level_can_spawn_entities_in_chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<bool>;
}
/// The handles' writes of the `level_spawning` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSpawningBackendMut {
    /// Side: server.
    /// Spawns a mob of `kind` (`minecraft:zombie`) at `pos` in the level as
    /// `EntityType.spawn` does with `reason`: the entity type creates it (its
    /// Parts and attachments with it, and `entity_constructing`), with a random
    /// yaw from the level random, then `finalize_spawn` and `finalizeSpawn`
    /// with that reason (equipment, variants, jockeys), and it joins the level
    /// (`entity_join_level`), not persistent, as a natural spawn joins. The
    /// level's spawn limits, flags and spawn rules don't apply, as they don't
    /// to `/summon`. The new mob as a handle. None, with nothing spawned, for a
    /// kind that isn't a mob, a position that isn't finite or is outside the
    /// spawnable bounds (`Level.isInSpawnableBounds`), an unloaded chunk, a
    /// stale handle, a level not in place, a client, a `finalize_spawn` that
    /// cancelled the spawn or a mob a mod kept out.
    /// Inside a transaction its join waits for the root commit, so a mod keeping the mob out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    #[access(WorldOnly)]
    fn level_spawn_mob(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        reason: SpawnReason,
    ) -> Option<EntityRef>;
    /// Side: server.
    /// Sets the level's natural spawning limits ([`SpawnLimits`]), from the next
    /// spawn pass on. They hold until the world closes, while the level is
    /// parked and across its closing and opening again, and are not saved: the
    /// world opens with vanilla's, so a mod sets its own in `level_load`. Mobs
    /// already in the level stay. False, with nothing set, on a client and for
    /// a stale handle or a level that isn't open.
    #[access(WorldOnly)]
    fn level_set_spawn_limits(&mut self, level: LevelRef, limits: SpawnLimits) -> bool;
    /// Side: server.
    /// Sets which categories the level's natural spawning may spawn
    /// ([`SpawnFlags`]), from the next spawn pass on. Kept and refused as
    /// `set_spawn_limits`.
    #[access(WorldOnly)]
    fn level_set_spawn_flags(&mut self, level: LevelRef, flags: SpawnFlags) -> bool;
}
