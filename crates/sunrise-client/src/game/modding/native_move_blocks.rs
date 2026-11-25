//! Moving blocks, their block entities and scheduled block ticks between levels (#5577).
//! Follows vanilla `/clone … move` (`CloneCommands.clone` with `Mode.MOVE`) step for step,
//! with `StructureTemplate.placeInWorld` rotation and mirroring.
use super::{
    Dim, Game,
    native_structures::{engine_mirror, engine_rotation},
    piston_carry_state::{PositionChange, land_position_data, take_position_data},
};
use mod_api::{
    BlockTransform, BoundingBox, LevelRef, MoveBlocksError, MovedBlocks, StructureTransform,
    TransactionError,
};
use std::collections::{HashMap, VecDeque};
use sunrise_client::{block_entities::Carried, containers::BlockEntity, world::World};
use sunrise_core::{
    nbt::{Compound, JavaString, Tag},
    position::BlockPos,
    ticks::TickArea,
};
use sunrise_world::{
    level::flags as level_flags,
    registry::BlockStateId,
    worldgen::structure::{Mirror, Rotation, StateTransforms, StructureTemplate},
};

struct MoveCell {
    source_pos: [i32; 3],
    dest_pos: [i32; 3],
    state: BlockStateId,
    previous_dest_state: BlockStateId,
    transformed_nbt: Option<Compound>,
}

struct CarriedMove {
    dest_pos: [i32; 3],
    state: BlockStateId,
    carried: Option<Carried>,
    position_data: Compound,
    had_entity: bool,
}

fn map_move_pos(
    pos: [i32; 3],
    origin: [i32; 3],
    shift: [i32; 3],
    mirror: Mirror,
    rotation: Rotation,
    pivot: [i32; 3],
) -> [i32; 3] {
    let rel = std::array::from_fn(|i| pos[i].wrapping_sub(origin[i]));
    let turned = StructureTemplate::transform(rel, mirror, rotation, pivot);
    std::array::from_fn(|i| shift[i].wrapping_add(turned[i]))
}

fn transform_state(
    state: BlockStateId,
    transforms: Option<&StateTransforms>,
    mirror: Mirror,
    rotation: Rotation,
) -> BlockStateId {
    match transforms {
        Some(t) => t.rotate(t.mirror(state, mirror), rotation),
        None => state,
    }
}

fn check_chunks_loaded(
    world: &World,
    box_area: BoundingBox,
    level: LevelRef,
) -> Result<(), MoveBlocksError> {
    for cx in (box_area.min[0] >> 4)..=(box_area.max[0] >> 4) {
        for cz in (box_area.min[2] >> 4)..=(box_area.max[2] >> 4) {
            if !world.chunks.contains_key(&(cx, cz)) {
                return Err(MoveBlocksError::NotLoaded { level });
            }
        }
    }
    Ok(())
}

fn check_block_entity_transform(
    world: &World,
    facts: &sunrise_items::blocks::BlockFacts,
    transforms: &HashMap<String, mod_api::__private::RegisteredBlockEntityTransform>,
    pos: [i32; 3],
    state: BlockStateId,
    structure_transform: StructureTransform,
) -> Result<Option<Compound>, MoveBlocksError> {
    if transforms.is_empty() {
        return Ok(None);
    }
    let key = (pos[0], pos[1], pos[2]);
    let (id, base_nbt) = match world.block_entities.get(&key) {
        Some(BlockEntity::Mod(entity)) => (entity.id().to_owned(), entity.nbt.clone()),
        Some(entity) => {
            let id = entity.vanilla_id().map(str::to_owned).unwrap_or_else(|| {
                sunrise_client::block_entity_types::factory_block(&facts.get(state).name).to_owned()
            });
            let nbt = match world.block_entity_extras(key).source {
                Some(Tag::Compound(map)) => map,
                _ => Compound::new(),
            };
            (id, nbt)
        }
        None => {
            let id = sunrise_client::block_entity_types::factory_block(&facts.get(state).name)
                .to_owned();
            let nbt = match world.block_entity_extras(key).source {
                Some(Tag::Compound(map)) => map,
                _ => Compound::new(),
            };
            (id, nbt)
        }
    };
    let Some(registration) = transforms.get(&id) else {
        return Ok(None);
    };
    if !registration.owner.is_enabled() {
        return Ok(None);
    }
    let mut working = base_nbt;
    let result = registration.owner.call(
        "move_blocks block entity transform",
        "block_entity_transform",
        || {
            registration
                .step
                .transform(&mut working, structure_transform)
        },
    );
    match result {
        Ok(Ok(())) => {
            for reserved in ["id", "x", "y", "z"] {
                working.remove(&JavaString::from(reserved));
            }
            Ok(Some(working))
        }
        Ok(Err(_)) | Err(_) => Err(MoveBlocksError::BlockEntityRefused { at: pos }),
    }
}

