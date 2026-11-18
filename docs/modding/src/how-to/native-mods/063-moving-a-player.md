<!-- chapter: Moving a player -->
<!-- separator: 1 -->

### Moving a player

`player.entity().move_by(host, delta, cause)` (`EntityRef::move_by`, the call that moves any other entity, on a
`&mut dyn ServerHost`) moves a player by `delta` through the level as `Entity.move` would: blocks, other entities' colliders and the player's
collision exclusions stop it, and a step up is taken. It returns the displacement made. As for every entity, a
`delta` beyond 64 blocks on an axis is refused. The `entity_move` hook hears it once, for
the player and with `cause`, and a listener may replace or cancel it as for any other entity (a cancelled move is
`Some([0.0; 3])` and the client is told nothing); `cause` changes only what the hook hears, not the physics.

The server's copy moves at once, and the player's client is sent a relative teleport: it adds the displacement to where
it is when the packet arrives and keeps its motion and look, so what it walked in the meantime is kept. Until it
confirms, which a client does at the start of its next frame, the server drops the position of the player's own reports
(their rotation is kept, and nothing is counted against the player).

So `move_by` is for a one-off move, a launch or a pull, and not for every tick: a development build logs a warning when
one player is moved on 20 ticks in a row. For motion on every tick, use `player.entity().push(host, delta)` from the
server half of the mod, or move the player from the client half of the mod.

A player on a moving collider stays on it when the target is still over it, and lets go otherwise. The call is refused
(`None`, nothing moves) for a value that is not finite, a target outside the world, a player that is riding something,
a sleeper, a player without a connection, or a move the collision query cannot answer.
