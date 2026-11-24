use super::*;

impl GameLevel<'_> {
    /// `BlockItem.updatePlacementContext`, keyed by the placing item.
    pub(in crate::game) fn native_placement_context(
        &self,
        item: &str,
        context: &PlacementContext,
        answer: &mut Option<Option<mod_api::PlacementContext>>,
    ) {
        self.fire_placement_context(item, context, answer);
    }
}
