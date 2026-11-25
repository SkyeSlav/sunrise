//! Startup registry of custom environment effect renderers
//! (`Hooks::register_custom_environment_effect_renderers`, NeoForge 26.3's
//! `RegisterCustomEnvironmentEffectRendererEvent` and `CustomEnvironmentEffectsRendererManager`).
//!
//! Registrations are validated and frozen once at client startup and survive resource reloads and
//! world changes. Renderers receive only extracted per-view state (`WorldRenderView` +
//! `SkyRenderState`/`CloudsRenderState`/`WeatherRenderState`) and draw through `WorldDrawContext`;
//! no live `ClientHost` world queries are available during drawing.

use mod_api::{
    __private::game::{
        CallbackOwner, EnvironmentEffectRendererImpl, environment_effect_registration_into_parts,
    },
    CloudsRenderState, CustomCloudsRenderer, CustomSkyboxRenderer, CustomWeatherEffectRenderer,
    DEFAULT_CUSTOM_ENVIRONMENT_EFFECT, EnvironmentEffectKind,
    EnvironmentEffectRendererRegistration, Identifier, SkyRenderState, WeatherRenderState,
    WorldDrawContext, WorldRenderView,
};
use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
    sync::OnceLock,
};

/// One frozen custom skybox renderer with its owning mod's panic quarantine.
pub struct RegisteredSkyboxRenderer {
    owner: CallbackOwner,
    renderer: Box<dyn CustomSkyboxRenderer>,
}

impl RegisteredSkyboxRenderer {
    pub fn owner(&self) -> &'static str {
        self.owner.id()
    }

    /// Invoke the custom skybox renderer under its owning mod's panic quarantine.
    /// Returns `true` to suppress vanilla skybox drawing; a panic disables this renderer for the
    /// session and falls back to vanilla (`false`).
    pub fn render_sky(
        &self,
        view: &WorldRenderView<'_>,
        sky: &SkyRenderState,
        draw: &mut dyn WorldDrawContext,
    ) -> bool {
        if !self.owner.is_enabled() {
            return false;
        }
        self.owner
            .call(
                "custom skybox renderer",
                "custom_skybox_renderer_draw",
                || self.renderer.render_sky(view, sky, draw),
            )
            .unwrap_or(false)
    }
}

/// One frozen custom clouds renderer with its owning mod's panic quarantine.
pub struct RegisteredCloudsRenderer {
    owner: CallbackOwner,
    renderer: Box<dyn CustomCloudsRenderer>,
}

impl RegisteredCloudsRenderer {
    pub fn owner(&self) -> &'static str {
        self.owner.id()
    }

    /// Invoke the custom clouds renderer under its owning mod's panic quarantine.
    /// Returns `true` to suppress vanilla cloud drawing; a panic disables this renderer for the
    /// session and falls back to vanilla (`false`).
    pub fn render_clouds(
        &self,
        view: &WorldRenderView<'_>,
        clouds: &CloudsRenderState,
        draw: &mut dyn WorldDrawContext,
    ) -> bool {
        if !self.owner.is_enabled() {
            return false;
        }
        self.owner
            .call(
                "custom clouds renderer",
                "custom_clouds_renderer_draw",
                || self.renderer.render_clouds(view, clouds, draw),
            )
            .unwrap_or(false)
    }
}

/// One frozen custom snow and rain renderer with its owning mod's panic quarantine.
pub struct RegisteredWeatherRenderer {
    owner: CallbackOwner,
    renderer: Box<dyn CustomWeatherEffectRenderer>,
}

