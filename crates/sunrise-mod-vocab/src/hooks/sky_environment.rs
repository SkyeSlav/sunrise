//! The `sky_environment` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Edit the visual environment sample for a joined client level, including single-player.
    /// This never runs for the title panorama or on the logical server. It does not change
    /// synchronized time or weather, game time, gameplay light, or packets.
    ///
    /// The sample starts after local visual settings. Each listener gets the original
    /// `partial_tick` and sees earlier listeners' edits to `day_time`, `rain`, and
    /// `raw_thunder` in dependency order. A frame may take several samples at different
    /// partial ticks; return the same edits for the same inputs.
    ///
    /// `day_time` replaces the level's default clock for display. Any signed tick value is
    /// accepted; timelines keep their own clocks and looping rules. A level without a default
    /// clock ignores the time edit, and an edit does not add celestial effects.
    ///
    /// Weather edits apply only where `Level.canHaveWeather` permits it. After all listeners
    /// run, NaN leaves that field at its original strength and edited rain and raw thunder
    /// are clamped as in `Level.setRainLevel` and `Level.setThunderLevel` (0 through 1).
    /// Display thunder is the resolved raw thunder multiplied by the resolved rain.
    #[hook(
        target = "sunrise_client::game::native_hooks::sky_environment::Game<'_>::native_sky_environment",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        event_per_listener = true,
        neoforge = "none (vanilla Level.getDefaultClockTime, Level.getRainLevel and Level.getThunderLevel)"
    )]
    fn sky_environment(
        partial_tick: f32,
        day_time: &mut i64,
        rain: &mut f32,
        raw_thunder: &mut f32,
    );
}
