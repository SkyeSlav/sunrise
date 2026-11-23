//! The `living_breathe` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's breathing this tick (`LivingBreatheEvent`, posted from
    /// `LivingEntity.baseTick` every tick for a living entity with the generic
    /// air logic): `entity` is the player or mob. `can_breathe`, `consume` and
    /// `refill` come in as Sunrise does it (a mod-free game changes nothing); set
    /// them to change this tick. With `can_breathe` the air rises by `refill`, up
    /// to its maximum (`entity_max_air_supply`: 300 for every entity except
    /// dolphins, which have 4800); a dolphin's vanilla `refill` is its maximum
    /// minus its current air. Without breath, air falls by `consume`, and at -20
    /// it is set to 0 and the entity takes 2 drowning damage. Negative amounts
    /// count as 0. Asked on servers for players and mobs. A water animal,
    /// axolotl or nautilus comes in with its own air logic's answer: where it
    /// breathes (water, or rain for an axolotl) `can_breathe` and a `refill` to
    /// its full air; elsewhere no breath and `consume` 1. Running out deals its
    /// own damage (drowning, or drying out).
    ///
    /// The listener has the entity's level and can read its air
    /// (`entity_air_supply`), its equipment and the block it stands in, and
    /// write items, attachments and criteria, each at once as in NeoForge. The game
    /// then applies the answer to the air the entity had when the listener was called:
    /// if the entity has gone, died or had its air changed in the meantime, it refuses
    /// (never recomputes) and the air is left as the listener left it; what the
    /// listener wrote stands. Mod effects that this tick's air update causes
    /// (drowning, drying out) are not part of the call.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "LivingBreatheEvent",
        cancellable = false,
        version = 4
    )]
    fn living_breathe(
        entity: mod_api::LivingRef,
        can_breathe: &mut bool,
        consume: &mut i32,
        refill: &mut i32,
    );
}