impl RegisteredWeatherRenderer {
    pub fn owner(&self) -> &'static str {
        self.owner.id()
    }

    /// Invoke the custom weather renderer under its owning mod's panic quarantine.
    /// Returns `true` to suppress vanilla snow and rain drawing; a panic disables this renderer
    /// for the session and falls back to vanilla (`false`).
    pub fn render_weather(
        &self,
        view: &WorldRenderView<'_>,
        weather: &WeatherRenderState,
        draw: &mut dyn WorldDrawContext,
    ) -> bool {
        if !self.owner.is_enabled() {
            return false;
        }
        self.owner
            .call(
                "custom weather effect renderer",
                "custom_weather_effect_renderer_draw",
                || self.renderer.render_weather(view, weather, draw),
            )
            .unwrap_or(false)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegistrationError {
    DefaultId {
        owner: &'static str,
        kind: EnvironmentEffectKind,
        id: Identifier,
    },
    DuplicateId {
        kind: EnvironmentEffectKind,
        id: Identifier,
        earlier: &'static str,
        owner: &'static str,
    },
    AlreadyPublished,
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("register_custom_environment_effect_renderers: ")?;
        match self {
            Self::DefaultId { owner, kind, id } => write!(
                f,
                "mod {owner}: cannot register a custom {kind} renderer for the default id {id}"
            ),
            Self::DuplicateId {
                kind,
                id,
                earlier,
                owner,
            } => write!(
                f,
                "mod {owner}: custom {kind} renderer {id} collides with earlier registration from mod {earlier}"
            ),
            Self::AlreadyPublished => {
                f.write_str("custom environment effect renderers were already published")
            }
        }
    }
}

impl std::error::Error for RegistrationError {}

/// Frozen per-slot custom environment effect renderer maps.
#[derive(Default)]
pub struct Registry {
    skybox: HashMap<Identifier, RegisteredSkyboxRenderer>,
    clouds: HashMap<Identifier, RegisteredCloudsRenderer>,
    weather: HashMap<Identifier, RegisteredWeatherRenderer>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

impl Registry {
    /// Validate and freeze collected registrations: refuses `minecraft:default` and duplicate IDs
    /// within the same effect slot, naming both mods.
    pub fn from_registrations(
        registrations: Vec<EnvironmentEffectRendererRegistration>,
    ) -> Result<Self, RegistrationError> {
        let mut skybox = HashMap::new();
        let mut clouds = HashMap::new();
        let mut weather = HashMap::new();
        for registration in registrations {
            let (owner, id, payload) = environment_effect_registration_into_parts(registration);
            let kind = match payload {
                EnvironmentEffectRendererImpl::Skybox(_) => EnvironmentEffectKind::Skybox,
                EnvironmentEffectRendererImpl::Clouds(_) => EnvironmentEffectKind::Clouds,
                EnvironmentEffectRendererImpl::Weather(_) => EnvironmentEffectKind::Weather,
            };
            if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
                return Err(RegistrationError::DefaultId { owner, kind, id });
            }
            match payload {
                EnvironmentEffectRendererImpl::Skybox(renderer) => match skybox.entry(id) {
                    Entry::Vacant(slot) => {
                        slot.insert(RegisteredSkyboxRenderer {
                            owner: CallbackOwner::for_mod(owner),
                            renderer,
                        });
                    }
                    Entry::Occupied(existing) => {
                        return Err(RegistrationError::DuplicateId {
                            kind,
                            id: existing.key().clone(),
                            earlier: existing.get().owner(),
                            owner,
                        });
                    }
                },
                EnvironmentEffectRendererImpl::Clouds(renderer) => match clouds.entry(id) {
                    Entry::Vacant(slot) => {
                        slot.insert(RegisteredCloudsRenderer {
                            owner: CallbackOwner::for_mod(owner),
                            renderer,
                        });
                    }
                    Entry::Occupied(existing) => {
                        return Err(RegistrationError::DuplicateId {
                            kind,
                            id: existing.key().clone(),
                            earlier: existing.get().owner(),
                            owner,
                        });
                    }
                },
                EnvironmentEffectRendererImpl::Weather(renderer) => match weather.entry(id) {
                    Entry::Vacant(slot) => {
                        slot.insert(RegisteredWeatherRenderer {
                            owner: CallbackOwner::for_mod(owner),
                            renderer,
                        });
                    }
                    Entry::Occupied(existing) => {
                        return Err(RegistrationError::DuplicateId {
                            kind,
                            id: existing.key().clone(),
                            earlier: existing.get().owner(),
                            owner,
                        });
                    }
                },
            }
        }
        Ok(Self {
            skybox,
            clouds,
            weather,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.skybox.is_empty() && self.clouds.is_empty() && self.weather.is_empty()
    }

    pub fn skybox(&self, id: &Identifier) -> Option<&RegisteredSkyboxRenderer> {
        if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
            return None;
        }
        self.skybox.get(id)
    }

