//! The `entity_enter_love_mode_pre` hook declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// An accepted feed is about to put `animal` in love in `level`, at
    /// vanilla's `Animal.setInLove`. `player` is the actual feeder, when known.
    /// Runs on the logical server, including integrated and dedicated servers.
    /// NeoForge has no event at this point.
    ///
    /// Return `Flow::Cancel` to skip the timer, breeding credit, entry hearts
    /// and `entity_enter_love_mode` notice, without undoing the feed. Ordinary
    /// animal food (including bucket remainders) and panda bamboo are used
    /// before this hook; equines, camels and llamas use food after their eating
    /// handler. Growth, taming and healing-only feeds do not fire this hook.
    /// If the animal is no longer live and eligible in the same level after
    /// the callback, entry is skipped without retrying or retargeting.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_enter_love_mode_pre"
    )]
    fn entity_enter_love_mode_pre(
        animal: EntityRef,
        level: LevelRef,
        player: Option<PlayerRef>,
        cancel: &mut bool,
    );
}
