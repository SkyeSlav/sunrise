# Hooks and the Host

A **hook** is a moment in the game, such as a tick, a block break or a player joining. Implement
that moment on `mod_api::Hooks` to receive its **event** and, where the signature provides one,
a **host**: the game's typed interface for that call. Use a [Part](parts.md) instead when the
behaviour belongs to particular registered objects rather than all matching objects.

![The game calls every mod that implements a hook, and a Part only for its own objects. Both receive an event and
reach the game through the Host, its handles, capabilities and attachments.](../images/hooks-parts.svg)

## Dispatch

| Hook kind | What the listener supplies |
| --- | --- |
| Listen | A reaction to a notice; every subscribed mod is called. |
| Cancellable | `Flow::Cancel` stops the action unless a later listener clears cancellation. |
| Replacement | The first accepted `Some` wins; `None` passes to the next listener. |
| Collection | Each listener can contribute to the event's list. |

Read the signature before choosing a hook: it tells you which side runs it, what you can change,
and whether you are observing a decision or participating in it. For exact signatures and return
rules, see the [generated API reference](../reference/api.md); [Hook notes](../reference/hook-notes.md)
explains how related hooks fit together. For a concrete tour, see [Entity hooks](../how-to/entities/entity-hooks.md).

## Failures and panics

A panic disables only that mod's callback for that hook for the session. The game names the mod
and hook in the log and, on a client, in a [diagnostic toast](../how-to/client/toast-notifications.md#admission-rules)
that mods cannot cancel or rewrite. Ordinary refusals are reported without disabling the callback.

A failed proposal is not a cancellation or replacement. Later eligible listeners see the last
accepted event state, not the original request. [Staged registration](load-phases.md#registration-and-setup)
discards the failed callback's declarations. On a journaled action its failed writes do not join
the action; this is not a rollback of arbitrary external or mod-owned side effects. A failed
[chunk-disk job](../how-to/chunk-data-load.md) cannot publish that job's edits.

## Listener priority and cancellation

`#[listener(priority = 100)]` runs before default-priority listeners. Higher `i32` priorities run
first; the default is zero and ties keep dependency order. This order is fixed at installation.

A listener normally skips an already-cancelled event. Add `receive_cancelled` to inspect it, or set
`event.cancelled = false` and return `Flow::Continue` to let later listeners and the vanilla action
proceed. `receive_cancelled` is only valid on cancellable hooks. These attributes order global hook
listeners, not Parts; see [Parts](parts.md) for their dispatch rules.

Cancellation controls the current action, not all its side effects. For example, cancelling a
projectile impact suppresses the hit without itself discarding the projectile. If a listener
also discards the projectile, a later listener clearing cancellation does not bring it back.

## Events and shared state

The game constructs the generated event and lends it to each listener. The public fields describe
what you may read or edit; the struct is `#[non_exhaustive]`, so use `..` when destructuring. Mutable
scalar fields are written back after dispatch, even after cancellation; mutable lists and other
compound fields are borrowed. Later listeners see accepted edits. When an edited stack-list output
is refused, that field keeps its preceding accepted value; valid scalar edits and cancellation still apply.

Some events lend the live subject instead of handing out a snapshot. Borrow the view only while
using it; a later borrow can fail if the subject is no longer resident. For journaled writes through
both the subject and the host, use the scope's [transaction](transactions.md). For example, a damage
container belongs to one hit: its Post runs after that hit resolves, before the caller resumes. Copy
facts you need later rather than trying to keep the view. To exercise dispatch, see [Testing](testing.md).

Hook and Part methods use `&self` and require `Send + Sync`. The same mod instance serves both
logical sides in single player. A host write may also call another hook of the same mod before it
returns. Release your locks before calling the host: otherwise a re-entrant listener can deadlock.
Use an [attachment](attachments.md) for data that belongs to one game object.

## Which thread runs a hook

Most hooks run on the thread that owns their side: the server thread or the client window's thread.
`thread_local!` is thread-local, not world-local or mod-local. Consult the hook or Part's docs for
exceptions, including these:

- **World generation:** placement and sampling can run concurrently on worker threads; some
  placement also runs on the server thread. These calls get a `WorldgenHost` or no host, not a live
  `Host`. Generation must not depend on which worker reaches your code first.
- **Rendering:** startup can register meshing and tint callbacks that later run on workers without
  a host. Answer from the inputs and immutable registration data, not mutable render state.
- **Loading and chunk I/O:** data callbacks may run on a loader thread. Chunk-disk callbacks get a
  restricted job-local view, not live chunks. See [Load phases and access](load-phases.md).

For ordinary shared state, use synchronized mod fields or publish immutable snapshots. A lock
makes access safe; it does not make an order-dependent algorithm deterministic.

## Level hosts

`Host` exposes services and handles. A handle identifies a live object; using it still requires the
right host and can fail if its target has gone away. A `ServerHost` can make authoritative writes;
a `ClientHost` reads the client's own state and provides client services. Match `HostSide` when a
hook can run on either side. A host can exist even when its world does not.

A restricted level hook lends only the access its signature needs. `LevelHost` is a read view of the
owning level. `LevelRandomHost` adds a loan of the live random stream; `LevelTicksHost` provides
scheduling, and `LevelWriteHost` permits local writes and effects. `HitHost` lends hit-scoped
operations, not block writes. Random draws from a live stream are not undone by an abort; see
[Randomness](randomness.md#borrow-the-live-level-stream).

Reads such as collision, light, signals and block entities use the owning side's available data.
They do not load chunks. Light or a vanilla air result is not proof of residency; check `is_loaded`
when you need a resident position. Treat `None` as unavailable where the reference says a view
cannot answer. A client's placement or shape query must not treat unavailable server-only data
as a server answer. Read-only slot snapshots don't open pending loot; an engine comparator read
through a `LevelRandomHost` can. For concrete examples see
[Camera and collision reads](../how-to/client/camera-and-collision-reads.md),
[Block-state facts](../how-to/world/block-state-facts.md) and [Capabilities](capabilities.md).

Use a [transaction](transactions.md) for related journaled writes, passing the host and context
from the same closure. To save from a hook, request a save at the next settled tick boundary
rather than saving synchronously while a subject can be borrowed. Use the supplied draw context
for drawing: it is scoped to the render call, not a way to reach the live renderer. See
[Model drawing](../how-to/client/model-drawing.md).

## After a hook that can change the world

A listener can remove, move or change objects before the engine applies an action. For decisions
that must still hold, the engine re-resolves the target by identity and checks the facts it used; if
those facts changed, it abandons the decision rather than recomputing it. The hook's own documentation
names the facts it checks. Do not retain a list position or assume a handle remains live across a call
that can dispatch another hook.
