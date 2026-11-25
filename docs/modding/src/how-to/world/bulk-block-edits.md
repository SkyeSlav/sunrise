# Typed bulk block edits

Call `level.edit_blocks(host, &edit)` on an active or parked server `LevelRef`.
Use complete `BlockState` values in `BlockEdit.palette`. Construct them with
`BlockState::of(&host.block_states(), block, constraints)` (or change an existing state
with `with`). `filters` contains owned `StatePredicate` values. Use
`StatePredicate::block(block, constraints)` once the block registry has resolved. It checks
property names and values against the game's block definition, not a caller's metadata.
`StatePredicate::tag(tag, constraints)` retains arbitrary property requirements, including
for an empty tag. Both constructors reject duplicate properties.

A predicate requires block identity or tag membership and every supplied property.
Omitted properties are wildcards, never default-state requirements. An unknown or
missing property on a tag member never matches. Sparse, non-Cartesian state tables
are supported: a partial predicate does not need to form a complete state with defaults.

Construct predicates once and pass them directly. To inspect a predicate, use
`block_key()` or `tag_key()` and borrow `constraints()`. Predicates and edits are
native values, not Wire payloads.

Before any mutation, the game resolves palette states and filter names and checks
run indices; a refusal returns `BlockEditError` with nothing changed. `BlockRun.only` can refer to any existing filter at that index. NBT
is not representable. Unloaded chunks and out-of-height portions of runs are skipped.
Edits preserve strict block-entity behavior, drop no contents from replaced block
entities, notify no neighbors and send whole changed chunks with their resulting connection groups.
Inside a transaction in the edited level, the new states and entities are visible immediately.
Abort restores the original states, resident block entities and connection groups; only the
outermost commit in that level publishes the changed chunks. Without an open transaction in
the edited level, the edit commits immediately, even if another level has a transaction open.
When a bulk edit and a `LevelRef::set_block` write touch the same position, they replay in
call order. Only the ordinary write can spill contents or notify neighbors.

The game's bulk block edit API is paired with `LevelRef::move_blocks` for moving regions with
their block entities and scheduled ticks.

## Moving blocks across levels

`level.move_blocks(host, to, area, offset, transform, strict)` moves the blocks in `area`
(`BoundingBox`) from `level` to `to` (the same level, including overlapping source and
destination boxes, or another open server level), displaced by `offset` and turned by
`transform` (`BlockTransform`: `StructureRotation`, `StructureMirror` and `pivot`, matching
`StructureTemplate.placeInWorld`).

The move follows vanilla's `/clone … move` (`CloneCommands.clone` with `Mode.MOVE`) step for
step, with no area size cap:

1. Preflight checks that every chunk in both source and destination boxes is loaded
   (`MoveBlocksError::NotLoaded { level }`) and that every block entity can be taken out,
   transformed and put back (`MoveBlocksError::BlockEntityRefused { at }`), before writing
   anything to either level.
2. Source cells are read in `z, y, x` order into solid, block-entity and other-block lists;
   source cells are cleared (barrier with `2 | 816`, then air with `3`, or `2 | 816` under
   `strict`).
3. Destination cells receive barriers (`2 | 816`) in reverse order, then the transformed states
   (`2`, or `2 | 816` under `strict`) in solid, block-entity, other order.
4. Every block entity moves once as the same holder with its data, components and attachments
   (never cloned or dropped), and its state is re-applied.
5. Unless `strict`, neighbour updates (`updateNeighboursOnBlockSet`) run in reverse order.
6. Pending scheduled block ticks are copied via `LevelTicks.copyAreaFrom`, preserving remaining
   delay, priority and sub-tick order while leaving source ticks in place.

`block_changed` fires for each `setBlock` write the move makes. A successful move returns
`MovedBlocks { blocks, block_entities }`.

## Stream column heights and blocks

`LevelRef::visit_surface_heights(host, columns, heightmap, visit)` works on both sides
(active and parked server levels, and the client's open level). It streams each column's
loaded height (`Option<i32>`) in input order without loading chunks or enforcing a fixed
column-count cap, stopping early when `visit` returns `ControlFlow::Break(())`.

`LevelRef::visit_columns(host, columns, heightmap, (below, above), visit)` streams loaded
server columns one at a time as borrowed `ColumnSlice<'a>` views (or `None` where the chunk
is not loaded), with no column-count or total-block cap:

- `slice.base` is the column's height on `heightmap`.
- `slice.blocks` holds the complete `BlockState` values from `base - below` up to
  `base + above - 1`, bottom first (air outside the chunk's vertical range). `below + above`
  must lie in `1..=level_height` (`LevelHeightAccessor.getHeight`).
- `slice.water_color` carries the `0xRRGGBB` server-side biome JSON water colour
  (`BiomeSpecialEffects.waterColor`) at the column's top (`base - 1`). Colormap-derived
  grass and foliage tints are client-side only. See
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

The non-exhaustive `TerrainColumn` is supplied by the game.

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
