//! The `can_place_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Container.canPlaceItem`: whether `slot` of this mod block entity's container admits
    /// the offered nonempty `stack`. `Some(true)` or `Some(false)` replaces the declared
    /// `ModContainer::insert_rules` answer; `None` keeps it. A subscribed slot refuses
    /// insertion if the entity snapshot or stack cannot be represented. Face, input-slot
    /// and stack limits still apply.
    ///
    /// This is a read-only query on the entity's saved data and current slots, with no Host.
    /// Hoppers, droppers, default `ITEMS` and `STORAGE` inserts, and nonempty `/loot insert`
    /// and `/loot replace block` offers ask it. Inserts query with the remaining offer
    /// before clamping to room. Default `STORAGE.space_for` does not ask this hook: for a
    /// subscribed slot it bounds insertion by physical room after face and input checks.
    /// Menus use `menu_slot_rules` instead (`Slot.mayPlace`).
    ///
    /// Seed a filter from an accepted write, not this query. Default capability writes to
    /// mod-owned storage do not emit `contents_changed`; use an explicit `set_changed`
    /// notice or seed in a provider that owns the accepted write.
    #[hook(
        side = "server",
        contract = "replace",
        out = "allowed",
        scope = "block_entity",
        key = "entity",
        non_exhaustive = true,
        neoforge = "none (vanilla Container.canPlaceItem; the item capability wraps this decision)"
    )]
    fn can_place_item();
}
