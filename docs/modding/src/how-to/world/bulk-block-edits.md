# Typed bulk block edits

Use complete `BlockState` values in `BlockEdit.palette`. Construct them with
`BlockState::of(&host.block_states(), block, constraints)` (or change an existing state
with `with`). `filters` contains owned `StatePredicate` values. `StatePredicate::block`
validates property names and values strictly; `StatePredicate::tag` retains arbitrary
property requirements, including for an empty tag, and rejects duplicate properties.

A predicate requires block identity or tag membership and every supplied property.
Omitted properties are wildcards, never default-state requirements. An unknown or
missing property on a tag member never matches. Sparse, non-Cartesian state tables
are supported: a partial predicate does not need to form a complete state with defaults.

All palette entries, predicates and run indices are prepared before any mutation.
NBT is not representable. At most 65,536 filters are allowed. Unloaded chunks and
out-of-height portions of runs are skipped. Edits preserve strict block-entity behavior,
drop no contents from replaced block entities, notify no neighbors and send whole changed chunks.
Inside a transaction in the level in place, the new states and entities are visible immediately.
Abort quietly restores both, along with the connection group each replaced block had. Only the
outermost commit in that level publishes the changed chunks, with their resulting connection groups.
When a bulk edit and a `LevelRef::set_block` write touch the same position, they replay in
call order. Only the ordinary write can spill contents or notify neighbors.

## Read column palettes

`ColumnBlocks.palette` also contains complete `BlockState` values. States of the
same block with different properties have distinct entries. Only distinct numeric
states are converted to API values, not each block in the returned columns.

`ColumnBlocks.water_colors` carries one `0xRRGGBB` server-side biome JSON water
colour (`BiomeSpecialEffects.waterColor`) per column at the column's top (`0` where
unloaded). Colormap-derived grass and foliage tints are client-side only. See
[Read blended biome tints on the client](biomes.md#read-blended-biome-tints-on-the-client)
for `ClientHost::biome_tint(pos, resolver)` and the equivalent read inside a block tint callback.

## Query terrain before loading chunks

`LevelRef::detached_generator(host)` returns a thread-safe `DetachedChunkGenerator` bound to an
open server level's generator, without creating, holding, or loading chunks. Clone and move it
onto background worker threads to avoid blocking the server tick.

- `sample_surface_columns(columns)` returns one optional `TerrainColumn` per input pair `(x, z)`,
  in input order, without loading chunks:
  - `height` is one above the highest solid block, skipping fluids, like OceanFloor. The scan
    starts at the level top, so the Nether returns its bedrock roof, not a floor.
  - `block` is that surface's `BlockKey`; `water` is one above the highest fluid over it,
    or absent on dry land. `biome` names the biome in the world's table, including data pack biomes.
  - Void columns and worlds without their own generator return `None`. Custom generators that
    do not provide a noise surface also have no answer.
  - Features (trees, ores, lakes and decorations) and slope rules are not included.
- `generate_chunk(pos, stage)` generates a full in-memory `ChunkSnapshot` up to `GenerationStage`
  (`Biomes`, `Noise`, `Surface`, `Carvers`, `Features`, `Light`) without mutating live chunks or
  saves. `Biomes..=Carvers` match live worldgen bit-for-bit (`has_light() == false`); `Features`
  and `Light` decorate over a detached 3x3 scratch neighbourhood, and only `Light` runs the light
  engine (`has_light() == true`).
- Once the level closes, remaining handles return `Err(GenerationError::Unavailable)` and release
  their worker state immediately.

Mods only receive `TerrainColumn`; it is non-exhaustive and constructed by the game.

#### Upgrading one saved compound

When a mod imports a schematic, quest book or other saved data, pass the compound and the
`DataVersion` of its source to `host.upgrade_nbt(kind, data_version, nbt)`. This runs the game's
own data fixers through the current data version. The method is available on both sides and
needs no world. It does not stamp a `DataVersion` or run fixes for an enclosing chunk or file.

Choose an `NbtDataType` for what the compound represents: `ItemStack`, `BlockState`,
`BlockEntity`, `Entity` or `EntityTree`. For example, an importer can upgrade a saved stack
with `host.upgrade_nbt(NbtDataType::ItemStack, source_version, stack_nbt)?` before decoding it
with `ItemStack::from_nbt`. For entities, choose `EntityTree` if the root's riders or
passengers should also be upgraded; `Entity` does not fix the root's riding tree. Vanilla's
standalone entity and block-entity fixers can retain old root keys alongside the upgraded fields.

Unknown mod IDs are preserved, though generic format changes like the item-stack count and
components still apply. A well-formed compound from the current game version is unchanged.
Data older than the first schema is read with that schema, as in vanilla. Pass `-1` for
unversioned data.

`NbtUpgradeError` distinguishes a future source version, a malformed compound (for example,
one missing its `id`, called `Name` on block states before DataVersion 5006), and legacy
tables that a fix needed but could not read. `TooDeep` and `UpgradedTooDeep` distinguish
input and output past vanilla's NBT read depth. This counts the root at level zero and includes
wrapper compounds when a mixed list is encoded. The upgrade has no size or node cap.
