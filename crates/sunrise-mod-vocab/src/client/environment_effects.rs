//! Custom environment effect renderers (NeoForge 26.3's
//! `RegisterCustomEnvironmentEffectRendererEvent`, `CustomSkyboxRenderer`, `CustomCloudsRenderer`,
//! `CustomWeatherEffectRenderer`, and `NeoForgeEnvironmentAttributes`).
//!
//! Environment effect renderers read only extracted per-view state ([`WorldRenderView`] and its
//! [`RenderDataKey`](crate::RenderDataKey) entries, plus [`SkyRenderState`],
//! [`CloudsRenderState`], or [`WeatherRenderState`]). Live world queries through `ClientHost` are
//! not available during drawing; extract any needed world data in
//! `Hooks::extract_level_render_state`.

use crate::{Identifier, WorldDrawContext, WorldRenderView};
use std::fmt;

/// The built-in `neoforge:custom_skybox` environment attribute identifier
/// (`NeoForgeEnvironmentAttributes.CUSTOM_SKYBOX`).
pub const CUSTOM_SKYBOX_ATTRIBUTE: &str = "neoforge:custom_skybox";

/// The built-in `neoforge:custom_clouds` environment attribute identifier
/// (`NeoForgeEnvironmentAttributes.CUSTOM_CLOUDS`).
pub const CUSTOM_CLOUDS_ATTRIBUTE: &str = "neoforge:custom_clouds";

/// The built-in `neoforge:custom_weather_effects` environment attribute identifier
/// (`NeoForgeEnvironmentAttributes.CUSTOM_WEATHER_EFFECTS`).
pub const CUSTOM_WEATHER_EFFECTS_ATTRIBUTE: &str = "neoforge:custom_weather_effects";

/// The built-in default renderer identifier (`minecraft:default`) that cannot be overridden
/// (`NeoForgeEnvironmentAttributes.DEFAULT_CUSTOM_{SKYBOX,CLOUDS,WEATHER_EFFECTS}`).
pub const DEFAULT_CUSTOM_ENVIRONMENT_EFFECT: &str = "minecraft:default";

/// Cloud rendering quality setting (`net.minecraft.client.CloudStatus`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// no wire: client-only cloud rendering status.
pub enum CloudStatus {
    /// Clouds are disabled (`CloudStatus.OFF`).
    Off,
    /// Flat 2D clouds (`CloudStatus.FAST`).
    Fast,
    /// Extruded 3D clouds (`CloudStatus.FANCY`).
    #[default]
    Fancy,
}

/// Extracted sky state for the active view (`net.minecraft.client.renderer.state.level.SkyRenderState`),
/// passed to [`CustomSkyboxRenderer::render_sky`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
// no wire: client-only per-view extracted sky state.
pub struct SkyRenderState {
    /// Sampled sky RGB colour (`minecraft:visual/sky_color`).
    pub sky_color: [f32; 3],
    /// Sampled sunrise/sunset RGBA colour (`minecraft:visual/sunrise_sunset_color`).
    pub sunrise_color: [f32; 4],
    /// Sampled environmental fog RGB colour (`minecraft:visual/fog_color`).
    pub fog_color: [f32; 3],
    /// Sampled sun angle in degrees (`minecraft:visual/sun_angle`).
    pub sun_angle: f32,
    /// Sampled moon angle in degrees (`minecraft:visual/moon_angle`).
    pub moon_angle: f32,
    /// Sampled star angle in degrees (`minecraft:visual/star_angle`).
    pub star_angle: f32,
    /// Sampled star brightness in `0.0..=1.0` (`minecraft:visual/star_brightness`).
    pub star_brightness: f32,
    /// Interpolated rain level in `0.0..=1.0`.
    pub rain_level: f32,
    /// Interpolated thunder level in `0.0..=1.0`.
    pub thunder_level: f32,
    /// Sky fog end distance (`minecraft:visual/sky_fog_end_distance`).
    pub sky_fog_end: f32,
    /// Whether the dimension's vanilla skybox is the End sky.
    pub is_end_sky: bool,
}

