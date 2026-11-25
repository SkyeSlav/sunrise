//! Daily integration tests for `LevelRef::move_blocks` (#5577).
use super::{Dim, Game, native_entity_refs::tests::with_game};
use mod_api::{
    BlockChangedEvent, BlockTransform, BlockUpdateFlags, BoundingBox, Hooks, LevelHost,
    MoveBlocksError, MovedBlocks, Registrar, RegistrationContext, StructureMirror,
    StructureRotation, StructureTransform, TransformRefusal,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{
    chunk::{Chunk, block_entity::ModBlockEntity},
    containers::BlockEntity,
    items::ItemStack,
};
use sunrise_core::{
    nbt::Tag,
    position::{BlockPos, ChunkPos},
    ticks::TickPriority,
};
use sunrise_mod_runtime::attachments::Attachments;
use sunrise_world::registry::BlockStateId;

fn load_chunk(game: &mut Game<'_>, cx: i32, cz: i32) {
    game.world
        .chunks
        .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
    let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(cx, cz));
}

fn load_ticking_chunk(game: &mut Game<'_>, cx: i32, cz: i32) {
    // `ChunkMap.prepareTickingChunk` (`ChunkMap::is_ticking_ready`): a ticking chunk must be
    // lit and surrounded by all eight neighbour chunks.
    for dx in -1..=1 {
        for dz in -1..=1 {
            let mut chunk = Chunk::empty(cx + dx, cz + dz);
            chunk.lit = true;
            game.world
                .chunks
                .insert((cx + dx, cz + dz), Arc::new(chunk));
        }
    }
    let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(cx, cz));
}

fn state(game: &Game<'_>, name: &str) -> BlockStateId {
    game.pack.assets.registry.default_state(name).unwrap()
}

fn resolve(game: &Game<'_>, name: &str, props: &[(&str, &str)]) -> BlockStateId {
    game.pack
        .assets
        .registry
        .resolve(&sunrise_world::chunk::BlockState {
            name: name.into(),
            properties: props
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        })
        .unwrap()
}

struct BlockChangedLog(Arc<Mutex<Vec<([i32; 3], BlockUpdateFlags)>>>);

#[mod_api::__subscriptions]
impl Hooks for BlockChangedLog {
    fn block_changed(&self, _: &dyn LevelHost, event: &mut BlockChangedEvent) {
        self.0
            .lock()
            .unwrap()
            .push(([event.pos.x, event.pos.y, event.pos.z], event.flags));
    }
}

struct RefuseModBlockEntityTransform;

#[mod_api::__subscriptions]
impl Hooks for RefuseModBlockEntityTransform {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.block_entity_type("refuses")
            .transform(|_: &mut mod_api::Compound, _: StructureTransform| {
                Err(TransformRefusal::new("test refusal"))
            })
            .build();
    }
}

#[test]
#[ignore = "daily"]
fn write_order_and_flags_match_clone_move_including_overlap() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "move_log",
        Box::new(BlockChangedLog(log.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            load_chunk(game, 0, 0);
            let overworld = game.level(game.dim.id()).unwrap();
            let stone = state(game, "minecraft:stone");
            let chest = state(game, "minecraft:chest");
            let torch = state(game, "minecraft:torch");
            let air = game.world.air;

            assert_eq!(
                game.level_set_block(overworld, [0, 64, 0], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                game.level_set_block(overworld, [1, 64, 0], chest, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert_eq!(
                game.level_set_block(overworld, [2, 64, 0], torch, BlockUpdateFlags::NONE),
                Ok(true)
            );
            log.lock().unwrap().clear();

            // Overlapping same-level move by +1 along X.
            let moved = overworld
                .move_blocks(
                    game,
                    overworld,
                    BoundingBox::new([0, 64, 0], [2, 64, 0]),
                    [1, 0, 0],
                    BlockTransform::default(),
                    false,
                )
                .unwrap();
            assert_eq!(moved, MovedBlocks::new(3, 1));

            let skip_all = BlockUpdateFlags::from_bits_retain(2 | 816);
            let all = BlockUpdateFlags::from_bits_retain(3);
            let clients = BlockUpdateFlags::from_bits_retain(2);
            let expected = vec![
                // Clear source: barrier (2|816), other blocks first then solid/be.
                ([2, 64, 0], skip_all),
                ([0, 64, 0], skip_all),
                ([1, 64, 0], skip_all),
                // Clear source: air (3).
                ([2, 64, 0], all),
                ([0, 64, 0], all),
                ([1, 64, 0], all),
                // Destination barriers (2|816) in reverse of [solid, be, other].
                ([3, 64, 0], skip_all),
                ([2, 64, 0], skip_all),
                ([1, 64, 0], skip_all),
                // Destination states (2) in [solid, be, other] order.
                ([1, 64, 0], clients),
                ([2, 64, 0], clients),
                ([3, 64, 0], clients),
            ];
            assert_eq!(*log.lock().unwrap(), expected);
            assert_eq!(game.world.block(0, 64, 0), air);
            assert_eq!(game.world.block(1, 64, 0), stone);
            assert_eq!(game.world.block(2, 64, 0), chest);
            assert_eq!(game.world.block(3, 64, 0), torch);
        });
    });
}

