# Capabilities

A capability lets a caller ask an object what it can do without knowing its implementation.
A pipe can ask a chest and a modded machine for `ITEMS` and use either through the same
`ResourceHandler<ItemResource>`. The **provider** decides whether to offer a handler; the
**handler** reads or moves the resource. Selection is read-only, not the transfer itself.

## Register and look up providers

For a block entity, request its type key during registration and offer a handler for its address:

```rust,ignore
r.block_entity_capability(&ITEMS, &pipe, |_, at, _, _, offer| {
    offer.offer(|| Box::new(PipeItems(at)));
});
```

The provider can inspect the supplied level, state, address and block entity. Leave `offer`
untouched to decline. The factory passed to `offer.offer` runs only when the caller needs a
handler. A `Capability` wraps that handler and names its provider through `owner()`.

| Goal | API |
| --- | --- |
| Offer storage by block entity type, including vanilla types | Register with `block_entity_capability`. |
| Offer storage by block type, with or without a block entity | Register with `block_capability`. |
| Get a handler at a position and face | Use `level.capability(host, &ITEMS, pos, side)`; a `None` side is unsided, not all faces. |
| Check for an offer without constructing a handler | Use the read-only `LevelHost::has_item_capability`. |
| Check whether a type is already covered | Use the registrar's coverage queries for a block, entity or item type. |

For block lookups the first offer wins: `_before_game` providers, then the game's provider, then
other providers. Within each mod group, block and block entity type providers run together in
registration order. Disabled mods are skipped. A stale or inactive level cannot lend a handler.

Coverage queries see built-ins and registrations so far, including earlier declarations by the
same mod. They do not run future callbacks or promise that a provider will answer at a specific
position or face. A read-only item probe distinguishes an offer, no offer and an unavailable
position. It does not load chunks or open loot; a client probes its local view, not the server's.
Look up a neighbour again on the next tick, or use a
[kept capability](capabilities/keeping-a-block-capability.md) to handle invalidation explicitly.

Default block storage declarations provide engine-managed item slots, fluid tanks or energy
buffers for mod blocks. Use a provider to extend a vanilla block instead. Item containers can
also target vanilla items. See [Block entity containers](../how-to/content-mods.md#block-entity-containers)
and [Item places and components](capabilities/item-places-and-components.md).

## Proxy metadata

A block capability is non-proxyable by default. An explicit proxyability declaration tells a
forwarding mod whether it can safely pass requests on to another block. Repeating the same value
is allowed; conflicting explicit values refuse registration. Reading the default is not a declaration.
The registrar sees metadata discovered so far; runtime readers see published snapshots.
Proxyability neither changes lookup order nor promises a provider will answer. See
[Declaring a custom capability](capabilities/own-capability.md) for keys with no built-in provider.

## Read and move items

`ItemResource` identifies an item and its components **without a count**. The handler uses the
host it is passed rather than keeping a borrow of the game. Keep state in journal-managed
storage so a failed transfer can restore it.

| Handler operation | What to expect |
| --- | --- |
| `size`, `resource`, `amount` | Read zero-based slots, their optional resource and its amount. |
| `capacity`, `is_valid` | Capacity is a limit, not free space or admission. Enforce acceptance rules on insert. |
| `insert_at`, `extract_at` | Move up to the requested amount at one index using the current transaction. |
| `insert`, `extract` | Let the handler choose the index; the default walks indices in order. |

Use `resource_util::move_resources` rather than moving the two sides independently. It probes
extraction and rolls it back before insertion, then commits a step only if the source can give
exactly what the destination accepted. An overanswer rolls back the whole helper call, even if
the caller later commits the enclosing transaction. Pass capability references to keep provider
names in an overanswer diagnostic. Stacking variants try occupied destination slots first.
See [Capability answer limits](capabilities/answer-limits.md) and the `resource_util` module in
the [generated API reference](../reference/api.md) for scans, wrappers and composition.

To try a transfer without keeping it, use a [transaction](transactions.md) and abort. Pass
`Some(tx)` for the innermost frame, or `None` to open a root when no frame is open. A wrong
context or an unavailable transaction is an error, not a successful zero transfer.

### Hopper and dropper attempts

Vanilla containers, including composter containers and container entities, keep their direct
automation paths. Other blocks can be reached through storage providers. Automation can probe
and roll back an attempt, so never keep transfer state outside the journal. Engine-managed slots
defer their implicit change notice to root commit; explicit notifications are immediate and
survive abort. A provider that ignores the journal can lose an item on a refused transfer.
A composter shows why the journal covers more than inventory slots: its capability ports
journal the block state and a composter-local random stream, and publish fill feedback only
on commit. An aborted probe does not shift the result of the next attempt. For caller-specific
hopper pull/push and dropper rules (including ordinary refusal vs. invalid overanswer, source-slot
reservation before callbacks, and hopper-minecart item-entity fallback), see `ITEMS` in the
[generated API reference](../reference/api.md).

## Keep addresses and values local

Handlers, `BlockSide` and local resource records are not network formats. To send a block
address, send its position, face and canonical dimension name, then resolve the named level on
receipt. Do not send a process-local `DimensionKey`. Use owned `ItemStack`, `FluidStack` or
mod-defined [Wire types](wire-types.md) for payload values, and the registered codec for
[attachments](attachments.md).

For other resource families, see [Block fluids](capabilities/block-fluids.md),
[Block energy](capabilities/block-energy.md) and [Entity energy and fluids](capabilities/entity-energy-and-fluids.md).
