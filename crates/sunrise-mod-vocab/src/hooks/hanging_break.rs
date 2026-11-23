//! The `hanging_break` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0244

pub trait Hooks {
    /// An item frame (`minecraft:item_frame`, `minecraft:glow_item_frame`) or
    /// a painting (`minecraft:painting`), `kind` its entity type and `pos` the
    /// block it hangs in, is about to break: a server player's hit
    /// (`player`, their state swapped in, so `PlayerRef::tell` answers them), a
    /// blast, or the wall it hangs on gone (`player` None for both). Where
    /// `BlockAttachedEntity.hurtServer` and its survival check would kill
    /// it, before anything drops or sounds. Set `cancel` and it stays, as it
    /// was. A hit that only takes the item out of a frame does not break the
    /// frame and is not asked. The game's own player's hit is not asked, nor
    /// is `/kill`. The listener may change the world: straight after it returns
    /// without cancelling, the game finds the frame or painting again by its entity
    /// id; if it is gone, or an empty frame being broken now holds an item, nothing
    /// breaks and nothing is removed. `dimension` is the level the frame or
    /// painting hangs in (`Entity.level().dimension()`: the level, not its
    /// dimension type).
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_hanging_break",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla hanging entity breaking; NeoForge adds no event or extension for it); AttackEntityEvent fires for players only"
    )]
    fn hanging_break(
        dimension: LevelDimension,
        player: Option<u128>,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    );
}
