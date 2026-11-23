//! The `block_landing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity landed on a block state this part is attached to (`Block.fallOn`), after falling `event.distance` blocks. Writable `event.distance` and `event.damage_multiplier` feed the fall calculation before hurt and sounds. The multiplier starts at the block's vanilla value (normally 1.0). Damage is `floor((distance + 1e-6 - safe_fall_distance) * damage_multiplier * fall_damage_multiplier_attribute)`, with safe fall distance 3 by default. Halving distance is not halving damage. Zero multiplier or a distance within the safe fall prevents damage and generic fall sounds. An entity that cannot be hurt by the fall (creative, fall immune) is told with zero multiplier and keeps its immunity, and a cancelled fall still lands. Non-finite inputs are ignored and negative multipliers clamp to zero. Runs synchronously on `&mut dyn ServerHost` with the live `event.level` and `event.entity` before `causeFallDamage` and `HIT_GROUND`, so block writes, sound plays, game events, random draws and entity edits take effect immediately and are visible to the rest of this landing and later entities in the same tick. Players, mobs and dropped items land here (an item with zero multiplier: items take no fall damage); projectiles and the other entities do not.
    #[hook(
        side = "server",
        contract = "collect",
        out = "damage_multiplier",
        scope = "block",
        key = "state",
        target = "sunrise_client::game::native_hooks::block_landing::Game<'_>::native_fall_on",
        neoforge = "LivingFallEvent",
        version = 3
    )]
    fn fall_on(
        level: LevelRef,
        entity: EntityRef,
        state: BlockState,
        pos: BlockPos,
        distance: &mut f64,
        damage_multiplier: &mut f32,
    );
    /// An entity bounced off a block state this part is attached to (`Block.bounceOn`). `event.fall_distance` is the distance stored after fall handling, so a grounded boat can report zero. It fires only when the entity bounced, synchronously on `&mut dyn ServerHost` with the live `event.level` and `event.entity` right before `GameEvent.BOUNCE` and `Entity.setDeltaMovement`. `event.movement` is the movement the bounce leaves it (`x`, `y`, `z`): change it to change what the entity keeps; non-finite answers are ignored. Runs on the server for players, mobs and other bodies that bounce in the base move.
    #[hook(
        side = "server",
        contract = "collect",
        out = "movement",
        scope = "block",
        key = "state",
        target = "sunrise_client::game::native_hooks::block_landing::Game<'_>::native_bounce_on",
        neoforge = "none (vanilla Block.bounceOn; IBlockExtension.getBounceRestitution only returns a factor)",
        version = 3
    )]
    fn bounce_on(
        level: LevelRef,
        entity: EntityRef,
        state: BlockState,
        pos: BlockPos,
        fall_distance: f64,
        movement: &mut [f64; 3],
    );
}
