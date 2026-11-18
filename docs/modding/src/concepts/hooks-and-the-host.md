# Hooks and the Host

A **hook** is one moment in the game: a tick, a block broken, a player joining. Your mod implements the hooks it
needs as methods of `mod_api::Hooks`, and the game calls each subscribed mod with the moment's **event** and a
**host**, its typed view of the game. A moment that belongs to one object is a method of a [Part](parts.md)
instead, called only for the objects your mod attached it to.

![The game calls every mod that implements a hook, and a Part only for its own objects. Both receive an event and
reach the game through the Host, its handles, capabilities and attachments.](../images/hooks-parts.svg)

## Dispatch

Mods run in order of their listener priority, highest first, then in dependency order: a mod after the mods it
depends on. When a mod cancels an event, the mods after it are skipped, unless they receive cancelled events
([Listener priority and cancellation](#listener-priority-and-cancellation)).

What a hook's return value means depends on its kind:

- **Listen** hooks are notices: every subscribed mod is called.
- **Cancellable** hooks return `Flow`: `Flow::Cancel` stops the game's action and skips later mods.
- **Replacement** hooks return an `Option`: the first mod to answer `Some` wins.
- **Collection** hooks call every mod, each adding to a list in the event.

A hook with no receiver gets no `Host`.

## Player tick hooks

`player_tick_pre` and `player_tick_post` bracket the player's body tick on both
logical sides. On the server, neither hook fires for a spectator whose bounds
touch an unloaded chunk: that player's body tick is skipped. Connection and
level upkeep and outgoing state updates still run. The server resolves whether
the body runs before the pre hook; a listener changing the player's position
does not change that decision for the current tick.

## Explosion membership

`explosion_detonate` version 2 exposes mutable `blocks` and `entities` lists
before an explosion applies its effects. The initial entity query excludes the
exploding source. Remove entries to spare them, or add live handles to make them
eligible; additions still obey normal distance, exposure and immunity checks.
Stale or invalid handles are dropped and repeated IDs are visited once. A mod
can explicitly add a valid source handle back; per-kind immunity still applies.

The final list controls membership, not global effect ordering. Its order is
used for ordinary storage targets. Crystals/dragon and players retain their
earlier separate passes, while frames and other decorations run last in storage
order. Block positions are shuffled before destruction. An empty entity list
spares direct entity effects, not indirect consequences of destroying support
blocks. With no subscriber the game does not build a target list.

## Saving and game events

To save from inside a hook, use `level.request_save(host)` so the save runs at the next settled
start-of-tick boundary. A synchronous `save-all` while an entity pass or held-out row is borrowed
fails with vanilla's `commands.save.failed` feedback and a failed command result, without saving.

To emit a vibration-producing game event, use `level.game_event(host, event, at, context)`.
`GameEventContext` carries an optional source entity and affected block state; see
[game events with context](../how-to/world/game-events.md) for listener semantics and refusals.

## Listener priority and cancellation

`#[listener]` on a hook method sets both, as NeoForge's `EventPriority` and `receiveCanceled` do. Two mods, each
its own crate:

```rust,ignore
#[mod_api::sunrise_mod]
impl Hooks for Protection {
    // Runs before mods at the default priority, 0; higher runs first.
    #[listener(priority = 100)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if self.protected(host, event.pos) { Flow::Cancel } else { Flow::Continue }
    }
}

#[mod_api::sunrise_mod]
impl Hooks for Override {
    // Receives cancelled events and may clear cancellation.
    #[listener(priority = -100, receive_cancelled)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if event.cancelled && self.allowed(host, event.pos) {
            event.cancelled = false;
        }
        Flow::Continue
    }
}
```

`priority` is an `i32`, 0 when omitted; mods of equal priority keep dependency order, as `[[inject]]`
priorities do ([Injections](injections.md#ordering)). Each hook's order is fixed once, when the mods are
installed. Every cancellable hook's event has a `cancelled` field: a mod without `receive_cancelled` always sees
`false`, since it is not called once the event is cancelled. A mod that receives cancelled events and sets it back to
`false` lets the game's behaviour run, and the mods after it are told again. An event the game hands over already
cancelled reaches only mods that receive cancelled events. `receive_cancelled` on a hook without a cancel does not
compile, and `#[listener]` is for a mod's hooks only: a part is called for its own thing, in no order.

Native and wasm mods use the same listener ordering and cancellation rules. Wasm subscription records carry the hook name, signed priority, and receive-cancelled flag through the shared-memory Wire boundary at load time. The event's `cancelled` field crosses with its other writable fields.

## Events and shared state

Each hook method takes its host, where it has one, and one event:

```rust,ignore
fn living_incoming_damage(
    &self,
    _level: &mut dyn HitHost,
    event: &mut LivingIncomingDamageEvent<'_>,
) -> Flow {
    if event.kind == "minecraft:pig" {
        let halved = event.container.new_damage() * 0.5;
        event.container.set_new_damage(halved);
    }
    Flow::Continue
}
```

For incoming damage, `event.weapon` is the direct entity's weapon, including a projectile's
saved firing weapon. `event.causing_weapon` is instead the causing living entity's current
main-hand weapon (`DamageSource.getEntity().getWeaponItem()`): an off-hand crossbow shot can
name the saved crossbow in `weapon` and the shooter's main-hand sword in `causing_weapon`.
An absent, unknown or non-living cause has no `causing_weapon`.

The event is a generated struct named after the catalog hook, in CamelCase plus `Event`, with
the hook point's arguments as public fields: values as values and borrowed ones as references (`event.kind`). A
number or `bool` a mod may change is a plain field, written back once every mod has run, even after a `Cancel`;
anything else a mod may change, a game struct or a list, is a `&mut` field. A cancellation or replacement is
the return value; a collection hook's list is a field (`event.lines`). One event goes through every mod, so each
sees the changes before it. To exercise dispatch behavior, use the hook's dispatch function
([Testing a mod](testing.md)).

The generator validates edited stack-list outputs inside each listener's owning scope. A refused stack
output retains the immediately preceding accepted value, not the event's initial value. Scalar
outputs and cancellation still commit; later listeners see the accepted stacks and run independently
under the usual cancellation rules. Each refused output logs one warning naming the mod, hook,
output and conversion error. This policy is identical for native and wasm listeners.

Hook methods take `&self` and require `Send + Sync`. The process-wide mod list is installed once, like mixin module statics. One mod instance serves both
the window's game and the server-thread game. A hook can also fire again inside
the same mod: for example, `LevelRef::set_block` fires `block_changed`. Mods keep mutable
state with interior mutability so these calls can share the instance safely. Data-load
hooks can run on any loading thread; item default changes must be identical on every side.

`Hooks` contains global hooks. Scoped hooks belong to `BlockPart`, `BlockEntityPart`,
`ItemPart`, `MenuPart` or `FluidPart` and run for the registered object.

## Item tooltip context

The item's `tooltip` hook (`Item.appendHoverText`) receives `event.context`, a
borrowed `TooltipAppendContext`. Component `TooltipAppender::append` callbacks
receive the same context: local player identity, `advanced` (F3+H), `creative`
(creative-tab slot), `nested` (a loaded crossbow projectile), and `tick_rate`.
The rate follows vanilla 26.3's `Item.TooltipContext.tickRate()`: the current
client level's tick-rate manager, or 20 when no level is present. It is not
measured TPS or the frame rate. The item hook still does not fire for nested
projectiles or hidden tooltips; adding context does not change line ordering.

## Level hosts

Level hooks use access-specific hosts rather than `Host`. `LevelHost` provides
`block_state` through `LevelAccess::block_state`, client-side detection and
`state_with_property`. `state_string(state)` serializes the full name and properties, or returns `None`
for an unknown state; `min_y()` and `height()` read the level's build range. It also reads light and loaded state, through the game's own queries:
`brightness(LightLayer::Block | Sky, pos)` (`getBrightness`, the stored 0 to 15),
`raw_brightness(pos, sky_darken)` (`getRawBrightness`, which `CropBlock.randomTick` asks with 0)
and `is_loaded(pos)` (`Level.isLoaded`: inside the build height and in a loaded chunk),
`is_raining_at(pos)` (`Level.isRainingAt`, false where `pos` is not loaded) and `enabled_features()`
(`LevelReader.enabledFeatures`, a borrowed `FeatureFlags<'_>`). These queries do not allocate.
None of them loads a chunk. Light above the build height reads as full sky and below it as
none, and a chunk that isn't loaded reads as the game's query does (full sky, no block light),
so check `is_loaded` first. It also answers tag and fluid questions without a `Host`: `block_in_tag(block_key, tag)`,
`fluid_state(pos)` (`getFluidState`, `None` where the block read is),
`state_fluid_state(state)` and `fluid_in_tag(fluid_name, tag)`. Fluid reads return `FluidState` with a
`FluidKey`, amount, falling flag and engine-derived source flag; `is_empty()` and `is_source()` report
those states without a registry lookup. `block_state_id(name)` parses a block and properties, and `block_state_default(block)` gives a block's default state, as on `Host`; both return `None` for invalid input or unavailable context. Tag reads are `None` when authoritative tags are unavailable, including on a client before its server's tags are applied. For item membership and member lists, use `Host::tags()`: resolve a `TagKey` with `Tags::key`, then call `Tags::is_in_tag`, `Tags::items` or `Tags::fluids`. Membership reads return `None` on clients until server-sent tags are applied, which `tags_updated` announces; they never substitute local resource-pack tags.
`default_components_bound` precedes `tags_updated` after item defaults are published and readable.
Both notices carry `TagsUpdateCause`: `ServerDataLoad { reload }` for the server's initial data
load or reload, and `ClientPacketReceived { integrated_server }` for a client's applied tags.
`should_update_static_data()` is true on the server and a remote client, but false on an
integrated client, whose server already updated the shared static data. Installing client tags
announces the bound defaults; it does not rebind those defaults.

`block_face_sturdy(pos, face, support)` reads `BlockState.isFaceSturdy` through the same engine query as the level handle, including dynamic and published shapes. `canSupportRigidBlock` is `(pos, Up, Rigid)`. `canSupportCenter` is `(pos, face, Center)`, except that vanilla also refuses `Down` on blocks tagged `minecraft:unstable_bottom_center`. `can_see_sky(pos)` tests full stored sky light through the level handle's query. Both return `None` on clients, for unloaded or out-of-height positions, or in a callback without the corresponding view. Neither loads chunks or opens loot containers. They are owning-game-thread reads, not available in worker-thread hook bodies.

`block_entity(pos)` reads this level's block entity without loading a chunk; it returns `None` when absent, unloaded or out for its tick. On a client it returns only the local block entity handle. `container_size(at)` and `container_stack(at, slot)` are the same side-local capability snapshots as `ItemsHost` supplies; neither generates unopened loot, and both refuse clients and unloaded positions. `BlockEntityPartState::state_in(level, entity)` and `state_existing_in` read the same attachment snapshots as their `Host` equivalents, without inserting a default, generating loot or loading chunks. Unloaded or stale holders fail; clients have no server Part state. All are owning-game-thread reads, never available in worker hook bodies.

`LevelRandomHost::container_signal(pos)` is the engine comparator container read. Unlike the immutable slot snapshot, it generates unopened loot once, as vanilla's `getItem` does, then calculates 0 to 15 using each stack's maximum size. It returns `None` on clients and unloaded positions, never loads a chunk, and runs only on the owning game thread. `analog_output` version 2 receives this mutable host so the loot generation and random draws happen in vanilla order; it permits no other world edit.

Redstone and shape reads on `LevelHost`, for placement and survival: `signal(pos, direction)`
(`SignalGetter.getSignal`), `direct_signal`, `direct_signal_to`, `best_neighbor_signal(pos)`,
`has_neighbor_signal(pos)` (what a lamp's or a door's placement reads) and `collision_shape(pos)` (the block's
collision boxes at `pos`, block-local, a block entity's published shape included, empty for no collision, `None`
for an unloaded position). The signal reads are the game's own, `block_signal` and `should_check_weak_power`
listeners included, and are the same ones `LevelRef` and the tick hooks give. `state_for_placement` and
`can_survive` receive a level that answers them, so a block can place or survive by what powers it or what shape
sits beside it. The levels of `state_for_placement`, `can_survive` and the level tick and shape-update hooks answer all of them;
the levels other callbacks get (a damage hit's, a part's read view) may return `None` for any of them, signal reads
and `collision_shape` alike.
A signal read never asks `state_for_placement` or `can_survive`, so it cannot re-enter the hook it was read in.
On a client (its placement prediction and its own shape updates ask `state_for_placement`, `can_survive`,
`may_place_on` and `can_sustain_plant` too) these reads answer from the client's loaded copy of the level, and the
reads above that refuse clients return `None`, as do `block_in_tag` and `fluid_in_tag`. `tags()` still returns a
service: `Tags::key` can resolve a key, but membership (`is_in_tag`) and member lists (`items`, `fluids`, `blocks`)
return `None` in this view. `BlockEntityPartState::state_in` gives its default there. Never answer from those as if
they were the server's: a client's own shape update that refuses a block removes it on that client.
`LevelTicksHost`'s signal reads moved here and now return `Option`.

Before a block's placement state is selected, `ItemPart::placement_context` replaces the item's
`BlockItem.updatePlacementContext` rewrite: `Some(Some(context))` rewrites the target and click,
`Some(None)` refuses, and `None` runs vanilla (including scaffolding's column walk). The placing
item cannot change, click and view values must be finite, and the target must be within bounds.
Replaceability is checked before the rewrite, not again on its answer. At the final `BlockItem.canPlace`
check, `ItemPart::must_survive` decides whether placement must pass `can_survive`: `Some(false)`
skips it, `Some(true)` requires it, and `None` keeps vanilla (scaffolding false, other block items
true). Standing/wall items retain their own unconditional candidate-survival checks. Entity
obstruction is still checked. Both hooks run on both sides with
a read-only `LevelSide`, dispatched by item id. These events have no stack yet: they decide from
the item and level, not held components. Player and dispenser placement offer the resolved decision
once to `entity_place`; if a listener changes its inputs, the placement is refused rather than
retargeted. The veto's placement neighbour and the post-placement notice use the resolved face,
and the notice uses the resolved click location. Neither changes the sides of the block placement or
survival hooks.

A neighbour's `BlockPart::fence_gate_in_wall` answers whether a fence gate sits at wall height
beside that state. It runs on both sides at placement and shape updates, with a read-only `LevelSide`:
`Some(true)` counts as a wall, `Some(false)` does not, and `None` keeps the `minecraft:walls` tag
check. It changes only the gate's `in_wall`, not wall connections or pathfinding.

`BlockPart::is_possible_to_respawn_in` answers the forced respawn column's space check on the
server, with a read-only host. If no bed or anchor rule handles the spawn, the server asks for
both the saved point and the block above. This includes a bed or anchor at which spawn cannot be
set. `Some(true)` accepts that state, `Some(false)` rejects it, and `None` keeps vanilla's predicate.
It does not select a bed or anchor's stand-up position; `respawn_position` handles those.

`LevelHost` has no random access: a draw from a read-only hook inside a level write would shift the level's random sequence away from vanilla's order. A hook that may draw is given a `LevelRandomHost`.
`LevelRandomHost` provides vanilla's `Level.random` so
mod draws come in vanilla's order. `LevelTicksHost` adds scheduling through
`schedule_block_tick` and `schedule_fluid_tick`, while
`LevelWriteHost` adds random access, `set_block`, `set_block_property`, `play_sound` and
`send_particles` and `level_event` (`levelEvent`, queued in order with the write's own events).
A damage hit's listeners (`living_incoming_damage`, `living_damage_pre`, `living_damage_post`,
`living_shield_block`, `living_use_totem`) are given a `HitHost`: a `LevelRandomHost` that also plays level
sounds and draws from an entity's own random, and writes no block. `host.play_sound_at(sound, at, source,
volume, pitch)` is `Level.playSound(null, x, y, z, ...)` and `host.play_sound_from(entity, ...)` is
`Level.playSound(null, entity, ...)`: every player in range hears it, and a cancelled hit still plays what was
queued. The seed is drawn at the call from the level's sound-seed stream, not from the level's random.
`entity.next_float(host)`, `next_double`, `next_bool` and `next_int(host, bound)` draw from the entity's actual
random stream, in listener order. A hit lends its attacker where the game applies a melee hit (the attacking player or mob, for player → mob, mob → mob and mob → player) and no entity at any other damage site (environment, projectiles, explosions, potions and the like); the attacking mob's own random is the one its AI draws from, and a hit built inside a mob's own tick lends none. Any other entity, a stale handle, a client and an out-of-range `bound` answer `None` (`false` for a sound).
Outside a write, a `LevelRef` plays level events, and their global form for every player on the server
([Playing level events](../how-to/world/level-events.md)).
`set_block(pos, state, flags)` is `Level.setBlock` on a whole state, as `LevelRef::set_block` takes it: it
removes the old block's block entity and creates the new block's in the write (`LevelChunk.setBlockState`), runs
the neighbour and shape updates `flags` ask for, and queues the write's level events in order. A mod block entity
it creates starts with the data the `block_entity` hook gives, which is fired in the write and receives the level
(`LevelSide`, read-only: vanilla's `newBlockEntity` gets no level), as the `Host`'s `native_set_block` fires it. It returns true when the block
changed, and false off an authoritative server, for a position that is not loaded or outside the build height,
an invalid state, or the state the block already has. It never loads a chunk. The game owns
the tick queue; mods do not pass it back to these helpers.

## Drawing

Frame drawing through `DrawContext`, `BlockEntityDrawContext` and `WorldDrawContext` stays
in `mod_api`. Raw GPU access is native-only, unstable and outside that API, in
`mod_render::render` (`sunrise-mod-render`). Replace its frame functions with native HEAD mixins compiled against the prebuilt game.
`render_needs` collects requests; `world_drawn` runs before the hand and HUD, and
`hand_drawn` runs after the hand, on the render thread. Use API draw contexts for API-typed drawing.

## After a hook that can change the world

A listener of a hook that gets a mutable host may remove, move or change objects. After such a hook
returns without cancelling, the game does not use the list position or the state it worked out before
the hook. It finds the object again by its entity id, checks the facts the action was decided on, and
abandons the action if the object is gone or a fact changed; it never works the action out again. The
hook still fires at its usual moment, and the check runs straight after it returns, before anything
moves. Stack checks are shallow: they compare the item id and the count, not components. Each hook's
doc names what it checks (`armor_stand_manipulate`, `entity_mount`, `cauldron_level_changed`,
`hanging_break`, `player_shear_entity`, `living_change_target`). The effect writes on a living handle
treat a mob that is marked removed but still listed as absent: they return false and change nothing.

## Hooks and injections

A hook is not an injection point: an `[[inject]]` never runs inside a hook's dispatch. A `head` injection that
cancels the game function which calls a hook keeps that hook from firing at all
([Injections](injections.md)).
