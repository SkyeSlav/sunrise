# Client networking

Call `host.disconnect(reason)` through `ClientHost` to leave the joined server or world at the end of the frame, outside hooks. Pass `Some(Text)` to show a disconnect reason or `None` for the normal leave flow. The disconnect screen renders the reason in the player's language, like other translatable text: a translatable component shows its translated text and a plain-text reason is unchanged. The call returns `Result<(), DisconnectError>`: `Unavailable` means no world is joined, it is closing, or a save is pending, and `Text` reports an invalid reason (a malformed or too-deep component is refused by that name and nothing changes). A pending request belongs to its current world epoch and cannot leave a later world. Server code continues to use the separate player disconnect operation.

## Holding outgoing play packets

Call `host.hold_packets()` to start an ordered hold and receive its opaque `HoldId`.
While a hold is active, another `hold_packets()` returns
`HoldError::Limit { packets: 256, bytes: 1_048_576 }` and leaves the first hold intact.
Uncancelled packets from `client_packet_send` queue in creation order;
mod packets accepted by `send_packet` join the queue when their root transaction commits.
`release_held(id)` sends them oldest first and returns the published count. `drop_held(id)`
discards the queue. Neither operation replays the cancellation hook or exposes the packet bodies.

The bounds are **256 packets** and **1,048,576 encoded bytes**, including packet type ids.
Overflow ends the hold and flushes it oldest first, including the packet that exceeded the bound.
Later packets send normally. The next release or drop of that id returns
`HoldError::Limit { packets: 256, bytes: 1_048_576 }`; subsequent calls return `Unknown`.
A failed release returns `HoldError::Send { sent, error }`, using `SendPacketError` for the reason,
and drops the unsent tail.

Disconnect, a protocol incarnation change, or an aborted dimension crossing discards the hold;
its id then returns `Unknown`. Starting without a joined physical play connection returns `NoWorld`.
Network-thread keep-alives, configuration packets, cookies and resource-pack answers bypass holds.
No packet inspection or editing beyond the existing `ClientPacket` view is provided.

## Configuration tasks before play

The server's `register_configuration_tasks` hook runs once per connecting client,
after the built-in registry, tag and resource-pack steps. Append
`ConfigurationTask::new(type_id)` to its task list. Every peer gets these tasks,
including vanilla clients.

`configuration_task_start` runs once for the active type. Use the event's
`ConfigurationConnection` to check `has_channel`, send a registered configuration
payload with `send_payload`, and `finish(host, task_type)`. Keep the connection
capability if a payload reply handler will finish the task later. In a main-thread
payload handler, obtain the server host from `IPayloadContext::server_host`.
Alternatively, `configuration_task_tick` checks the task each server tick and
sets `finished` when it is complete. The next task starts only after completion.

A wrong task type, refused payload send, explicit `fail`, or callback panic closes
that client with the configuration-error reason. Check the channel before sending
to an optional or vanilla peer, then finish immediately or explain why it cannot
join. Sends use the existing negotiated payload transport and publish on transaction
commit. A callback panic aborts its transaction.

There is **no per-task timeout**. A client answering keep-alives can remain in
configuration indefinitely while its task waits. The connection's ordinary idle
and keep-alive checks still apply.

## Typed payloads

