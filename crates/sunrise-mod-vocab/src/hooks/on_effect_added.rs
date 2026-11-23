//! The `on_effect_added` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `MobEffect.applyEffectTick`: the content mod's resolved `EffectKey` `effect` acts on the
    /// live `holder` in `level` at `amplifier`. Clear `keep` and the effect ends now, before it
    /// counts down.
    ///
    /// Runs on the server with `&mut dyn ServerHost` and the live holder in place: read health,
    /// max health, active effects (`holder.effect(host, key)`), food (narrow `holder.entity()`
    /// with `EntityRef::as_player`), and level game rules directly through the handles, and apply
    /// healing (`holder.heal`), damage (`holder.entity().hurt`), food/exhaustion (`PlayerRef::feed`,
    /// `PlayerRef::add_exhaustion`), or other world changes immediately.
    ///
    /// Called only when this Part's own cadence requests it (with the declaring package's — or,
    /// for a `served_by` content package, its serving native host's — Parts running first). Each
    /// Part sees the live state left by earlier Parts; clearing `keep` skips later Parts and ends
    /// the effect before its countdown. Structural edits to the holder's active effects during the
    /// callback follow vanilla `LivingEntity.tickEffects` (`ConcurrentModificationException`): the
    /// current effect finishes and the pass stops before advancing to later effects. Server-side.
    #[hook(
        side = "server",
        scope = "effect",
        key = "effect",
        target = "mod_api::parts::effects::dispatch_mob_effect_tick",
        version = 5,
        contract = "listen",
        neoforge = "none (vanilla MobEffect.applyEffectTick; NeoForge adds no event or extension for it)"
    )]
    fn apply_effect_tick(
        effect: EffectKey,
        level: LevelRef,
        holder: LivingRef,
        amplifier: i32,
        keep: &mut bool,
        #[engine] tick_count: i32,
    );
    /// `MobEffect.onEffectAdded`: called on the [`MobEffectPart`]s attached to the mod effect
    /// `effect` after `LivingEntity.addEffect` inserts an instance of `effect` onto `entity`
    /// that the entity did not already have (after attribute modifiers and the vanilla added
    /// sound, before `onEffectStarted`). Does not fire on reapplications, `forceAddEffect`,
    /// world load or client sync. The declaring package's parts run first, then dependent
    /// packages in dispatch and registration order. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::mob_effect_events::Game<'_>::native_on_effect_added",
        dispatch = "flat",
        side = "server",
        scope = "effect",
        key = "effect",
        version = 2,
        contract = "listen",
        neoforge = "none (vanilla MobEffect.onEffectAdded; NeoForge adds no event or extension for it)"
    )]
    fn on_effect_added(effect: EffectKey, entity: LivingRef, amplifier: i32);
}