#[test]
#[ignore = "daily"]
fn chest_with_attachment_moves_across_levels_as_same_holder_without_drop_or_duplicate() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        load_chunk(game, 0, 0);
        let back = game.enter_command_level(Dim::Nether).unwrap();
        load_chunk(game, 0, 0);
        game.enter_command_level(back).unwrap();

        let overworld = game.level("minecraft:overworld").unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        let chest = state(game, "minecraft:chest");
        let air = game.world.air;
        let from = [2, 64, 2];
        let to = [6, 64, 6];

        assert_eq!(
            game.level_set_block(overworld, from, chest, BlockUpdateFlags::NONE),
            Ok(true)
        );
        game.world.block_entities.insert(
            (from[0], from[1], from[2]),
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:diamond", 5))]),
        );
        let Tag::Compound(saved_attachments) = Tag::compound([(
            "move_test",
            Tag::compound([("mark", Tag::compound([("value", Tag::Int(99))]))]),
        )]) else {
            unreachable!()
        };
        game.world.block_entities.restore_attachments(
            (from[0], from[1], from[2]),
            Attachments::from_nbt(saved_attachments.clone()),
        );

        let moved = overworld
            .move_blocks(
                game,
                nether,
                BoundingBox::new(from, from),
                [4, 0, 4],
                BlockTransform::default(),
                false,
            )
            .unwrap();
        assert_eq!(moved, MovedBlocks::new(1, 1));
        assert_eq!(game.world.block(from[0], from[1], from[2]), air);
        assert!(
            game.world
                .block_entities
                .get(&(from[0], from[1], from[2]))
                .is_none()
        );
        assert!(
            game.world
                .block_entities
                .attachments((from[0], from[1], from[2]))
                .is_none()
        );
        assert!(game.dropped.is_empty(), "chest contents must not drop");

        let back = game.enter_command_level(Dim::Nether).unwrap();
        assert_eq!(game.world.block(to[0], to[1], to[2]), chest);
        let diamonds = match game.world.block_entities.get(&(to[0], to[1], to[2])) {
            Some(BlockEntity::Chest(slots)) => slots[0].as_ref().map(|s| s.count),
            _ => None,
        };
        assert_eq!(diamonds, Some(5));
        assert_eq!(
            game.world
                .block_entities
                .attachments((to[0], to[1], to[2]))
                .map(|a| a.try_to_nbt().unwrap()),
            Some(saved_attachments)
        );
        assert!(game.dropped.is_empty());
        game.enter_command_level(back).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn pending_block_tick_fires_at_destination_on_scheduled_game_tick() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        load_ticking_chunk(game, 0, 0);
        let back = game.enter_command_level(Dim::Nether).unwrap();
        load_ticking_chunk(game, 0, 0);
        game.enter_command_level(back).unwrap();

        let overworld = game.level("minecraft:overworld").unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        let lamp_lit = resolve(game, "minecraft:redstone_lamp", &[("lit", "true")]);
        let lamp_unlit = resolve(game, "minecraft:redstone_lamp", &[("lit", "false")]);
        let lamp_block = game.res.behaviours().block(lamp_lit);

        let from = [2, 64, 2];
        let to = [6, 64, 2];
        assert_eq!(
            game.level_set_block(overworld, from, lamp_lit, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let start_time = game.world.game_time;
        game.block_ticks.schedule_block(
            start_time,
            BlockPos::new(from[0], from[1], from[2]),
            lamp_block,
            4,
            TickPriority::Normal,
        );

        let moved = overworld
            .move_blocks(
                game,
                nether,
                BoundingBox::new(from, from),
                [4, 0, 0],
                BlockTransform::default(),
                false,
            )
            .unwrap();
        assert_eq!(moved, MovedBlocks::new(1, 0));

        let back = game.enter_command_level(Dim::Nether).unwrap();
        assert_eq!(game.world.block(to[0], to[1], to[2]), lamp_lit);
        assert!(
            game.block_ticks
                .has_block_tick(BlockPos::new(to[0], to[1], to[2]), lamp_block)
        );
        for dt in 1..4 {
            game.world.game_time = start_time + dt;
            game.tick_scheduled().unwrap();
            assert_eq!(
                game.world.block(to[0], to[1], to[2]),
                lamp_lit,
                "tick must not fire early at +{dt}"
            );
        }
        game.world.game_time = start_time + 4;
        game.tick_scheduled().unwrap();
        assert_eq!(
            game.world.block(to[0], to[1], to[2]),
            lamp_unlit,
            "pending tick must fire at destination on the scheduled game tick"
        );
        game.enter_command_level(back).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn unloaded_chunk_refuses_with_not_loaded_and_both_levels_unchanged() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        load_chunk(game, 0, 0);
        // Open and park the Nether without loading any chunks so `game.level` can resolve it
        // while destination chunk (2, 0) remains unloaded.
        let back = game.enter_command_level(Dim::Nether).unwrap();
        game.enter_command_level(back).unwrap();
        let overworld = game.level("minecraft:overworld").unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        let stone = state(game, "minecraft:stone");

        assert_eq!(
            game.level_set_block(overworld, [2, 64, 2], stone, BlockUpdateFlags::NONE),
            Ok(true)
        );
        // Destination chunk (2, 0) in nether is unloaded.
        let err = overworld.move_blocks(
            game,
            nether,
            BoundingBox::new([2, 64, 2], [2, 64, 2]),
            [30, 0, 0],
            BlockTransform::default(),
            false,
        );
        assert_eq!(err, Err(MoveBlocksError::NotLoaded { level: nether }));
        assert_eq!(game.world.block(2, 64, 2), stone);

        // Source chunk (2, 0) in overworld is unloaded.
        let err = overworld.move_blocks(
            game,
            overworld,
            BoundingBox::new([32, 64, 2], [32, 64, 2]),
            [-30, 0, 0],
            BlockTransform::default(),
            false,
        );
        assert_eq!(err, Err(MoveBlocksError::NotLoaded { level: overworld }));
        assert_eq!(game.world.block(2, 64, 2), stone);
    });
}

