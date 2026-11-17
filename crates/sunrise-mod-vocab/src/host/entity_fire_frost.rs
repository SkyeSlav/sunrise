//! The `entity_fire_frost` area: burning and freezing. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_fire_frost` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFireFrostBackend {
    /// Side: server.
    /// Whether the entity's type never burns (`EntityType.fireImmune`):
    /// players and mobs.
    #[access(WorldOnly)]
    fn entity_fire_immune(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The ticks the entity burns for (`Entity.getRemainingFireTicks`):
    /// players, mobs, armor stands and items.
    #[access(WorldOnly)]
    fn entity_fire_ticks(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// Returns the entity's frozen ticks (`Entity.getTicksFrozen`).
    #[access(WorldOnly)]
    fn entity_ticks_frozen(&self, entity: EntityRef) -> Option<i32>;
}
/// The handles' writes of the `entity_fire_frost` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFireFrostBackendMut {
    /// Side: server.
    /// Sets the remaining fire ticks (`Entity.setRemainingFireTicks`).
    #[access(WorldOnly)]
    fn entity_set_remaining_fire_ticks(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// Clears positive fire ticks while retaining fire immunity (`Entity.clearFire`).
    #[access(WorldOnly)]
    fn entity_clear_fire(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// `Entity.extinguishFire`: plays the ordinary extinguish sound when the
    /// entity is burning, then clears positive fire ticks. Fire-immune mobs
    /// and fire-resistant dropped stacks do not play a sound; negative
    /// fire-immunity countdowns are retained.
    /// Uses the engine's entity/cosmetic random and normal sound route.
    /// Supports players, guests, mobs, armor stands and dropped items. False
    /// for a stale or unsupported entity.
    /// Unlike this operation, `clear_fire` is always silent and draws no random.
    #[access(WorldOnly)]
    fn entity_extinguish_fire(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Sets the entity's frozen ticks (`Entity.setTicksFrozen`).
    #[access(WorldOnly)]
    fn entity_set_ticks_frozen(&mut self, entity: EntityRef, ticks: i32) -> bool;
}
