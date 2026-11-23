//! The `dispenser_acts` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `DispenserBlock.getDispenseMethod`, for every mod at once: a dispenser (not a dropper)
    /// on the authoritative server has picked a non-empty slot (`DispenserBlockEntity.getRandomSlot`)
    /// and is about to choose what to do with `stack`. Subscribe once; no item or block key is
    /// needed. An empty dispenser, or one with no block entity, asks no one. A stack the world's
    /// feature flags disable is dropped as vanilla drops it, and no one is asked.
    ///
    /// The first answer the game accepts wins: the faced block's `dispense_method`, the item's
    /// `dispense_item` and vanilla's behaviour don't run. With no accepted answer they run in
    /// that order, as before. The answer is theirs: a remainder of None empties the slot,
    /// `Some(stack)` replaces it (the hook owns consumption: no extra item is removed
    /// afterwards), and `event` is the level event played with the smoke: 1000 the click
    /// (`DefaultDispenseItemBehavior.playDefaultSound`), 1001 the fail sound
    /// (`OptionalDispenseItemBehavior.playSound`), or 0 for no sound or smoke. Answer the
    /// original stack with 0 to do nothing, like vanilla's `DispenseItemBehavior.NOOP`; to drop
    /// one normally, answer with `host.dispense_default`'s remainder and 1000.
    ///
    /// Another event, or a remainder the game can't hold, is refused with a warning naming the
    /// mod, and the next mod is asked; only the answer is refused, not what the listener already
    /// did through the `Host`. In an open transaction in the dispenser's level, sound and smoke
    /// follow the behaviour's published effects at the outermost commit and are discarded on
    /// abort.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_dispenser_acts",
        dispatch = "direct",
        engine_accepts = true,
        side = "server",
        contract = "replace",
        out = "answer",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event for it, only the item-keyed DispenserBlock.registerBehavior)"
    )]
    fn dispenser_acts(
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
    );
}
