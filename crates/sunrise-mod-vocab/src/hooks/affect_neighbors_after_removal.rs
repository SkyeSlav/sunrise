//! The `affect_neighbors_after_removal` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Block.affectNeighborsAfterRemoval` for a mod's block: an authoritative server removed the
    /// block `state` from `pos` (`moved`: a piston pushed it) and has run its template class's own
    /// updates. The listener has the level's write host at the removal site and tells the neighbours
    /// itself, as the block's own override would: `update_neighbors` with
    /// `NeighbourUpdate::OutputSignal` (`updateNeighbourForOutputSignal`), `Neighbours`
    /// (`updateNeighborsAt`) or `NeighboursExcept(direction)` (`updateNeighborsAtExceptFromFacing`).
    /// Each call runs at once, in call order, through the level's normal update application, and
    /// updates it causes recurse as they do in vanilla. `ticks` is the removal's.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        version = 2,
        target = "sunrise_client::game::native_hooks::affect_neighbors_after_removal::GameLevel<'_>::native_affect_neighbors_after_removal",
        neoforge = "none (vanilla Block.affectNeighborsAfterRemoval; NeoForge adds no event or extension for it)"
    )]
    fn affect_neighbors_after_removal(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        moved: bool,
        #[engine] ticks: &mut Ticks,
    );
}
