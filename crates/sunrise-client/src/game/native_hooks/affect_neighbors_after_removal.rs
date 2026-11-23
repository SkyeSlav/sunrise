use super::*;

impl GameLevel<'_> {
    /// `Block.affectNeighborsAfterRemoval` for a mod's block: an authoritative server removed the
    /// block `state` from `pos` (`moved`: a piston pushed it) and has run its template class's own
    /// updates. The listener tells the neighbours itself through the level's write host, each
    /// update applied at once, as the block's own override would; `ticks` is the removal's.
    #[inline(always)]
    pub(in crate::game) fn native_affect_neighbors_after_removal(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        moved: bool,
    ) {
        self.fire_affect_neighbors_after_removal(ticks, dimension, pos, state, moved);
    }
}
