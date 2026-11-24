//! `sleep_skip_night`: a mod keeps the night from being skipped while enough players sleep.
use crate::game::Game;
use mod_api::{Hooks, ModEntry, ServerHost, SleepSkipNightEvent};
use std::sync::{Arc, Mutex};

/// Records what each call saw, and cancels the skip.
struct Keep(Arc<Mutex<Vec<(String, i32, i64, i64)>>>);
#[mod_api::__subscriptions]
impl Hooks for Keep {
    fn sleep_skip_night(
        &self,
        _: &mut dyn ServerHost,
        event: &mut SleepSkipNightEvent<'_>,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().push((
            event.dimension.to_owned(),
            event.percentage,
            event.sleeping,
            event.active,
        ));
        mod_api::Flow::Cancel
    }
}

/// Guest 12 in bed for 100 ticks at night, in the rain, in this level.
fn deep_asleep(game: &mut Game<'_>) {
    let level = game.dim.id().to_owned();
    game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
        g.dimension = level.clone();
    });
    let player = game.player(12).unwrap();
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.bed = Some([5, 64, 5]);
    });
    assert!(player.set_sleep_timer(game, 100));
    let clock = game.level_clock_name(game.dim);
    game.set_command_clock_time(&clock, 13_000);
    game.world.weather.raining = true;
    game.world.weather.rain_time = 500;
    game.world.weather.rain_level = (1.0, 1.0);
}

#[test]
#[ignore = "daily"]
fn a_canceled_skip_keeps_the_clock_the_sleepers_and_the_rain() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        assert!(game.ui.rules.set(
            "players_sleeping_percentage",
            sunrise_client_core::gamerules::Value::Int(1)
        ));
        let clock = game.level_clock_name(game.dim);
        deep_asleep(game);
        let (active, sleeping, _) = game.sleep_counts();
        assert!(game.enough_slept());

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new("keep", Box::new(Keep(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || game.tick_sleep());
        assert_eq!(
            *heard.lock().unwrap(),
            [(game.dim.id().to_owned(), 1, sleeping, active)]
        );
        assert_eq!(game.command_clock_time(&clock), 13_000);
        assert!(game.world.weather.raining);
        assert!(game.lan.as_ref().unwrap().hub.guests[0].bed.is_some());

        // With no mod canceling, the same tick skips the night as vanilla does.
        game.tick_sleep();
        assert_eq!(game.command_clock_time(&clock), 24_000);
        assert!(!game.world.weather.raining);
        assert!(game.lan.as_ref().unwrap().hub.guests[0].bed.is_none());
    });
}
