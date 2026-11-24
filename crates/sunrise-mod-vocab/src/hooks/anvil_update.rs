//! The `anvil_update` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An anvil at `pos` made vanilla's result with a left input, for
    /// `player` (UUID) and `player_entity_id` (`AnvilUpdateEvent`, matching
    /// NeoForge `AnvilMenu.createResult` / `CommonHooks.onAnvilUpdate`), live
    /// on every `createResult` with no answer cache: `inputs` lends the
    /// stacks, rename text and `vanilla_result` (its `output` and costs, and the
    /// `uncapped_output` and `unclamped_xp_cost` vanilla computed before its
    /// 40-level drop and rename-only clamp); `edits` starts with `Keep`,
    /// vanilla's costs and `too_expensive` (vanilla's test: a cost of 40 or
    /// more without infinite materials). Set an output or edit the costs
    /// without changing the inputs. `too_expensive` decides the server's drop
    /// of the result and the client's "Too Expensive!" label; taking stays
    /// decided by `AnvilMenu.mayPickup`, with no 40 check.
    /// Cancellation clears the result and both costs. An invalid
    /// replacement output is logged and ignored (keeping vanilla's output while
    /// still applying the cost edits). `edits.xp_cost` stays signed during
    /// listener dispatch and clamps to `0` when applied (`AnvilMenu.setCost`).
    /// Both sides, including the update after material consumption with the
    /// left slot still present. On the server that update observes the player
    /// after `AnvilMenu.onTake` has charged the levels.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilUpdateEvent",
        cancellable = true,
        version = 4
    )]
    fn anvil_update(
        player: u128,
        player_entity_id: i32,
        pos: [i32; 3],
        inputs: &mod_api::item::menu::AnvilUpdateInputs<'_>,
        edits: &mut mod_api::item::menu::AnvilUpdateEdits,
        cancel: &mut bool,
    );
}
