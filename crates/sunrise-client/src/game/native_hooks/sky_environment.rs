use super::Game;

pub(in crate::game) struct SkyEnvironmentSample {
    pub day_time: i64,
    pub rain: f32,
    pub thunder: f32,
    pub day_time_changed: bool,
    /// When either weather strength changed, packs take the resolved raw thunder and rain
    /// instead of their ordinary synchronized inputs.
    pub raw_thunder_override: Option<f32>,
}

impl SkyEnvironmentSample {
    fn new(day_time: i64, (rain, raw_thunder): (f32, f32)) -> Self {
        Self {
            day_time,
            rain,
            thunder: raw_thunder * rain,
            day_time_changed: false,
            raw_thunder_override: None,
        }
    }

    fn resolve(
        original_time: i64,
        weather: Option<(f32, f32)>,
        has_default_clock: bool,
        dispatch: impl FnOnce(&mut i64, &mut f32, &mut f32),
    ) -> Self {
        let (original_rain, original_thunder) = weather.unwrap_or_default();
        let (mut day_time, mut rain, mut raw_thunder) =
            (original_time, original_rain, original_thunder);
        dispatch(&mut day_time, &mut rain, &mut raw_thunder);
        if !has_default_clock {
            day_time = original_time;
        }
        let resolved_strength = |before: f32, after: f32| {
            if after.is_nan() || after.to_bits() == before.to_bits() {
                before
            } else {
                // Level.setRainLevel and Level.setThunderLevel.
                after.clamp(0.0, 1.0)
            }
        };
        (rain, raw_thunder) = if weather.is_some() {
            (
                resolved_strength(original_rain, rain),
                resolved_strength(original_thunder, raw_thunder),
            )
        } else {
            (0.0, 0.0)
        };
        Self {
            day_time_changed: day_time != original_time,
            raw_thunder_override: (rain.to_bits() != original_rain.to_bits()
                || raw_thunder.to_bits() != original_thunder.to_bits())
            .then_some(raw_thunder),
            ..Self::new(day_time, (rain, raw_thunder))
        }
    }
}

impl Game<'_> {
    pub(in crate::game) fn native_sky_environment(
        &self,
        partial_tick: f32,
        day_time: &mut i64,
        rain: &mut f32,
        raw_thunder: &mut f32,
    ) {
        self.fire_sky_environment(partial_tick, day_time, rain, raw_thunder);
    }

    pub(in crate::game) fn sample_sky_environment(
        &self,
        partial_tick: f32,
    ) -> SkyEnvironmentSample {
        let day_time = self.view_day_time();
        let weather = self
            .can_have_weather()
            .then(|| self.view_weather(partial_tick));
        if mod_api::__private::listened(mod_api::HookId::SkyEnvironment)
            && !self.is_logical_server()
            && !self.shows_panorama()
            && let Some(remote) = self.remote.as_ref().filter(|remote| remote.has_level())
        {
            SkyEnvironmentSample::resolve(
                day_time,
                weather,
                remote.has_default_clock(),
                |day_time, rain, raw_thunder| {
                    self.native_sky_environment(partial_tick, day_time, rain, raw_thunder);
                },
            )
        } else {
            SkyEnvironmentSample::new(day_time, weather.unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SkyEnvironmentSample;
    use mod_api::{
        ClientHost, Hooks, ModEntry, SkyEnvironmentEvent, dispatch_sky_environment,
        testing::{FakeHost, with_dispatch},
    };
    use sunrise_client::world::Weather;

    struct SetTimeAndThunder;
    #[mod_api::__subscriptions]
    impl Hooks for SetTimeAndThunder {
        fn sky_environment(&self, _: &dyn ClientHost, event: &mut SkyEnvironmentEvent) {
            event.partial_tick = 1.0;
            event.day_time = i64::MIN;
            event.raw_thunder = f32::INFINITY;
        }
    }

    struct DiscardRain;
    #[mod_api::__subscriptions]
    impl Hooks for DiscardRain {
        fn sky_environment(&self, _: &dyn ClientHost, event: &mut SkyEnvironmentEvent) {
            // Dispatch contains listener panics, so make the observed inputs part of the result.
            event.rain = if (event.partial_tick, event.day_time, event.raw_thunder)
                == (0.25, i64::MIN, f32::INFINITY)
            {
                f32::NAN
            } else {
                0.0
            };
        }
    }

    #[test]
    fn sky_environment_resolves_the_final_listener_edits() {
        let host = FakeHost::new();
        let sample = |weather, has_default_clock| {
            SkyEnvironmentSample::resolve(
                6000,
                weather,
                has_default_clock,
                |day_time, rain, raw_thunder| {
                    dispatch_sky_environment(&host, 0.25, day_time, rain, raw_thunder);
                },
            )
        };
        let weather = Weather {
            rain_level: (0.375, 0.875),
            thunder_level: (0.625, 0.125),
            ..Weather::default()
        };
        let weather = Some(weather.raw_levels(0.25));
        let unedited = with_dispatch(Vec::new().into(), || sample(weather, true));
        assert_eq!(
            (unedited.day_time, unedited.rain, unedited.thunder),
            (6000, 0.5, 0.25)
        );
        assert!(!unedited.day_time_changed);
        assert_eq!(unedited.raw_thunder_override, None);
        with_dispatch(
            vec![
                ModEntry::new("set_time_and_thunder", Box::new(SetTimeAndThunder)),
                ModEntry::new("discard_rain", Box::new(DiscardRain)),
            ]
            .into(),
            || {
                let edited = sample(weather, true);
                assert_eq!(
                    (edited.day_time, edited.rain, edited.thunder),
                    (i64::MIN, 0.5, 0.5)
                );
                assert!(edited.day_time_changed);
                assert_eq!(edited.raw_thunder_override, Some(1.0));

                let forbidden = sample(None, true);
                assert_eq!(
                    (forbidden.day_time, forbidden.rain, forbidden.thunder),
                    (i64::MIN, 0.0, 0.0)
                );
                assert_eq!(forbidden.raw_thunder_override, None);

                let no_clock = sample(weather, false);
                assert_eq!(
                    (no_clock.day_time, no_clock.rain, no_clock.thunder),
                    (6000, 0.5, 0.5)
                );
                assert!(!no_clock.day_time_changed);
            },
        );
    }
}
