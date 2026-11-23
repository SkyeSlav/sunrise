# Attachments

An attachment stores typed mod data on an object instead of in a map your mod has to keep in
step with the world. Use `AttachmentType<T>` for the type and policy, and a `Holder` for the
object. Declare its namespaced ID during `Hooks::register`:

```rust,ignore
let _ = registrar.register_attachment_type("example:counter", false);
```

The registrar records refusals even if you ignore the result. Only the namespace owner (by
manifest or `served_by` delegation) can declare the ID, but other packages may use that declared
type. An undeclared use reports the caller and attachment ID. The boolean must agree with the
type's `sync` policy. Declaring a synced type requires a client-compared package; a
`side = "server"` package can use another package's synced type but cannot declare its own.

## Reading and writing

`T` is `Clone + Send + Sync + 'static`. `get` returns a clone and inserts the default when no
value is stored. To inspect without creating a default, use `has` or `get_existing`; `has` can
report a saved entry even before it's decoded. `set` runs the codec, if supplied, before
changing the stored value. The same ID cannot be decoded as two different Rust types.

```rust,ignore
use mod_api::{AttachmentType, Holder};

static COUNTER: AttachmentType<u32> = AttachmentType {
    namespace: "example", name: "counter", default: || 0,
    codec: None, copy_on_death: false, sync: false,
};

// In a server callback with its host, for a resident chunk:
let chunk = Holder::Chunk([0, 0]);
let next = COUNTER.get_existing(host, chunk)?.unwrap_or_default() + 1;
COUNTER.set(host, chunk, next)?;
```

A missing server holder does not create storage. Distinguish a missing or temporarily busy
holder from one whose family does not support attachments at all. See `AttachmentError` and
`AttachmentType` in the [generated API reference](../reference/api.md).

## Holders and persistence

A `Holder` addresses a level, chunk, block entity, non-player entity ID or player UUID in the
relevant storage scope. To retain a world-scoped identity, use `AttachmentHolder`, which carries
the typed handle; it does not keep the object loaded. Mutable level hosts can reach their level,
loaded chunks and block entities. Read-only level hosts cannot access attachments.

Supply an `AttachmentCodec` to persist the value as a compound. Without a codec the value is
lost on unload or restart. Unknown mods' encoded entries survive load and save without being
decoded. Holder data is stored under `sunrise:attachments`, by namespace and name; each level
uses the dimension's `data/sunrise/attachments.dat` file rather than `level.dat`. See
`Holder::Entity` in the [generated API reference](../reference/api.md) for entity persistence
details.

## Querying holders

`host.query(&TYPE)` captures resident holders and their stored value bindings in the host's current
dimension. Rows decode lazily and borrow the captured value; a malformed entry is an error, not
a default. Later writes and unloads appear only in fresh queries. A capture doesn't keep holders
loaded or deep-freeze interior mutability. Traversal is not sorted or stable across structural
changes. For usage and costs, see [Query attachment holders](attachments/querying.md).

On the server, `query_mut` gives explicit write-through operations that recheck residency and
use the ordinary journal. No mutable reference or implicit commit is lent. Wrap a group of edits
in a [transaction](transactions.md) if it must succeed together. A query captured in a transaction
can be used only while its frame is current; requery after it closes. Do not persist or encode
batches, selectors or write tokens.

## Synchronization

Persistence alone does not make a value synced. Set `sync: true` and supply a codec: syncing
uses the same encoded compound, not a separate network codec. Defaults created by `get`, writes,
removals and explicit `sync_data` requests are coalesced at the end of the tick; rolled-back
changes are not sent. Clients receive the current value when they start tracking its holder.

Recipients are the clients that track the entity or chunk, the player and their trackers for
player data, or everyone in the level for level data. The connection must negotiate attachment
sync; vanilla clients get no attachment data. A client reads its local copy through the host.
Client writes affect only that copy, not the server; tracked copies go away with their holders.
Restricted level hosts do not expose client attachment copies. The default takes no holder,
and there is no custom copy handler, per-recipient predicate or partial update. For send limits,
failure outcomes (logged drops vs. queue disconnect) and connection-epoch behavior across
reconfigurations, see `AttachmentType::sync` and `AttachmentType::sync_data` in the
[generated API reference](../reference/api.md). See
[Block entity attachment](attachments/block-entity-attachment.md) for a worked sync example.

## Copying on respawn and conversion

| Replacement | What is copied |
| --- | --- |
| Player respawns after death | Only entries marked `copy_on_death`, which requires a codec. |
| Player returns from the End after credits | Every serializable entry, regardless of `copy_on_death`. |
| A mob converts into a different mob | Every serializable entry; a cancelled conversion copies nothing. |

Copies use the encoded representation, including eligible raw entries from absent mods. The
replacement does not share the old decoded instances. Player copying does not depend on a
`player_clone` listener; after conversion the new mob's spawn supplies its synced data.

## Reading the original player

During `player_clone`, `event.original` lends the old player while `event.player` names the
replacement. Use the original view to read data that wasn't copied, including transient
attachments; a default read there doesn't insert it. Original-player writes can join a host
transaction, but change only the old player. The view cannot outlive the callback.

The replacement already has its final respawn level but isn't yet published in the bulk
attachment index. Requery after placement: an old write token is stale even though the UUID is
reused. See `OriginalPlayer` in the [API reference](../reference/api.md) and the
[Hook notes](../reference/hook-notes.md) for the callback's available views.