    pub fn clouds(&self, id: &Identifier) -> Option<&RegisteredCloudsRenderer> {
        if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
            return None;
        }
        self.clouds.get(id)
    }

    pub fn weather(&self, id: &Identifier) -> Option<&RegisteredWeatherRenderer> {
        if id.as_str() == DEFAULT_CUSTOM_ENVIRONMENT_EFFECT {
            return None;
        }
        self.weather.get(id)
    }
}

/// Validate and publish custom environment effect renderer registrations once at client startup.
pub fn register(
    registrations: Vec<EnvironmentEffectRendererRegistration>,
) -> Result<(), RegistrationError> {
    if registrations.is_empty() {
        return Ok(());
    }
    let registry = Registry::from_registrations(registrations)?;
    REGISTRY
        .set(registry)
        .map_err(|_| RegistrationError::AlreadyPublished)
}

/// Whether any custom environment effect renderer was registered at startup.
pub fn has_any() -> bool {
    REGISTRY.get().is_some_and(|registry| !registry.is_empty())
}

/// Resolve the registered skybox renderer for `id`, or `None` when default or unregistered.
pub fn skybox(id: &Identifier) -> Option<&'static RegisteredSkyboxRenderer> {
    REGISTRY.get()?.skybox(id)
}

/// Resolve the registered clouds renderer for `id`, or `None` when default or unregistered.
pub fn clouds(id: &Identifier) -> Option<&'static RegisteredCloudsRenderer> {
    REGISTRY.get()?.clouds(id)
}