/// Extracted cloud state for the active view, passed to [`CustomCloudsRenderer::render_clouds`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
// no wire: client-only per-view extracted cloud state.
pub struct CloudsRenderState {
    /// Active cloud quality setting (`CloudStatus`).
    pub status: CloudStatus,
    /// Sampled cloud RGBA colour (`minecraft:visual/cloud_color`).
    pub color: [f32; 4],
    /// Sampled cloud layer Y height (`minecraft:visual/cloud_height`).
    pub height: f32,
    /// Cloud render range in chunks (`Options.cloudRange`).
    pub range: u32,
    /// Cloud fog end distance (`minecraft:visual/cloud_fog_end_distance`).
    pub cloud_fog_end: f32,
    /// World game time in ticks.
    pub game_time: i64,
    /// Sub-tick interpolation fraction in `[0.0, 1.0)`.
    pub partial_tick: f32,
}

/// Extracted weather state for the active view
/// (`net.minecraft.client.renderer.state.level.WeatherRenderState`), passed to
/// [`CustomWeatherEffectRenderer::render_weather`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
// no wire: client-only per-view extracted weather state.
pub struct WeatherRenderState {
    /// Interpolated rain level in `0.0..=1.0`.
    pub rain_level: f32,
    /// Interpolated thunder level in `0.0..=1.0`.
    pub thunder_level: f32,
    /// Weather effect radius in blocks (`Options.weatherRadius`).
    pub radius: i32,
    /// World game time in ticks.
    pub game_time: i64,
    /// Sub-tick interpolation fraction in `[0.0, 1.0)`.
    pub partial_tick: f32,
}

/// Custom skybox renderer registered via
/// [`RegisterCustomEnvironmentEffectRenderersEvent::register_skybox`] and selected by the
/// `neoforge:custom_skybox` environment attribute (NeoForge `CustomSkyboxRenderer`).
///
/// Any world data needed during drawing must be extracted in `Hooks::extract_level_render_state`
/// and read via [`WorldRenderView::render_data`]; no live `ClientHost` world queries are
/// available here.
pub trait CustomSkyboxRenderer: Send + Sync + 'static {
    /// Render the custom skybox for the active view.
    ///
    /// Return `true` to suppress vanilla skybox rendering for this view, or `false` to let
    /// vanilla draw as well (matching NeoForge `CustomSkyboxRenderer.renderSky`).
    fn render_sky(
        &self,
        view: &WorldRenderView<'_>,
        sky: &SkyRenderState,
        draw: &mut dyn WorldDrawContext,
    ) -> bool;
}

/// Custom clouds renderer registered via
/// [`RegisterCustomEnvironmentEffectRenderersEvent::register_clouds`] and selected by the
/// `neoforge:custom_clouds` environment attribute (NeoForge `CustomCloudsRenderer`).
///
/// Any world data needed during drawing must be extracted in `Hooks::extract_level_render_state`
/// and read via [`WorldRenderView::render_data`]; no live `ClientHost` world queries are
/// available here.
pub trait CustomCloudsRenderer: Send + Sync + 'static {
    /// Render custom clouds for the active view.
    ///
    /// Return `true` to suppress vanilla cloud rendering for this view, or `false` to let
    /// vanilla draw as well (matching NeoForge `CustomCloudsRenderer.renderClouds`).
    fn render_clouds(
        &self,
        view: &WorldRenderView<'_>,
        clouds: &CloudsRenderState,
        draw: &mut dyn WorldDrawContext,
    ) -> bool;
}

