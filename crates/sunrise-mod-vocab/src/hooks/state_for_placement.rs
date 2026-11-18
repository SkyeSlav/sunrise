//! The `state_for_placement` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0089

pub trait Hooks {
    /// A block is about to take the state it is placed in
    /// (`Block.getStateForPlacement(BlockPlaceContext)`), `state` the block's
    /// own default state. Answer only for your blocks: put the state you want
    /// in `answer` as `Some(Some(state))` — a state of that block, as vanilla
    /// sets properties on its own `defaultBlockState` (another block's is
    /// ignored) — or refuse the placement with `Some(None)`: the block goes
    /// nowhere and the stack is kept, as vanilla's `null`. Left empty, the
    /// block's own placement runs. `context` is
    /// vanilla's `BlockPlaceContext` (the clicked face, the placer's rotation,
    /// the clicked position, whether the clicked block is being replaced, the
    /// item in hand) and `fluid` the fluid in the position the block would go
    /// in, which a waterlogged block reads. Build the state from `state` with
    /// `native_state_with_property`, and know whether `state` is yours with
    /// `native_state_block`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`). The item's `must_survive` controls
    /// the ordinary final survival check. Standing/wall candidate survival and
    /// entity obstruction are independent. A trait mod returns
    /// `Some(Some(state))` to choose a state, `Some(None)` to refuse, or
    /// `None` to leave the answer to the next mod or vanilla. Changes to the event's fluid
    /// copy are mod-provided data forwarded to later Parts, never consumed by the engine.
    /// Asked on both sides, as vanilla asks it: a client asks it from its
    /// placement prediction, with its loaded copy of the level (match
    /// `LevelSide`). Reads a client can't answer, tag tests among them, are
    /// `None` there, and `BlockEntityPartState::state_in` gives its default:
    /// never answer from those as if they were the server's. A client refusal
    /// keeps its stack unspent until the server answers. Answer `None` on a
    /// client where only a server can decide: the template's rule then
    /// predicts, and the server still decides.
    /// Version 2 is catalog metadata; rebuild for the `LevelSide` signature.
    #[hook(
        side = "both",
        version = 2,
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        target = "sunrise_client::game::native_hooks::base::LevelView<'_>::native_state_for_placement",
        neoforge = "none (vanilla Block.getStateForPlacement; NeoForge adds no event or extension for it)"
    )]
    fn state_for_placement();
}
