//! The `should_cause_reequip_animation` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.shouldCauseReequipAnimation`: does the
    /// first-person hand lower and raise again to show `stack`, the stack the
    /// hand holds now, in place of `old`, the stack it shows? `slot` is the
    /// hand's `EquipmentSlot` (`MainHand` or `OffHand`). `slot_changed` is
    /// whether the selected hotbar slot differs from the one at the previous
    /// main-hand check that reached the Parts (always false for the off hand).
    ///
    /// Asked each client tick that the hands aren't busy, main hand then off
    /// hand, after vanilla's instant replacements
    /// (`FirstPersonHandsAndItems.shouldInstantlyReplaceVisibleItem`: the
    /// same stack changed in place, an equal one but for its damage, or an
    /// item whose model turns the swap animation off is shown at once). Both
    /// stacks are non-empty: an empty hand on either side lowers the hand
    /// without asking. Equal stacks are asked too. `reequip` arrives with
    /// vanilla's answer (whether the hand still shows another stack). `true`
    /// lowers the hand and raises it with `stack`; `false` shows `stack` at
    /// once. A `true` answer doesn't undo an instant replacement already made.
    /// Only the `ItemPart`s attached to `old`'s item are asked, each with the
    /// original inputs and the previous Part's `reequip`. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "listen",
        scope = "item",
        key = "old",
        event_per_listener = true,
        neoforge = "IItemExtension.shouldCauseReequipAnimation"
    )]
    fn should_cause_reequip_animation(
        stack: &ItemStack,
        old: &ItemStack,
        hand: Hand,
        slot: EquipmentSlot,
        slot_changed: bool,
        reequip: &mut bool,
    );
}