/// Custom snow and rain renderer registered via
/// [`RegisterCustomEnvironmentEffectRenderersEvent::register_weather`] and selected by the
/// `neoforge:custom_weather_effects` environment attribute
/// (NeoForge `CustomWeatherEffectRenderer`).
///
/// Any world data needed during drawing must be extracted in `Hooks::extract_level_render_state`
/// and read via [`WorldRenderView::render_data`]; no live `ClientHost` world queries are
/// available here.
pub trait CustomWeatherEffectRenderer: Send + Sync + 'static {
    /// Render custom snow and rain effects for the active view.
    ///
    /// Return `true` to suppress vanilla snow and rain rendering for this view, or `false` to
    /// let vanilla draw as well (matching NeoForge
    /// `CustomWeatherEffectRenderer.renderSnowAndRain`).
    fn render_weather(
        &self,
        view: &WorldRenderView<'_>,
        weather: &WeatherRenderState,
        draw: &mut dyn WorldDrawContext,
    ) -> bool;
}

/// Which environment effect slot a custom renderer targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// no wire: client-only registration slot discriminator.
pub enum EnvironmentEffectKind {
    /// Custom skybox (`neoforge:custom_skybox`).
    Skybox,
    /// Custom clouds (`neoforge:custom_clouds`).
    Clouds,
    /// Custom snow and rain effects (`neoforge:custom_weather_effects`).
    Weather,
}

impl fmt::Display for EnvironmentEffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Skybox => "skybox",
            Self::Clouds => "clouds",
            Self::Weather => "weather",
        })
    }
}

/// Error returned when a custom environment effect renderer registration is refused immediately.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client startup registration error.
pub enum EnvironmentEffectRegistrationError {
    /// Attempted to register a renderer under the reserved default identifier
    /// (`minecraft:default`).
    DefaultId {
        /// Which environment effect slot was targeted.
        kind: EnvironmentEffectKind,
        /// The refused identifier.
        id: Identifier,
    },
}

impl fmt::Display for EnvironmentEffectRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultId { kind, id } => write!(
                f,
                "cannot register a custom {kind} renderer for the default id {id}"
            ),
        }
    }
}

impl std::error::Error for EnvironmentEffectRegistrationError {}

// hidden: engine-internal renderer payload unpacked at client startup freeze.
// no wire: owns a process-local renderer trait object.
#[doc(hidden)]
pub enum EnvironmentEffectRendererImpl {
    /// Custom skybox renderer payload.
    Skybox(Box<dyn CustomSkyboxRenderer>),
    /// Custom clouds renderer payload.
    Clouds(Box<dyn CustomCloudsRenderer>),
    /// Custom weather renderer payload.
    Weather(Box<dyn CustomWeatherEffectRenderer>),
}

// no wire: owns a process-local renderer trait object.
/// One attributed custom environment effect renderer staged during
/// `Hooks::register_custom_environment_effect_renderers`.
// exhaustive: private fields keep renderer ownership and payload opaque.
pub struct EnvironmentEffectRendererRegistration {
    owner: &'static str,
    id: Identifier,
    renderer: EnvironmentEffectRendererImpl,
}

impl EnvironmentEffectRendererRegistration {
    /// The registering mod's identifier.
    pub fn owner(&self) -> &'static str {
        self.owner
    }

    /// The custom environment effect identifier this renderer is bound to.
    pub fn id(&self) -> &Identifier {
        &self.id
    }

    /// Which environment effect slot this registration targets.
    pub fn kind(&self) -> EnvironmentEffectKind {
        match self.renderer {
            EnvironmentEffectRendererImpl::Skybox(_) => EnvironmentEffectKind::Skybox,
            EnvironmentEffectRendererImpl::Clouds(_) => EnvironmentEffectKind::Clouds,
            EnvironmentEffectRendererImpl::Weather(_) => EnvironmentEffectKind::Weather,
        }
    }
}

// hidden: engine-internal unpacker used when freezing registrations at client startup.
#[doc(hidden)]
pub fn environment_effect_registration_into_parts(
    registration: EnvironmentEffectRendererRegistration,
) -> (&'static str, Identifier, EnvironmentEffectRendererImpl) {
    (registration.owner, registration.id, registration.renderer)
}

