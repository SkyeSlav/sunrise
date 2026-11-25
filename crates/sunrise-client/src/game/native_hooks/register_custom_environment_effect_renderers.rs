use super::*;

impl Game<'_> {
    /// Collect the mods' custom environment effect renderer registrations once at client startup
    /// (NeoForge `RegisterCustomEnvironmentEffectRendererEvent`).
    pub(in crate::game) fn native_register_custom_environment_effect_renderers(
        context: &mod_api::RegistrationContext,
        renderers: &mut Vec<mod_api::EnvironmentEffectRendererRegistration>,
    ) {
        Self::fire_register_custom_environment_effect_renderers(context, renderers);
    }
}