/// Resolve the registered weather effect renderer for `id`, or `None` when default or unregistered.
pub fn weather(id: &Identifier) -> Option<&'static RegisteredWeatherRenderer> {
    REGISTRY.get()?.weather(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::CustomEffectIds;
    use mod_api::{
        __private::game::environment_effect_registration_from_parts, Camera, LevelRenderState,
        LevelRenderStateWrite, RenderDataKey,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const SKY_DATA_KEY: RenderDataKey<u32> = RenderDataKey::new("test:sky_data");

    struct CountingSky {
        calls: Arc<AtomicUsize>,
        observed_data: Arc<AtomicUsize>,
        suppress: bool,
    }
    impl CustomSkyboxRenderer for CountingSky {
        fn render_sky(
            &self,
            view: &WorldRenderView<'_>,
            _sky: &SkyRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(&v) = view.render_data(&SKY_DATA_KEY) {
                self.observed_data.store(v as usize, Ordering::SeqCst);
            }
            self.suppress
        }
    }

    struct CountingClouds {
        calls: Arc<AtomicUsize>,
        suppress: bool,
    }
    impl CustomCloudsRenderer for CountingClouds {
        fn render_clouds(
            &self,
            _view: &WorldRenderView<'_>,
            _clouds: &CloudsRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.suppress
        }
    }

    struct CountingWeather {
        calls: Arc<AtomicUsize>,
        suppress: bool,
    }
    impl CustomWeatherEffectRenderer for CountingWeather {
        fn render_weather(
            &self,
            _view: &WorldRenderView<'_>,
            _weather: &WeatherRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.suppress
        }
    }

    struct PanickingSky;
    impl CustomSkyboxRenderer for PanickingSky {
        fn render_sky(
            &self,
            _view: &WorldRenderView<'_>,
            _sky: &SkyRenderState,
            _draw: &mut dyn WorldDrawContext,
        ) -> bool {
            panic!("intentional test panic in custom skybox renderer");
        }
    }

    struct NoopDraw;
    impl WorldDrawContext for NoopDraw {}

    fn test_camera() -> Camera {
        Camera {
            position: [0.0, 64.0, 0.0],
            eye_height: 1.62,
            partial_tick: 0.25,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: [0.0; 16],
            projection: [0.0; 16],
            dimension: "example:custom".into(),
        }
    }

    fn sky_reg(
        owner: &'static str,
        id: &Identifier,
        renderer: impl CustomSkyboxRenderer,
    ) -> EnvironmentEffectRendererRegistration {
        environment_effect_registration_from_parts(
            owner,
            id.clone(),
            EnvironmentEffectRendererImpl::Skybox(Box::new(renderer)),
        )
    }

    fn clouds_reg(
        owner: &'static str,
        id: &Identifier,
        renderer: impl CustomCloudsRenderer,
    ) -> EnvironmentEffectRendererRegistration {
        environment_effect_registration_from_parts(
            owner,
            id.clone(),
            EnvironmentEffectRendererImpl::Clouds(Box::new(renderer)),
        )
    }

    fn weather_reg(
        owner: &'static str,
        id: &Identifier,
        renderer: impl CustomWeatherEffectRenderer,
    ) -> EnvironmentEffectRendererRegistration {
        environment_effect_registration_from_parts(
            owner,
            id.clone(),
            EnvironmentEffectRendererImpl::Weather(Box::new(renderer)),
        )
    }

    #[test]
    fn default_id_is_refused_at_freeze() {
        let default_id = Identifier::parse(DEFAULT_CUSTOM_ENVIRONMENT_EFFECT).unwrap();
        let err = Registry::from_registrations(vec![sky_reg(
            "bad_mod",
            &default_id,
            CountingSky {
                calls: Arc::new(AtomicUsize::new(0)),
                observed_data: Arc::new(AtomicUsize::new(0)),
                suppress: true,
            },
        )])
        .err()
        .unwrap();
        assert_eq!(
            err,
            RegistrationError::DefaultId {
                owner: "bad_mod",
                kind: EnvironmentEffectKind::Skybox,
                id: default_id,
            }
        );
    }

    #[test]
    fn duplicate_id_in_same_slot_names_both_mods_while_different_slots_coexist() {
        let shared = Identifier::parse("example:shared").unwrap();
        // Same ID across skybox and clouds is valid.
        let ok = Registry::from_registrations(vec![
            sky_reg(
                "mod_a",
                &shared,
                CountingSky {
                    calls: Arc::new(AtomicUsize::new(0)),
                    observed_data: Arc::new(AtomicUsize::new(0)),
                    suppress: true,
                },
            ),
            clouds_reg(
                "mod_a",
                &shared,
                CountingClouds {
                    calls: Arc::new(AtomicUsize::new(0)),
                    suppress: true,
                },
            ),
        ])
        .unwrap();
        assert!(ok.skybox(&shared).is_some());
        assert!(ok.clouds(&shared).is_some());
        assert!(ok.weather(&shared).is_none());

        // Duplicate ID in the same slot fails naming both mods.
        let err = Registry::from_registrations(vec![
            sky_reg(
                "mod_first",
                &shared,
                CountingSky {
                    calls: Arc::new(AtomicUsize::new(0)),
                    observed_data: Arc::new(AtomicUsize::new(0)),
                    suppress: true,
                },
            ),
            sky_reg(
                "mod_second",
                &shared,
                CountingSky {
                    calls: Arc::new(AtomicUsize::new(0)),
                    observed_data: Arc::new(AtomicUsize::new(0)),
                    suppress: true,
                },
            ),
        ])
        .err()
        .unwrap();
        assert_eq!(
            err,
            RegistrationError::DuplicateId {
                kind: EnvironmentEffectKind::Skybox,
                id: shared,
                earlier: "mod_first",
                owner: "mod_second",
            }
        );
        let msg = err.to_string();
        assert!(msg.contains("mod_first"));
        assert!(msg.contains("mod_second"));
    }

    #[test]
    fn vanilla_dimensions_run_zero_callbacks_and_custom_dimensions_suppress_matching_passes() {
        let sky_calls = Arc::new(AtomicUsize::new(0));
        let sky_data = Arc::new(AtomicUsize::new(0));
        let cloud_calls = Arc::new(AtomicUsize::new(0));
        let weather_calls = Arc::new(AtomicUsize::new(0));

        let sky_id = Identifier::parse("example:aurora_sky").unwrap();
        let cloud_id = Identifier::parse("example:nebula_clouds").unwrap();
        let weather_id = Identifier::parse("example:ash_rain").unwrap();

        let registry = Registry::from_registrations(vec![
            sky_reg(
                "example",
                &sky_id,
                CountingSky {
                    calls: Arc::clone(&sky_calls),
                    observed_data: Arc::clone(&sky_data),
                    suppress: true,
                },
            ),
            clouds_reg(
                "example",
                &cloud_id,
                CountingClouds {
                    calls: Arc::clone(&cloud_calls),
                    suppress: false,
                },
            ),
            weather_reg(
                "example",
                &weather_id,
                CountingWeather {
                    calls: Arc::clone(&weather_calls),
                    suppress: true,
                },
            ),
        ])
        .unwrap();

        // Vanilla dimensions have default attributes -> CustomEffectIds::default() -> zero callbacks.
        let vanilla_ids = CustomEffectIds::default();
        assert!(
            vanilla_ids
                .skybox
                .as_ref()
                .and_then(|id| registry.skybox(id))
                .is_none()
        );
        assert!(
            vanilla_ids
                .clouds
                .as_ref()
                .and_then(|id| registry.clouds(id))
                .is_none()
        );
        assert!(
            vanilla_ids
                .weather
                .as_ref()
                .and_then(|id| registry.weather(id))
                .is_none()
        );
        assert_eq!(sky_calls.load(Ordering::SeqCst), 0);
        assert_eq!(cloud_calls.load(Ordering::SeqCst), 0);
        assert_eq!(weather_calls.load(Ordering::SeqCst), 0);

        // Custom dimension with matching attributes runs callbacks with extracted state only.
        let mut state = LevelRenderState::new();
        let writer: &mut dyn LevelRenderStateWrite = &mut state;
        writer.set_render_data(&SKY_DATA_KEY, 99u32);
        let camera = test_camera();
        let view = WorldRenderView::new(&camera, 0.25, 100, false, &[]).__with_render_data(&state);
        let mut draw = NoopDraw;

        assert!(registry.skybox(&sky_id).unwrap().render_sky(
            &view,
            &SkyRenderState::default(),
            &mut draw
        ));
        assert!(!registry.clouds(&cloud_id).unwrap().render_clouds(
            &view,
            &CloudsRenderState::default(),
            &mut draw
        ));
        assert!(registry.weather(&weather_id).unwrap().render_weather(
            &view,
            &WeatherRenderState::default(),
            &mut draw
        ));
        assert_eq!(sky_calls.load(Ordering::SeqCst), 1);
        assert_eq!(sky_data.load(Ordering::SeqCst), 99);
        assert_eq!(cloud_calls.load(Ordering::SeqCst), 1);
        assert_eq!(weather_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn panicking_renderer_falls_back_to_vanilla_and_disables_itself() {
        let sky_id = Identifier::parse("faulty:sky").unwrap();
        let registry =
            Registry::from_registrations(vec![sky_reg("faulty", &sky_id, PanickingSky)]).unwrap();
        let camera = test_camera();
        let view = WorldRenderView::new(&camera, 0.0, 1, false, &[]);
        let mut draw = NoopDraw;
        let renderer = registry.skybox(&sky_id).unwrap();
        assert!(!renderer.render_sky(&view, &SkyRenderState::default(), &mut draw));
        assert!(!renderer.render_sky(&view, &SkyRenderState::default(), &mut draw));
    }
}
