# Entity hooks

Implement catalog hooks as methods of `mod_api::Hooks`, with `#[mod_api::sunrise_mod]` on the `impl`.
The attribute derives subscriptions from the implemented methods. See
[Writing hooks](../../start/writing-hooks.md).

Native rendering callbacks such as `sunrise_mod_render::render::world_drawn`,
`hand_drawn` and `world_views` use a HEAD mixin. The launcher compiles the patched
function against the prebuilt game. Callbacks borrow the frame's GPU resources;
`world_views` supplies additional camera views. The renderer refuses invalid,
duplicate and oversized view requests by name, as `push` returns them.

Use the geometry types re-exported as `sunrise_mod_render::render::{Vertex, EntitySpan}`.
Implement the `render_needs` callback to set `RenderNeeds` requests for world depth
and entity geometry; these requests feed the engine's frame requirements so it
retains the resources your drawing callbacks borrow.

## Reading a hand or equipment slot

Resolve an entity with `host.living(entity)`, then use `living.with_equipment(host, slot, read)`
for one equipment slot or `living.with_hand_item(host, hand, read)` for the selected main-hand
stack or off hand. Both return the callback's result in `Some`; an empty or unavailable slot
returns `None` without running the callback. On the client only the local player's slots are
available.

```rust,ignore
let count = living.with_hand_item(host, Hand::Main, |stack| stack.count());
let helmet = living
    .with_equipment(host, EquipmentSlot::Head, |stack| stack.project())
    .flatten();
```

Use `project()` when you need components or an owned stack after the callback. For the
server-side attacker's live weapon, use `with_weapon_item`; for the weapon recorded by a
projectile, see [Death sources and weapons](../items/death-loot-context.md).

## Checking an entity's tags

