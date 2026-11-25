use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Once per world view after vanilla level state extraction (NeoForge's
    /// `ExtractLevelRenderStateEvent`).
    pub(in crate::game) fn native_extract_level_render_state(
        &self,
        view: &mod_api::WorldRenderView<'_>,
        state: &mut mod_api::LevelRenderState,
    ) {
        self.fire_extract_level_render_state(view, state);
    }
}
