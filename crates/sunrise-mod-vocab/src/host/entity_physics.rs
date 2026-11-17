//! The `entity_physics` area: an entity's box, motion and collision. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_physics` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPhysicsBackend {
    /// Side: both.
    /// The entity's velocity in blocks a tick (`Entity.getDeltaMovement`). A
    /// player's is the move its client last reported (`getKnownMovement`); an
    /// entity Sunrise doesn't simulate (a display, a painting) has none, so
    /// zero. On a client, another entity's is the motion the server last
    /// sent it (`Entity.lerpMotion`). API 379.
    #[access(WorldOnly)]
    fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: both.
    /// The entity's box (`Entity.getBoundingBox`): its lowest and highest
    /// corner. Displays and markers have zero-sized physical boxes; interactions use
    /// their saved dimensions. None for End crystals and the dragon; a dragon part's is its own. On a client, reads
    /// its own copy of a tracked entity, or the local player's predicted box.
    #[access(WorldOnly)]
    fn entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]>;
    /// Side: server.
    /// The entity's width and height (`Entity.getBbWidth`, `getBbHeight`):
    /// its box's, so a baby's or a scaled entity's own.
    #[access(WorldOnly)]
    fn entity_dimensions(&self, entity: EntityRef) -> Option<[f64; 2]>;
    /// Side: server.
    /// `Entity.noPhysics`: a player's is whether it spectates (`Player.tick`
    /// sets it from `isSpectator`); a mob's is false (a vex sets it only for
    /// the length of its own tick); an arrow's or a trident's is its
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. The
    /// ender dragon, an area-effect cloud, an ominous item spawner and a
    /// shulker bullet set it in their constructors and never clear it.
    #[access(WorldOnly)]
    fn entity_no_physics(&self, entity: EntityRef) -> Option<bool>;
}
/// The handles' writes of the `entity_physics` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPhysicsBackendMut {
    /// Side: server.
    /// `Entity.resetFallDistance`: how far the entity has fallen is forgotten, so its next
    /// landing hurts it as a step does (a fan's or a launcher's push, a mod's soft block).
    /// Players too, the acting one and guests. False for a stale handle or an entity not in
    /// the level in place; true for a kind that keeps no fall distance, which has none to forget.
    #[access(WorldOnly)]
    fn entity_reset_fall_distance(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Sets the entity's velocity (`Entity.setDeltaMovement`); clients see it
    /// as they see the entity move. False for a player (its client moves
    /// it), an entity Sunrise doesn't simulate, a value that isn't finite,
    /// or on a client. API 379.
    #[access(WorldOnly)]
    fn entity_set_velocity(&mut self, entity: EntityRef, velocity: [f64; 3]) -> bool;
    /// Side: server.
    /// Adds `delta` to the entity's velocity (`Entity.push`,
    /// `addDeltaMovement`). False for an unavailable player, an entity
    /// Sunrise doesn't simulate, or a value that isn't finite.
    #[access(WorldOnly)]
    fn entity_push(&mut self, entity: EntityRef, delta: [f64; 3]) -> bool;
    /// Side: server.
    /// Sets the player's collision exclusions, boxes of its own level
    /// (`[min_x, min_y, min_z, max_x, max_y, max_z]`) in which that level's blocks don't stop it:
    /// the part of a block's collision shape inside a box doesn't collide, a block sharing volume
    /// with one is no wall to suffocate in and doesn't act on the player inside it (cobwebs, powder
    /// snow, damaging blocks), and the server's check of the player's moves reads
    /// the same boxes. Entity colliders and the world border still stop it. Each call replaces
    /// the last set; an empty slice clears it, and so do a change of level, a respawn and
    /// leaving. Nothing is saved, and nothing reaches the player's client: its own half sets the
    /// same boxes with `ClientHost::set_collision_exclusions`. Refused, keeping the set there
    /// was: more than 64 boxes, a value that isn't finite, a minimum above its maximum, an
    /// entity that isn't a player, a stale handle.
    #[access(WorldOnly)]
    fn entity_set_collision_exclusions(
        &mut self,
        entity: EntityRef,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError>;
    /// Side: server.
    /// Sets the entity's no-physics flag (`Entity.noPhysics`), the value
    /// `entity_no_physics` reads: its moves pass through blocks. Refused, and
    /// nothing written, for a player (`Player.tick` rewrites the flag from the
    /// spectator state every tick), an item entity and a vex (`ItemEntity.tick`
    /// and `Vex.tick` both rewrite it every tick). An arrow's or a trident's
    /// write also updates the synced flag clients read. An armor stand
    /// recomputes the flag on load (`ArmorStand.readAdditionalSaveData`:
    /// `noPhysics = !hasPhysics()`), so its write is not persisted: it lasts
    /// only until the stand is saved and loaded again. The ender dragon, an
    /// area-effect cloud, an ominous item spawner and a shulker bullet take it
    /// too, from the `true` their constructors set. An entity Sunrise keeps
    /// unsimulated keeps the flag in its saved compound. False for those three
    /// kinds, a kind with no writable flag, a stale handle, or on a client.
    #[access(WorldOnly)]
    fn entity_set_no_physics(&mut self, entity: EntityRef, no_physics: bool) -> bool;
}
