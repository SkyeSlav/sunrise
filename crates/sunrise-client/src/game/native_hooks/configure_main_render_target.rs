use super::*;

impl Game<'_> {
    /// Ask client mods once at startup whether the main world render target needs a stencil
    /// buffer (NeoForge's `ConfigureMainRenderTargetEvent`).
    pub(in crate::game) fn native_configure_main_render_target(
        context: &mod_api::RegistrationContext,
        enable_stencil: &mut bool,
    ) {
        Self::fire_configure_main_render_target(context, enable_stencil);
    }
}
