pub use client::environment_effects::{
    CUSTOM_CLOUDS_ATTRIBUTE, CUSTOM_SKYBOX_ATTRIBUTE, CUSTOM_WEATHER_EFFECTS_ATTRIBUTE,
    CloudStatus, CloudsRenderState, CustomCloudsRenderer, CustomSkyboxRenderer,
    CustomWeatherEffectRenderer, DEFAULT_CUSTOM_ENVIRONMENT_EFFECT, EnvironmentEffectKind,
    EnvironmentEffectRegistrationError, EnvironmentEffectRendererRegistration, SkyRenderState,
    WeatherRenderState,
};
mod __private {
    mod game {
        pub use crate::client::environment_effects::{
            EnvironmentEffectRendererImpl, environment_effect_registration_from_parts,
            environment_effect_registration_into_parts,
        };
    }
}