#[test]
#[ignore = "daily"]
fn refused_block_entity_returns_block_entity_refused_and_changes_nothing() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "refuse_be",
        Box::new(RefuseModBlockEntityTransform),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            load_chunk(game, 0, 0);
            let back = game.enter_command_level(Dim::Nether).unwrap();
            load_chunk(game, 0, 0);
            game.enter_command_level(back).unwrap();

            let overworld = game.level("minecraft:overworld").unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let stone = state(game, "minecraft:stone");
            let from = [2, 64, 2];
            assert_eq!(
                game.level_set_block(overworld, from, stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            game.world.block_entities.insert(
                (from[0], from[1], from[2]),
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "refuse_be:refuses".into(),
                    Default::default(),
                ))),
            );

            let err = overworld.move_blocks(
                game,
                nether,
                BoundingBox::new(from, from),
                [4, 0, 4],
                BlockTransform::default(),
                false,
            );
            assert_eq!(err, Err(MoveBlocksError::BlockEntityRefused { at: from }));
            assert_eq!(game.world.block(from[0], from[1], from[2]), stone);
            assert!(
                game.world
                    .block_entities
                    .get(&(from[0], from[1], from[2]))
                    .is_some()
            );
            let back = game.enter_command_level(Dim::Nether).unwrap();
            assert_eq!(game.world.block(6, 64, 6), game.world.air);
            game.enter_command_level(back).unwrap();
        });
    });
}

#[test]
#[ignore = "daily"]
fn rotated_move_turns_real_oak_stairs_facing() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        load_chunk(game, 0, 0);
        let overworld = game.level(game.dim.id()).unwrap();
        let stairs_north = resolve(
            game,
            "minecraft:oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        );
        assert_eq!(
            game.level_set_block(overworld, [2, 64, 2], stairs_north, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let moved = overworld
            .move_blocks(
                game,
                overworld,
                BoundingBox::new([2, 64, 2], [2, 64, 2]),
                [4, 0, 0],
                BlockTransform::new(
                    StructureRotation::Clockwise90,
                    StructureMirror::None,
                    [0, 0, 0],
                ),
                false,
            )
            .unwrap();
        assert_eq!(moved, MovedBlocks::new(1, 0));
        assert_eq!(game.world.block(2, 64, 2), game.world.air);
        assert_eq!(
            overworld.block_property(game, [6, 64, 2], "facing"),
            Some("east")
        );
    });
}