A typed payload sends a value of your own type on a registered channel. Its body is the value's
`PayloadCodec` encoding (vanilla's network encoding), with nothing around it, so a peer reads it
as an ordinary registered payload.

Derive `PayloadCodec` for the value, register it in `Hooks::register`, and keep the token the
registrar returns:

```rust,ignore
use mod_api::*;
use std::sync::OnceLock;

#[derive(PayloadCodec)]
struct Counter {
    counter: VarInt,
}

static REPLY: OnceLock<ToClient<Counter>> = OnceLock::new();

fn register(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    // The server answers each counter with the next one.
    view.play_to_server_typed::<Counter>(PayloadId("example:count".into()), |value, cx| {
        let counter = value.counter.0.checked_add(1).ok_or("counter is too large")?;
        let next = Counter { counter: VarInt(counter) };
        REPLY.get().unwrap().reply(cx, &next).map(drop).map_err(|e| format!("{e:?}"))
    });
    let reply = view.play_to_client_typed::<Counter>(
        PayloadId("example:count_reply".into()),
        |value, _| Ok(log::info!("server counted {}", value.counter.0)),
    );
    let _ = REPLY.set(reply);
}
```

- **Registration.** `play_to_server_typed`, `play_to_client_typed` and `play_bidirectional_typed`,
  and their `configuration_*` and `common_*` forms, sit beside the raw methods on the same
  registrar and take its version, `optional()` and `executes_on(...)`. The handler receives the
  decoded value and the connection's `IPayloadContext`. A typed id has one declaration: the same
  id in another protocol refuses load.
- **Tokens.** Registration returns a `ToServer<T>` (client to server) or `ToClient<T>` (server to
  client), both for a bidirectional one. Keep it and send with it; it has a Wire form, so a wasm
  mod keeps it the same way. A token grants nothing on its own: each send checks it.
- **Sending and replying.** `ToServer::send(host, &value)` sends from the client
  (`ClientHost::send_payload_to_server`), `ToClient::send(host, player, &value)` from the server
  (`PlayerRef::send_payload`), and `reply(cx, &value)` on either answers on the connection a
  handler received on. They return the raw sends' results: `Queued` means accepted locally.
  Sending the wrong direction does not compile. To address the single-player owner,
  use the ordinary player ref from the [players service](../entities/server-services-and-players.md).
- **Sending to a group.** A `ToClient<T>` also sends to a set of players without you building
  the set, each checking the token and encoding once, then sending the same bytes to each
  player and returning how many sends were accepted: `send_to_all(host, &value)` (every
  server player), `send_in_level(host, level, &value)`, `send_near(host, level, pos, radius,
  except, &value)` (players in the level, not `except`, strictly closer to `pos` than
  `radius`) and `send_tracking_chunk(host, chunk, &value)` (the chunk's watchers). They
  are NeoForge's `PacketDistributor` `sendToAllPlayers`, `sendToPlayersInDimension`,
  `sendToPlayersNear` and `sendToPlayersTrackingChunk`. A player whose send is refused (no
  channel, gone since the list was read, backpressure) is skipped and not counted; only a
  stale token or an encode failure is an `Err`, before anything is sent.
- **To the players tracking an entity.** `send_tracking_entity(host, entity, and_self, &value)`
  sends to the entity's trackers (`ChunkMap.TrackedEntity.seenBy`, NeoForge's
  `sendToPlayersTrackingEntity`; `EntityRef::trackers(host)` is the same list), and
  also to the entity itself when it is a player and `and_self` is set
  (`sendToPlayersTrackingEntityAndSelf`), each player once. A stale or absent entity has no
  trackers, so nothing is sent and the count is 0.
- **The one-call send underneath.** `PayloadsServerHost::players_send_payload(players, id, bytes)`
  queues the same bytes on each listed player's connection with one boundary call, where
  `player_send_payload` crosses once per player; the group sends above use it. Each connection's
  queue owns its own copy of the bytes. Every player is judged by `player_send_payload`'s rules on
  its own: a player whose send is refused is skipped and not counted, and the result is how many
  were accepted (0 for an empty list). A player listed twice is sent to twice. Like every send,
  it is published when the root transaction commits, and an abort publishes nothing.
- **Schema negotiation.** The derive gives the value type a schema fingerprint (field types in
  order, variant positions; not names), and it is part of the negotiated version. A peer whose
  type has another shape is refused at join when the payload is required; when both sides
  declared it `optional()`, it is absent and a send returns `UnsupportedPeer`.
- **Named errors.** `PayloadSendError::StaleToken` for a token the registrar did not hand out
  as it claims (forged, from another load, read as another type, or its mod off);
  `EncodeFailed` for a value the peer could not decode (a bound or the allocation budget),
  before anything is queued; `WrongProtocol` for a send in a phase the id is not declared in. A
  received body that is not one value of the type closes the session, naming the mod, and the
  handler does not run. A handler that panics is disabled: its transaction rolls back, nothing it
  staged is sent, the session closes, and it is not called again.
- **Raw channel failures stay local.** A raw channel (`play_raw`) carries bytes from any client,
  so a declaring mod's handler that returns `Failed`, or stays busy past its deadline, does not
  close the session: the game logs the mod and the channel, rolls back that delivery's staged
  work (a staged disconnect included), and still delivers the other declaring mods' copies. A
  handler that succeeds and asks to disconnect still disconnects, and so does a refusal by the
  main thread's work queue. A typed payload's failure closes the session as above.