// hidden: engine/test constructor for building attributed registrations directly.
#[doc(hidden)]
pub fn environment_effect_registration_from_parts(
    owner: &'static str,
    id: Identifier,
    renderer: EnvironmentEffectRendererImpl,
) -> EnvironmentEffectRendererRegistration {
    EnvironmentEffectRendererRegistration {
        owner,
        id,
        renderer,
    }
}

impl crate::RegisterCustomEnvironmentEffectRenderersEvent<'_> {
    /// Register a custom skybox renderer for `id` (NeoForge `registerSkyboxRenderer`),
    /// referenced by the `neoforge:custom_skybox` environment attribute.
    ///
    /// Refuses `minecraft:default` immediately with [`EnvironmentEffectRegistrationError::DefaultId`].
    /// Duplicate IDs across mods or listeners fail client startup naming both mods.
    pub fn register_skybox(
        &mut self,
        id: Identifier,
        renderer: impl CustomSkyboxRenderer,
    ) -> Result<(), EnvironmentEffectRegistrationError> {
        if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
            return Err(EnvironmentEffectRegistrationError::DefaultId {
                kind: EnvironmentEffectKind::Skybox,
                id,
            });
        }
        self.renderers.push(EnvironmentEffectRendererRegistration {
            owner: self.mod_id,
            id,
            renderer: EnvironmentEffectRendererImpl::Skybox(Box::new(renderer)),
        });
        Ok(())
    }

    /// Register a custom clouds renderer for `id` (NeoForge `registerCloudRenderer`),
    /// referenced by the `neoforge:custom_clouds` environment attribute.
    ///
    /// Refuses `minecraft:default` immediately with [`EnvironmentEffectRegistrationError::DefaultId`].
    /// Duplicate IDs across mods or listeners fail client startup naming both mods.
    pub fn register_clouds(
        &mut self,
        id: Identifier,
        renderer: impl CustomCloudsRenderer,
    ) -> Result<(), EnvironmentEffectRegistrationError> {
        if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
            return Err(EnvironmentEffectRegistrationError::DefaultId {
                kind: EnvironmentEffectKind::Clouds,
                id,
            });
        }
        self.renderers.push(EnvironmentEffectRendererRegistration {
            owner: self.mod_id,
            id,
            renderer: EnvironmentEffectRendererImpl::Clouds(Box::new(renderer)),
        });
        Ok(())
    }

    /// Register a custom snow and rain renderer for `id`
    /// (NeoForge `registerWeatherEffectRenderer`), referenced by the
    /// `neoforge:custom_weather_effects` environment attribute.
    ///
    /// Refuses `minecraft:default` immediately with [`EnvironmentEffectRegistrationError::DefaultId`].
    /// Duplicate IDs across mods or listeners fail client startup naming both mods.
    pub fn register_weather(
        &mut self,
        id: Identifier,
        renderer: impl CustomWeatherEffectRenderer,
    ) -> Result<(), EnvironmentEffectRegistrationError> {
        if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
            return Err(EnvironmentEffectRegistrationError::DefaultId {
                kind: EnvironmentEffectKind::Weather,
                id,
            });
        }
        self.renderers.push(EnvironmentEffectRendererRegistration {
            owner: self.mod_id,
            id,
            renderer: EnvironmentEffectRendererImpl::Weather(Box::new(renderer)),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, LevelRenderState, LevelRenderStateWrite, RenderDataKey};
    use std::sync::{Arc, Mutex};

    struct DummySky(Arc<Mutex<Option<u32>>>);
    impl CustomSkyboxRenderer for DummySky {
        fn render_sky(
            &self,
            view: &WorldRenderView<'_>,
            sky: &SkyRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            static KEY: RenderDataKey<u32> = RenderDataKey::new("example:sky_phase");
            *self.0.lock().unwrap() = view.render_data(&KEY).copied();
            sky.sun_angle >= 0.0
        }
    }

    struct DummyClouds;
    impl CustomCloudsRenderer for DummyClouds {
        fn render_clouds(
            &self,
            _view: &WorldRenderView<'_>,
            _clouds: &CloudsRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            true
        }
    }

    struct DummyWeather;
    impl CustomWeatherEffectRenderer for DummyWeather {
        fn render_weather(
            &self,
            _view: &WorldRenderView<'_>,
            _weather: &WeatherRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            false
        }
    }

    struct NoopDraw;
    impl WorldDrawContext for NoopDraw {}

    #[test]
    fn refuses_default_id_and_attributes_valid_registrations() {
        let mut renderers = Vec::new();
        let mut event = crate::RegisterCustomEnvironmentEffectRenderersEvent {
            mod_id: "aurora",
            renderers: &mut renderers,
        };
        let default_id = Identifier::parse(DEFAULT_CUSTOM_ENVIRONMENT_EFFECT).unwrap();
        assert_eq!(
            event.register_skybox(default_id.clone(), DummySky(Arc::new(Mutex::new(None)))),
            Err(EnvironmentEffectRegistrationError::DefaultId {
                kind: EnvironmentEffectKind::Skybox,
                id: default_id.clone(),
            })
        );
        assert_eq!(
            event.register_clouds(default_id.clone(), DummyClouds),
            Err(EnvironmentEffectRegistrationError::DefaultId {
                kind: EnvironmentEffectKind::Clouds,
                id: default_id.clone(),
            })
        );
        assert_eq!(
            event.register_weather(default_id.clone(), DummyWeather),
            Err(EnvironmentEffectRegistrationError::DefaultId {
                kind: EnvironmentEffectKind::Weather,
                id: default_id,
            })
        );
        assert!(renderers.is_empty());

        let observed = Arc::new(Mutex::new(None));
        event
            .register_skybox(
                Identifier::parse("aurora:sky").unwrap(),
                DummySky(Arc::clone(&observed)),
            )
            .unwrap();
        event
            .register_clouds(Identifier::parse("aurora:clouds").unwrap(), DummyClouds)
            .unwrap();
        event
            .register_weather(Identifier::parse("aurora:weather").unwrap(), DummyWeather)
            .unwrap();

        assert_eq!(renderers.len(), 3);
        assert_eq!(renderers[0].owner(), "aurora");
        assert_eq!(renderers[0].id().as_str(), "aurora:sky");
        assert_eq!(renderers[0].kind(), EnvironmentEffectKind::Skybox);
        assert_eq!(renderers[1].kind(), EnvironmentEffectKind::Clouds);
        assert_eq!(renderers[2].kind(), EnvironmentEffectKind::Weather);

        // Verify extracted-state-only contract: sky renderer reads RenderDataKey from view
        // without needing any ClientHost.
        static KEY: RenderDataKey<u32> = RenderDataKey::new("example:sky_phase");
        let mut state = LevelRenderState::new();
        let writer: &mut dyn LevelRenderStateWrite = &mut state;
        writer.set_render_data(&KEY, 42u32);
        let camera = Camera {
            position: [0.0, 64.0, 0.0],
            eye_height: 1.62,
            partial_tick: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: [0.0; 16],
            projection: [0.0; 16],
            dimension: "minecraft:overworld".into(),
        };
        let view = WorldRenderView::new(&camera, 0.5, 100, false, &[]).__with_render_data(&state);
        let mut draw = NoopDraw;
        let (_, _, payload) = environment_effect_registration_into_parts(renderers.remove(0));
        let EnvironmentEffectRendererImpl::Skybox(sky_renderer) = payload else {
            panic!("expected skybox payload");
        };
        assert!(sky_renderer.render_sky(&view, &SkyRenderState::default(), &mut draw));
        assert_eq!(*observed.lock().unwrap(), Some(42));
    }
}
