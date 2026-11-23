# Ticking a registered mob effect

Attach server-side behaviour to a content-declared effect with a `MobEffectPart`:

```rust,ignore
let effect = r.effect_key("meals:comfort");
if let Err(error) = r.mob_effect_part(&effect, Comfort) {
    context.refuse(&format!("Cannot attach Comfort: {error:?}"));
    return;
}
```

Implement the Part with `#[mod_api::sunrise_part]`. Its methods are:

- `on_effect_added` (version 2, vanilla's `MobEffect.onEffectAdded`) runs only when
  `LivingEntity.addEffect` inserts an instance of this effect onto an entity that did
  not already have it, after the active map and attribute modifiers are updated and
  vanilla's added sound plays (before `onEffectStarted`). It receives the resolved
  `effect` key, the target `entity` (`LivingRef`) and `amplifier`. It never runs on
  reapplications, `forceAddEffect`, world load or client sync.
- `should_apply_effect_tick_this_tick` receives the resolved `effect` key, remaining
  duration (`tick_count`, or the holder's tick count for an infinite effect), and
  amplifier. Set `event.apply = true` to run the effect now; it starts false.
- `apply_effect_tick` runs only when this Part's own cadence method says to apply. It receives
  `&mut dyn ServerHost`, the live `level: LevelRef`, the live `holder: LivingRef`, `effect` and
  `amplifier`. Read and write health (`holder.health`, `holder.heal`), damage
  (`holder.entity().hurt`), active effects (`holder.effect`), level rules (`level.game_rule`),
  and player food (`holder.entity().as_player(host)` -> `player.food`, `player.feed`,
  `player.add_exhaustion`) directly against the live world. Clear `event.keep` to end the effect
  before its duration counts down and skip later Parts.

`should_apply_effect_tick_this_tick` is version 4, `apply_effect_tick` is version 5, and
`on_effect_added` is version 2; all are scoped to `effect` and keyed by `effect`; there are no
corresponding global `Hooks` methods. Each Part has its own `apply`, starting false,
so one package's cadence never drives another package's action. The owning package's
Parts (the declaring package's, or the serving native host's for an effect declared by a
`served_by` content package) run first within that effect, even with `order = "before"` or `"none"`. Other Parts
run by mod dispatch order, then registration order. Each Part sees the live holder state left
by earlier Parts. This owner-first rule is specific to effects; it does not change package
dispatch or the order of block entity Parts.

A Part may attach to an effect declared by its own package, by a content package it
serves (`served_by`), or by an active required
or optional dependency in its `[dependencies]`. An attachment without that
dependency refuses loading, naming the effect, its owning package and the dependency to add.
If the effect's namespace is the ID of an absent package, its Part and key request are
skipped only when an optional dependency row applies on the installation side and survives
instance overrides. A present package must declare the requested effect; other unknown effects
keep the normal unknown-key refusal.

Invalid keys refuse with `MobEffectPartError::InvalidKey`; built-in vanilla effects refuse
immediately with `MobEffectPartError::VanillaEffect`; attaching the same concrete
Part type twice returns `Duplicate`. A Part attached to several
effects distinguishes them with `event.effect`.

To observe every effect addition across vanilla and mod effects alike, subscribe to the
global `Hooks::mob_effect_added` event (NeoForge's `MobEffectEvent.Added`, version 2),
which fires after `canBeAffected` / `mob_effect_applicable` and before active-effect
mutation with `entity`, `old_effect`, `new_effect` and optional `source` (including
reapplications and unchanged updates; non-cancellable).

## Reading and mutating the holder

`apply_effect_tick` receives the live server host, `event.level`, and `event.holder` with the
entity in place (for mobs, the borrowed mob tick suspends and restores the mob list before
dispatch).

For example, an effect that heals every 80 ticks can set `apply` when
`event.tick_count % 80 == 0`, then check `event.holder.effect(host, regeneration_key).is_none()`,
check `event.holder.entity().as_player(host).and_then(|p| p.food(host))` for positive saturation,
and call `event.holder.heal(host, 1.0)`. A mob returns `None` from `as_player` and can still heal.

Read level rules through `event.level.game_rule(host, "minecraft:natural_health_regeneration")`.

`player.add_exhaustion(host, amount)` follows vanilla's `FoodData.addExhaustion` on players:
it adds the amount and caps the result at 40, without a lower clamp. Negative
amounts therefore subtract exhaustion. To remove at most 4, read `player.food(host)`
and pass `-food.exhaustion.min(4.0)` only when exhaustion is positive.

If a callback structurally adds or removes effects on the holder during `apply_effect_tick`,
the current effect finishes and the active-effect pass stops before advancing to later effects,
matching vanilla's `LivingEntity.tickEffects` (`ConcurrentModificationException`).

A `side = "server"` package can attach a Part because its methods are server-only.