In a server hook that gives you an entity and a host, use
`entity.tags(host).is_some_and(|tags| tags.contains("mymod:marked"))` to match a
scoreboard tag without allocating. To read all tags, the
[borrowed view](../../concepts/parts/entity-parts-and-mob-goals.md#entity-tags)
yields UTF-16 slices in saved order. Choose checked or lossy conversion explicitly
when you need UTF-8.

## Opening a container

`player_container_open` runs synchronously after a successful open. Failed opens and opening the player's
own inventory do not notify. Replacing an open menu closes it before announcing the new one.

## Leaving a bed

`player_bed_leave` is a server-side, listen-only notice. It runs before the
sleeping position and pose are cleared, so a listener can still read the player
as sleeping at `pos`; it cannot cancel the wake. For example, a listener recording
why a player left a bed should retain both flags rather than treating every wake
as a manual leave:

| Wake reason | `(wake_immediately, update_level)` |
| --- | --- |
| Morning after skipping the night | `(false, false)` |
| Explicit leave or bed-rule refusal | `(false, true)` |
| Generic stop-sleeping, missing bed or death | `(true, true)` |

`wake_immediately` resets the sleep counter to **0** when true, skipping the
fade-out; false sets it to **100** for the normal fade-out. `update_level` is false
for the morning wake because the sleeping list has already been handled, and
true for the individual wakes above. The same pairs apply to the acting player
and stored guests. Waking that same player again inside the callback does not
emit a second notice.

See the [generated API reference](../../reference/api.md) and the hook's
[caller notes](../../reference/hook-notes.md#player-bed-leave-callers).

## Dropped-item expiry

`item_expire` runs on the server when a dropped item reaches its lifespan (normally 6000 ticks),
before the game decides whether to discard it. Read the stack from `event.stack` and the drop's
position with `host.entity(event.entity).and_then(|entity| entity.position(host))`.

To extend its life, add to `event.extra_life` with `wrapping_add` so contributions combine like
NeoForge's signed 32-bit integers. The game adds that amount to the lifespan with the same
arithmetic, clamps the result to `0..=32766` (`Short.MAX_VALUE - 1` in NeoForge's `ItemEntity.tick`),
and leaves the current age unchanged. The drop survives only if its age is below its new lifespan
and it hasn't otherwise been removed.

To observe actual removals instead, listen to `entity_leave_level` and check for
`event.kind == "minecraft:item"`. It reports departures from any cause, not just expiry. A drop
discarded at expiry leaves before the next drop ticks; extending its life does not fire a leave
notice.

## Physical movement decisions

`entity_move` runs on the server before one physical `Entity.move` invocation,
including its no-physics and stuck-multiplier handling. This hook receives the
loop context, not a `Host`: it decides only the movement delta. `MoveCause`
distinguishes self movement, player packets, pistons, shulker boxes, shulkers and a
moving collider carrying a body (`Carried`, version 2 of the hook);
boats and minecarts use self movement.

A replacement must have finite components; the hook imposes no per-axis magnitude
limit. Each invalid proposal is reported with its mod and discarded whole, including
its cancellation, without disabling the listener. Later listeners see the preceding
accepted delta. An unchanged game delta is not revalidated. Cancellation
skips the invocation: no position, velocity, collision-flag or fall
change, and no displacement notice. An overridden player or vehicle packet move
sends a correction even below the normal distance threshold, for creative players
and from obstructed starts too. A replacement equal to the request retains the
normal vanilla distance rules.

If a listener panics or incurs a phase refusal, its proposed delta and any
change to the cancel flag are discarded. Later eligible listeners see the last
accepted result; a failed callback neither vetoes movement nor clears an earlier
veto. Only a panic disables that mod's callback for this hook for the session.

A mod moves an entity through the same moment with `EntityRef::move_by(host, delta, cause)`
(`Entity.move(cause, delta)`, server only): the collision scan stops it at blocks, the world border and
entities, it steps up, and its ground flag follows. This is not a teleport or a velocity setter, but
collision response can change the body's velocity through block restitution. If the base fall handling
removes the body, no restitution query runs. The `entity_move` hook fires once with `cause`, and a
cancelled move returns `[0.0; 3]`. See the generated reference for which bodies the handle can move
and for the relative-teleport semantics of player moves.

`EntityRef::on_block(host)` uses the block retained by the last support check for players, mobs,
armor stands and nonliving bodies that keep ground support, including items, boats and minecarts.
It does not rerun collision detection. If there is no retained support, it reads the block at
vanilla's feet offset.

`living_travel` runs on both peers, after the player's swim steering or at a
mob's travel entry: the client's own movement fires it too, so a mod installed
on both sides predicts the edit or veto the server applies, and the server's
position still wins. It receives the loop context, not a `Host`, and lets a
listener change the input and velocity or cancel. `TravelMode` names the branch
about to run, including species overrides: an allay or vex reports `Flying`, and
a squid reports `VelocityOnly` in or out of water. A replacement does not change
the branch. A body that does not travel, such as a shulker, fires no hook; neither
does a replayed player tick.

Edits to `input`, edits to `velocity` and cancellation are independent: there is
no per-listener output validator or synthetic magnitude cap. Velocity writes
follow vanilla `Entity.setDeltaMovement`: any finite vector is applied, while a
non-finite vector leaves the previous velocity intact without reverting valid
input edits or cancellation. Input flows through vanilla `LivingEntity.travel`
(`Entity.moveRelative` and `Entity.getInputVector`): vectors with squared length
above `1.0` are normalized, and a non-finite input adds no acceleration without
reverting valid velocity edits or cancellation.

Cancellation skips the branch, its nested moves and flying vertical damping,
without undoing earlier work such as swim steering. Listeners registered with
`#[listener(receive_cancelled)]` still run and may set `cancelled` back to
`false`. If the branch runs and calls `entity_move`, that move can still be
vetoed. A listener panic or phase refusal leaves earlier edits standing and
later eligible listeners running. Only a panic disables the failed callback for
this hook for the session.

For a player, a veto holds the position accepted at the server's travel boundary;
it does not rewind reports already accepted before that tick. While the veto
stands, the server corrects new reports that would move the player away. An
edit that changes the stored velocity or the acceleration the chosen branch
applies sends one correction, even if collision absorbs it, if the player
remains eligible and no correction awaits acknowledgement. The server compares
inputs using that branch's `moveRelative` operation, at the same speed and yaw
as the move. Inputs that have the same effect or are unused by the branch do
not trigger a correction. A separate teleport in the same tick takes priority.
The correction carries the live server position and velocity. The client never
calls this hook.

## Swimming eligibility

`Hooks::can_start_swimming` asks for a Boolean from the local player and the
server's acting player. Return `Some(false)` to refuse or `Some(true)` to allow;
the first answer wins. Return `None` to leave vanilla's answer in place. The
player and `LogicalSide` are in the event, and the shared `Host` is read-only.
Declare `side = "both"` and install the mod on both sides. Base the answer on facts
available to both sides that tick; the server remains authoritative for position.
For example, read the feet slot with `host.living(event.player.entity())` and
return `Some(false)` on `SwimmingCheck::Entry` while a certain pair of boots is
worn. Return `None` for callers your rule does not affect.

`Entry` runs after fluid sampling for a sprinting player who isn't swimming or
flying. The query runs before the passenger check: allowing it still cannot
start swimming while riding. `ShallowWater` runs in local sprint eligibility
when vanilla would not treat the water as shallow but the body is still in a
swimmable fluid. In that case denying the predicate counts as shallow and can
prevent or stop sprinting. An existing swim does not ask the entry question.

## Teleport hooks

`ender_pearl_teleport` covers player-owned and mob-owned pearls. Its `owner` is an
`EntityRef`; listeners can cancel or redirect `target`, and read what the pearl hit
(`hit_entity`, or `hit_block` and `hit_face`, and the point `hit_at`). Player owners retain the
acting-player context and mutable fall `damage`. Mob owners take no pearl damage
and make no endermite roll. Pearl placement does not emit the command-only
`entity_teleport` notification. The pearl is consumed even when teleportation is canceled.

`item_consumption_teleport` covers players and foxes with the random-teleport
consume effect. Its `consumer` is a `LivingRef`, and `consumed` is the stack captured
before use, including count and components. Cancellation ends further teleport
attempts, not consumption. Fox callbacks run before that fox's living AI/movement
and before the next mob ticks, with the full server host. Listener changes to an
actor's item, position or eligibility survive rather than being overwritten by a
stale teleport decision. A consumed stack the API can't carry, such as a saved item
whose id isn't registered, isn't heard: the listeners aren't called and the teleport
goes on as vanilla's.

## Fishing medium

`fishing_medium` lets a mod say that a bobber outside water floats and fishes where it is: lava,
the void under an island, a cloud. The first mod in order to answer decides; later mods are not
asked. The answer is a `FishingMedium` whose `surface` is the world Y the bobber floats at. The
bobber is in the medium in the block of `surface` or below (its y under `floor(surface) + 1`),
so a surface of -64.0 holds y -63.5 but not y -62.5, and negative heights work the same as
positive ones. A `surface` that is not finite is no medium.

On the server the answer also carries a `loot_table` and `lure_ticks`. The catch rolls the named
table, and the wait for a fish is `min + nextInt(max - min + 1)` less the rod's Lure, with `min`
raised to at least 1 and `max` to at least `min`. A table id that is malformed or names no table
the server has rolls vanilla's `minecraft:gameplay/fishing`, with one warning naming the mod and
the id. A client reads only the `surface`: it draws the bobber floating and ignores the table and
the wait.

#### Item use lifecycle

`living_entity_use_item_tick` runs after the may-still-use check. Its `duration`
replaces the remaining-ticks counter. Next come per-tick effects only above zero,
the decrement, and server completion at zero or below.

On the server, canceling Tick or setting duration to zero or below **finishes**
the use: food is eaten or a potion applied, Finish fires, and use state clears.
Setting one runs the effects and finishes in that same tick. This never calls
release-using, so a bow fires nothing. A use-on-release item such as a crossbow
does not finish here: its counter goes negative until release.

On a client, cancellation sets the counter to -1 before decrement and skips effects. The negative countdown continues;
the use remains active until server completion or release, so releasing still
notifies the server.

The neighbouring events differ: canceling Start prevents the use beginning,
but duration zero does begin it. Canceling Stop skips only release-using, not
clearing the use. Finish is not cancellable; its result stack can be replaced.

## Player size

`entity_size` resolves dimensions through a read-only `Host`, as NeoForge's `EntityEvent.Size` does: when an
entity is constructed and in `Entity.refreshDimensions`, which a pose change runs. A player's box takes the
answer for the pose it is in. The pose fit tests of `Player.updatePlayerPose` read `Entity.getDimensions(pose)`,
which fires nothing, so a candidate pose is tested at vanilla's size: a listener's crouching size does not make
the crouching pose fit where vanilla's would not. A player is asked when its pose or vanilla size changed (a
client's own player after its tick, a guest's once per server tick); keep the listener cheap.