impl Game<'_> {
    fn resolve_move_dim(&self, level: LevelRef) -> Result<Dim, MoveBlocksError> {
        if !self.level_handle_current(level) {
            return Err(MoveBlocksError::Transaction(
                TransactionError::CommitRefused,
            ));
        }
        if level.dimension() == self.dim.key() {
            return Ok(self.dim);
        }
        self.levels
            .parked_dims()
            .into_iter()
            .find(|dim| dim.key() == level.dimension())
            .ok_or(MoveBlocksError::Transaction(
                TransactionError::CommitRefused,
            ))
    }

    /// Move blocks, their block entities (as the same holder, with attachments) and pending
    /// scheduled block ticks from `area` in `level` to `to`, displaced by `offset` and turned by
    /// `transform`, following vanilla `/clone … move`.
    pub(super) fn level_move_blocks(
        &mut self,
        level: LevelRef,
        to: LevelRef,
        area: BoundingBox,
        offset: [i32; 3],
        transform: BlockTransform,
        strict: bool,
    ) -> Result<MovedBlocks, MoveBlocksError> {
        if !self.is_logical_server() || self.world_debug {
            return Err(MoveBlocksError::Transaction(TransactionError::Unavailable));
        }
        let rotation = engine_rotation(transform.rotation)
            .ok_or(MoveBlocksError::Transaction(TransactionError::Unavailable))?;
        let mirror = engine_mirror(transform.mirror)
            .ok_or(MoveBlocksError::Transaction(TransactionError::Unavailable))?;
        let from_dim = self.resolve_move_dim(level)?;
        let to_dim = self.resolve_move_dim(to)?;

        let state_transforms = if rotation != Rotation::None || mirror != Mirror::None {
            Some(
                self.state_transforms()
                    .ok_or(MoveBlocksError::Transaction(TransactionError::Unavailable))?,
            )
        } else {
            None
        };
        let barrier = self
            .pack
            .assets
            .registry
            .default_state("minecraft:barrier")
            .ok_or(MoveBlocksError::Transaction(TransactionError::Unavailable))?;
        let air = self.world.air;

        let from_box = BoundingBox::from_corners(area.min, area.max);
        let origin = from_box.min;
        let delta = std::array::from_fn(|i| from_box.max[i].wrapping_sub(origin[i]));
        let shift = std::array::from_fn(|i| origin[i].wrapping_add(offset[i]));
        let c1 = StructureTemplate::transform([0, 0, 0], mirror, rotation, transform.pivot);
        let c2 = StructureTemplate::transform(delta, mirror, rotation, transform.pivot);
        let dest_box = BoundingBox::from_corners(c1, c2).moved(shift);

        // 1. Preflight: both source and destination chunks must be loaded (`hasChunksAt`).
        let (solid_list, block_entities_list, other_blocks_list, clear_blocks_list) = {
            let from_world = self
                .native_level_world_result(level)
                .map_err(|_| MoveBlocksError::Transaction(TransactionError::CommitRefused))?;
            let to_world = self
                .native_level_world_result(to)
                .map_err(|_| MoveBlocksError::Transaction(TransactionError::CommitRefused))?;
            check_chunks_loaded(from_world, from_box, level)?;
            check_chunks_loaded(to_world, dest_box, to)?;

            // 2. Read the source in z, y, x order into three lists and validate block entities.
            let facts = self.res.facts();
            let be_transforms = mod_api::__private::block_entity_transforms();
            let structure_transform = StructureTransform::new(transform.rotation, transform.mirror);
            let mut solid_list = Vec::new();
            let mut block_entities_list = Vec::new();
            let mut other_blocks_list = Vec::new();
            let mut clear_blocks_list = VecDeque::new();

            for z in from_box.min[2]..=from_box.max[2] {
                for y in from_box.min[1]..=from_box.max[1] {
                    for x in from_box.min[0]..=from_box.max[0] {
                        let source_pos = [x, y, z];
                        let dest_pos = map_move_pos(
                            source_pos,
                            origin,
                            shift,
                            mirror,
                            rotation,
                            transform.pivot,
                        );
                        if from_world.carrier_fences.locks_cell(&source_pos)
                            || to_world.carrier_fences.locks_cell(&dest_pos)
                        {
                            return Err(MoveBlocksError::Transaction(
                                TransactionError::CommitRefused,
                            ));
                        }
                        if from_world.block_entities.is_taken(&(x, y, z)) {
                            return Err(MoveBlocksError::BlockEntityRefused { at: source_pos });
                        }
                        if to_world.block_entities.is_taken(&(
                            dest_pos[0],
                            dest_pos[1],
                            dest_pos[2],
                        )) {
                            return Err(MoveBlocksError::BlockEntityRefused { at: dest_pos });
                        }
                        let raw_state = from_world.block(x, y, z);
                        let dest_state =
                            transform_state(raw_state, state_transforms, mirror, rotation);
                        let previous_dest_state =
                            to_world.block(dest_pos[0], dest_pos[1], dest_pos[2]);
                        let state_facts = facts.get(raw_state);
                        let has_be = from_world.block_entities.contains_key(&(x, y, z))
                            || state_facts.block_entity
                            || from_world.block_entities.stateful_entity(raw_state.0);
                        if has_be {
                            let transformed_nbt = check_block_entity_transform(
                                from_world,
                                facts,
                                &be_transforms,
                                source_pos,
                                raw_state,
                                structure_transform,
                            )?;
                            block_entities_list.push(MoveCell {
                                source_pos,
                                dest_pos,
                                state: dest_state,
                                previous_dest_state,
                                transformed_nbt,
                            });
                            clear_blocks_list.push_back(source_pos);
                        } else if !state_facts.solid_render && !state_facts.full_collision {
                            other_blocks_list.push(MoveCell {
                                source_pos,
                                dest_pos,
                                state: dest_state,
                                previous_dest_state,
                                transformed_nbt: None,
                            });
                            clear_blocks_list.push_front(source_pos);
                        } else {
                            solid_list.push(MoveCell {
                                source_pos,
                                dest_pos,
                                state: dest_state,
                                previous_dest_state,
                                transformed_nbt: None,
                            });
                            clear_blocks_list.push_back(source_pos);
                        }
                    }
                }
            }
            (
                solid_list,
                block_entities_list,
                other_blocks_list,
                clear_blocks_list,
            )
        };

        let here = self.dim;
        if self.dim != from_dim && self.enter_command_level(from_dim).is_none() {
            return Err(MoveBlocksError::Transaction(TransactionError::Unavailable));
        }

        // Take out every source block entity as the same holder before clearing the source.
        let mut carried_moves = Vec::with_capacity(block_entities_list.len());
        let mut carry_changes = Vec::with_capacity(block_entities_list.len());
        for cell in &block_entities_list {
            let from_key = (cell.source_pos[0], cell.source_pos[1], cell.source_pos[2]);
            let dest_key = (cell.dest_pos[0], cell.dest_pos[1], cell.dest_pos[2]);
            let raw_state = self.world.block(from_key.0, from_key.1, from_key.2);
            let partner = self
                .pack
                .assets
                .registry
                .state(raw_state)
                .and_then(|s| sunrise_gameplay::chests::chest_connected_pos(s, cell.source_pos))
                .map(|p| (p[0], p[1], p[2]));
            let mut position_data = take_position_data(&mut self.world, from_key);
            let cached = self.hoppers.remove(&from_key).map(|h| h.cooldown);
            let is_hopper = sunrise_client::block_entity_types::factory_block(
                &self.res.facts().get(raw_state).name,
            ) == "minecraft:hopper";
            let cooldown = if is_hopper {
                cached.or_else(|| {
                    position_data
                        .get(&JavaString::from("Source"))
                        .and_then(|t| t.get("TransferCooldown"))
                        .and_then(|t| match t {
                            Tag::Int(n) => Some(*n),
                            _ => None,
                        })
                })
            } else {
                None
            };
            if let Some(n) = cooldown {
                position_data.insert("HopperCooldown".into(), Tag::Int(n));
            }
            let mut carried = self.world.block_entities.take_for_carry(from_key);
            if let Some(updated) = cell.transformed_nbt.clone() {
                if let Some(Carried {
                    entity: BlockEntity::Mod(ref mut entity),
                    ..
                }) = carried
                {
                    entity.nbt = updated;
                } else {
                    position_data.insert("Source".into(), Tag::Compound(updated));
                }
            }
            let had_entity = carried.is_some()
                || !position_data.is_empty()
                || self.res.facts().get(raw_state).block_entity;
            carried_moves.push(CarriedMove {
                dest_pos: cell.dest_pos,
                state: cell.state,
                carried,
                position_data,
                had_entity,
            });
            carry_changes.push(PositionChange::Carry {
                from: from_key,
                to: dest_key,
                partner,
            });
        }
        self.finish_piston_position_changes(carry_changes);

        // 3. Clear the source: barrier with 2 | 816, then air with 3 (or 2 | 816 under strict).
        let default_update_flags = level_flags::UPDATE_CLIENTS
            | if strict {
                level_flags::UPDATE_SKIP_ALL_SIDEEFFECTS
            } else {
                0
            };
        let barrier_flags = default_update_flags | level_flags::UPDATE_SKIP_ALL_SIDEEFFECTS;
        let clear_air_flags = if strict {
            default_update_flags
        } else {
            level_flags::UPDATE_ALL
        };
        for &pos in &clear_blocks_list {
            self.set_block_live(pos, barrier, barrier_flags, false);
        }
        for &pos in &clear_blocks_list {
            self.set_block_live(pos, air, clear_air_flags, false);
        }
        let _ = self.process_level_events();

        // Switch to destination level for steps 4..=8.
        if self.dim != to_dim {
            let _ = self.enter_command_level(to_dim);
        }

        // Combined list: solid, block-entity, other.
        let mut combined = Vec::with_capacity(
            solid_list.len() + block_entities_list.len() + other_blocks_list.len(),
        );
        combined.extend(solid_list);
        combined.extend(block_entities_list);
        combined.extend(other_blocks_list);

        // 4. Destination barriers in reverse order with 2 | 816.
        for cell in combined.iter().rev() {
            self.set_block_live(cell.dest_pos, barrier, barrier_flags, false);
        }

        // 5. Destination states in solid, block-entity, other order with 2 (or 2 | 816 under strict).
        let mut moved_blocks = 0u64;
        for cell in &combined {
            if self.set_block_live(cell.dest_pos, cell.state, default_update_flags, true) {
                moved_blocks += 1;
            }
        }

        // 6. Block entities: land each carried block entity as the same holder, then set state again.
        let mut moved_block_entities = 0u64;
        let mut land_changes = Vec::with_capacity(carried_moves.len());
        for item in carried_moves {
            let dest_key = (item.dest_pos[0], item.dest_pos[1], item.dest_pos[2]);
            if let Some(carried) = item.carried {
                self.world.block_entities.land_carried(dest_key, carried);
                moved_block_entities += 1;
            } else if item.had_entity {
                moved_block_entities += 1;
            }
            land_position_data(&mut self.world, dest_key, &item.position_data);
            if let Some(Tag::Int(cooldown)) =
                item.position_data.get(&JavaString::from("HopperCooldown"))
            {
                self.hoppers.insert(
                    dest_key,
                    super::machines::HopperState::with_cooldown(*cooldown),
                );
            } else {
                self.hoppers.remove(&dest_key);
            }
            if self.world.block_entities.attachments(dest_key).is_some() {
                self.world.attachment_landings.insert(
                    sunrise_mod_runtime::attachments::Holder::BlockEntity(item.dest_pos),
                );
            }
            land_changes.push(PositionChange::Land { at: dest_key });
            self.set_block_live(item.dest_pos, item.state, default_update_flags, false);
        }
        self.finish_piston_position_changes(land_changes);

        // 7. Neighbour updates unless strict, in reverse order of combined.
        if !strict {
            for cell in combined.iter().rev() {
                let at = BlockPos::new(cell.dest_pos[0], cell.dest_pos[1], cell.dest_pos[2]);
                let previous = cell.previous_dest_state;
                self.with_level(|b, level, ticks| {
                    b.update_neighbours_on_block_set(level, ticks, at, previous);
                });
            }
        }
        let _ = self.process_level_events();

        // 8. Pending block ticks copied with LevelTicks.copyAreaFrom; source ticks stay in place.
        let tick_area = TickArea::new(
            BlockPos::new(from_box.min[0], from_box.min[1], from_box.min[2]),
            BlockPos::new(from_box.max[0], from_box.max[1], from_box.max[2]),
        );
        let map_tick_pos = |p: BlockPos| {
            let d = map_move_pos(
                [p.x, p.y, p.z],
                origin,
                shift,
                mirror,
                rotation,
                transform.pivot,
            );
            BlockPos::new(d[0], d[1], d[2])
        };
        if from_dim == to_dim {
            self.block_ticks
                .copy_block_ticks_mapped(tick_area, map_tick_pos);
        } else if let Some(source_ticks) = self.levels.parked_block_ticks_of(level.dimension_id()) {
            self.block_ticks
                .copy_block_ticks_from_mapped(source_ticks, tick_area, map_tick_pos);
        }

        if self.dim != here {
            let _ = self.enter_command_level(here);
        }

        Ok(MovedBlocks::new(moved_blocks, moved_block_entities))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stair with `facing=north/east/south/west` (states 0..=3) turns under `transform_state`
    /// exactly as `BlockState.rotate` and `BlockState.mirror` turn it, and its position moves
    /// around the pivot as `StructureTemplate.transform` does.
    #[test]
    fn a_rotated_move_turns_a_stairs_facing_like_block_state_rotate() {
        // State indices: 0 = north, 1 = east, 2 = south, 3 = west.
        // rotate[s] = [cw90, cw180, ccw90]; mirror[s] = [left_right (N<->S), front_back (E<->W)].
        let transforms = StateTransforms::from_json(
            r#"{
                "rotate": [[1,2,3], [2,3,0], [3,0,1], [0,1,2]],
                "mirror": [[2,0], [1,3], [0,2], [3,1]],
                "full_block": [false, false, false, false],
                "loot_containers": [],
                "block_entities": {}
            }"#,
            4,
        )
        .unwrap();
        let north = BlockStateId(0);
        let east = BlockStateId(1);
        let south = BlockStateId(2);
        let west = BlockStateId(3);

        assert_eq!(
            transform_state(north, Some(&transforms), Mirror::None, Rotation::None),
            north
        );
        assert_eq!(
            transform_state(
                north,
                Some(&transforms),
                Mirror::None,
                Rotation::Clockwise90
            ),
            east
        );
        assert_eq!(
            transform_state(
                north,
                Some(&transforms),
                Mirror::None,
                Rotation::Clockwise180
            ),
            south
        );
        assert_eq!(
            transform_state(
                north,
                Some(&transforms),
                Mirror::None,
                Rotation::Counterclockwise90
            ),
            west
        );
        // Mirror runs before rotate: north mirrored LeftRight -> south, then Clockwise90 -> west.
        assert_eq!(
            transform_state(
                north,
                Some(&transforms),
                Mirror::LeftRight,
                Rotation::Clockwise90
            ),
            west
        );

        // Position transform around pivot [0, 0, 0] with offset [10, 0, 0]:
        // [12, 64, 20] relative to origin [10, 64, 20] is [2, 0, 0];
        // CW90 turns [2, 0, 0] -> [0, 0, 2]; shifted by origin + offset = [20, 64, 20] -> [20, 64, 22].
        let origin = [10, 64, 20];
        let shift = [20, 64, 20];
        assert_eq!(
            map_move_pos(
                [12, 64, 20],
                origin,
                shift,
                Mirror::None,
                Rotation::Clockwise90,
                [0, 0, 0]
            ),
            [20, 64, 22]
        );
    }
}
