//! The block item's pre-placement target rewrite.

pub trait Hooks {
    /// `BlockItem.updatePlacementContext`: replace `context` before selecting
    /// the block's placement state, on both sides with a read-only level host.
    /// `Some(Some(context))` rewrites the target and click, `Some(None)` refuses
    /// placement, and `None` keeps the item's vanilla rewrite (including
    /// scaffolding's column walk). The placing item cannot be changed.
    /// Non-finite click or view values are refused. The rewritten target must
    /// be inside the level's bounds; its replaceability is not rechecked, as
    /// vanilla checks the original context before rewriting. This does not bypass
    /// survival or obstruction checks; see `must_survive` for the former.
    /// Dispatch is by `item` id. This event has no stack: the decision is from
    /// the item and level, not held-stack components.
    #[hook(
        side = "both",
        contract = "replace",
        scope = "item",
        key = "item",
        out = "answer",
        target = "sunrise_client::game::native_hooks::placement_context::GameLevel<'_>::native_placement_context",
        neoforge = "none (vanilla BlockItem.updatePlacementContext)"
    )]
    fn placement_context();
}
