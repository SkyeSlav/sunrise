# Observing advancement progress

Subscribe to `player_advancement_progress` to inspect the whole advancement immediately after
one criterion is granted or revoked, before its completion rewards. `event.progress.criteria`
includes obtained and unobtained criteria; `obtained_at` is the original saved grant date or
`None`. `requirements` lists the OR groups, `completed_requirements` counts satisfied groups,
and `done` is the game's completion result. A player-issued `/reload` mutates and
snapshots the acting player's live progress, including during a nested turn.

Keep `event.progress.clone()` if it is needed later: the owned `AdvancementProgress` snapshot
cannot change the player's progress and is not updated by subsequent events. No-op grants and
revokes do not fire.

## Read or replace a player's saved progress

Use `PlayerRef::advancements(self, host: &dyn Host) -> Option<String>` to get an owned
JSON snapshot from the logical server. It returns `None` when the player's progress is
unavailable, for example on a client or for a stale, displaced or fake player.
`PlayerRef::set_advancements(self, host: &mut dyn ServerHost, snapshot: &str) -> bool`
replaces the whole snapshot for a live connected player, including the integrated
server's player. Neither call opens a save file.

The snapshot uses `PlayerAdvancements.save` JSON. For example:

```json
{
  "example:build_a_bridge": {
    "criteria": { "place_deck": "1970-01-01 01:00:01 +0100" },
    "done": true
  }
}
```

Readback includes the current `DataVersion` and only advancements with obtained
criteria. On import, progress is migrated from the supplied version (or vanilla's
unversioned-save default), and unknown advancements and obsolete criteria are dropped.
`criteria` and `done` are optional, and completion is calculated from the loaded
requirements, not from the input `done`. A grant date uses `yyyy-MM-dd HH:mm:ss Z`;
the numeric offset sets its instant, and each retained instant must fit signed
64-bit epoch milliseconds.

A successful import clears any progress that was omitted; `{}` clears everything.
If the input is invalid or the player has no usable connection, the method returns
`false` and changes nothing. Import is not a grant or revoke: it does not fire
advancement hooks, give rewards, announce a completion, or show a toast. The next
normal advancement update resets only this player's client with its full visible
tree and progress.

## Display icons

An advancement's `display.icon` is an item stack, including its count, damage and
supported component patch. The toast, advancement widgets, tabs and hover draw that stack;
LAN sends it through the same item-stack conversion as inventory synchronization, including
explicit component removals resolved through the peer's component registry.
Remote clients convert it once when receiving the advancement, not each frame.

If a remote icon fails the custom-component permission check (including malformed
nested templates), the advancement remains visible with the same item and count.
The **entire component patch** is discarded: every addition, including damage and
glint overrides; every removal; and cached wire bytes. Item defaults apply instead.

### Trying the icon surfaces

The runnable datapack `crates/sunrise-client/tests/fixtures/advancement-icons`
contains two root advancements with wooden-pickaxe icons, count 3 and damage 7:
`sunrise_test:glint` explicitly enables glint; `sunrise_test:plain` disables it.
Its titles and descriptions use vanilla translations.

Copy the fixture directory into a **fresh test world's** `datapacks/` directory,
then run `/reload`. With cheats/operator permission:

```text
/advancement grant @s only sunrise_test:plain
/advancement grant @s only sunrise_test:glint
```

Watch the two toasts: only the second pickaxe shimmers, both show count 3 and a
partially depleted durability bar. Press **L** to open Advancements, select the
fixture's glint tab, then hover its pickaxe widget. The tab, widget and hover must
all shimmer and retain the count and damage bar; the plain tab must not shimmer.
Repeat on a second Sunrise client connected to the test world's LAN server.
To replay the toast, revoke that advancement and grant it again:

```text
/advancement revoke @s only sunrise_test:glint
/advancement grant @s only sunrise_test:glint
```

The native regression
`game::advancements::icons::tests::advancement_icons_reach_local_and_remote_ui_surfaces`
checks actual GUI draw submissions at 640×480 with GUI scale 1, including the
two permission-failure cases in isolated child processes. The resource-free
`lan::advancements::tests::advancement_icons_roundtrip_count_damage_and_opposite_glint_overrides`
checks the LAN packet, remote stack conversion and byte differences when only
the same icon's glint override changes.
