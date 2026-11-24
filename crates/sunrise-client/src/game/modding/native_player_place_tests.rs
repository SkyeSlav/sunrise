//! `player.place_block` against real LAN guests, through the public player handle, with a
//! literal test for each refusal.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use mod_api::{EntityRef, PlayerWorldRefusal as Refusal, WorldEpoch};

/// The API's value for a block state of the world.
fn api_state(game: &super::Game<'_>, name: &str) -> mod_api::BlockState {
    use super::api_values::TryIntoApi;
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap()
        .try_into_api()
        .unwrap()
}

fn loaded(game: &mut super::Game<'_>) {
    // The write is `level_set_block`'s, which an authoritative server only takes.
    game.options.dedicated = Some(Vec::new());
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
    );
}

/// A bed written for a player gets its head (`setPlacedBy`).
#[test]
#[ignore = "daily"]
fn a_block_placed_as_a_player_runs_set_placed_by() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let bed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:red_bed")
            .unwrap();
        let api_bed = api_state(game, "minecraft:red_bed");
        assert_eq!(
            b.place_block(game, level, [2, 70, 2], api_bed, true),
            Ok(())
        );
        assert_eq!(game.world.block(2, 70, 2), bed);
        let head = game.world.block(2, 70, 1);
        let facts = game.res.facts();
        assert_eq!(
            facts.get(head).name,
            facts.get(bed).name,
            "the head was placed"
        );
    });
}

/// Each refusal is its own name and writes nothing.
#[test]
#[ignore = "daily"]
fn each_place_block_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let stone = api_state(game, "minecraft:stone");
        let empty = game.world.block(3, 70, 3);
        let at = [2, 70, 2];
        assert_eq!(b.place_block(game, level, at, stone, false), Ok(()));
        assert_eq!(
            b.place_block(game, level, at, stone, false),
            Err(Refusal::Unchanged),
            "stone already stands there"
        );
        for pos in [[2, 400, 2], [2, -100, 2], [30_000_000, 70, 2]] {
            assert_eq!(
                b.place_block(game, level, pos, stone, true),
                Err(Refusal::OutOfWorld { pos })
            );
        }
        let far = [1600, 70, 1600];
        assert_eq!(
            b.place_block(game, level, far, stone, true),
            Err(Refusal::NotLoaded { pos: far })
        );
        // A mod cannot make a state the world lacks; the game's own check names one.
        let nothing = sunrise_world::registry::BlockStateId(u32::MAX);
        assert_eq!(
            game.player_place_block(b, level, [3, 70, 3], nothing, true),
            Err(Refusal::UnknownState)
        );
        let old =
            mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
        assert_eq!(
            b.place_block(game, old, [3, 70, 3], stone, true),
            Err(Refusal::StaleLevel)
        );
        let gone = mod_api::__private::game::player_ref(
            EntityRef::new(
                WorldEpoch::new(b.entity().epoch().get() + 1),
                b.entity().id(),
            ),
            b.uuid(),
        );
        assert_eq!(
            gone.place_block(game, level, [3, 70, 3], stone, true),
            Err(Refusal::Stale)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.dead = true)
            .unwrap();
        assert_eq!(
            b.place_block(game, level, [3, 70, 3], stone, true),
            Err(Refusal::Dead)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
    });
}

/// A level that is open but is not the one in place is `OtherLevel`: the check compares the
/// supplied level with the installed one, not with the player's dimension.
#[test]
#[ignore = "daily"]
fn a_level_not_in_place_is_other_level() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let overworld = game.level("minecraft:overworld").unwrap();
        let stone = api_state(game, "minecraft:stone");
        game.enter_level(super::Dim::Nether).unwrap();
        assert_eq!(
            b.place_block(game, overworld, [2, 70, 2], stone, true),
            Err(Refusal::OtherLevel)
        );
    });
}

/// On a client it is `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_place_a_block_as_a_player() {
    with_client_game(|game| {
        let level = mod_api::LevelRef::new(WorldEpoch::new(0), game.dim.key());
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        let stone = api_state(game, "minecraft:stone");
        assert_eq!(
            ghost.place_block(game, level, [2, 70, 2], stone, true),
            Err(Refusal::Client)
        );
    });
}

/// `BlockBehaviour.onPlace` runs for every write, so a part on glass that finds a signal beside
/// it lights itself within the write (`GemLampBlock.onPlace`): by a level write and by a
/// player's placement, with the lit state in the level before any tick. Glass that is not
/// powered stays, and a block with no part is unchanged.
#[test]
#[ignore = "daily"]
fn a_part_lights_itself_in_on_place_for_a_level_write_and_a_player_placement() {
    use mod_platform::__private::part_tests::{block_on_place_game_thread, block_on_places};

    // The fixture installs its mod for the process: run it in a child of its own.
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::native_player_place_tests::a_part_lights_itself_in_on_place_for_a_level_write_and_a_player_placement",
        "SUNRISE_BLOCK_ON_PLACE_CHILD",
        &["place"],
    ) {
        return;
    }
    block_on_place_game_thread();
    with_guests(|game| {
        loaded(game);
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (glass, lit, plain) = (
            state("minecraft:glass"),
            state("minecraft:white_stained_glass"),
            state("minecraft:black_stained_glass"),
        );
        let air = api_state(game, "minecraft:air").block();
        let dimension = game.dim.id().to_owned();
        for z in [2, 6, 10, 14] {
            game.world
                .set(4, 70, z, state("minecraft:redstone_block"))
                .unwrap();
        }
        assert!(block_on_places().is_empty());

        // A level write beside the redstone block: lit at once, with no tick run.
        assert!(game.native_set_block(&dimension, [5, 70, 2], glass, 3));
        assert_eq!(game.world.block(5, 70, 2), lit);
        assert_eq!(block_on_places(), [([5, 70, 2], air, false)]);

        // A player's placement beside the redstone block.
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let api_glass = api_state(game, "minecraft:glass");
        assert_eq!(
            b.place_block(game, level, [5, 70, 6], api_glass, true),
            Ok(())
        );
        assert_eq!(game.world.block(5, 70, 6), lit);
        assert_eq!(block_on_places(), [([5, 70, 6], air, false)]);

        // A block with no part is unchanged beside the same redstone block.
        assert!(game.native_set_block(&dimension, [5, 70, 10], plain, 3));
        assert_eq!(game.world.block(5, 70, 10), plain);
        assert!(block_on_places().is_empty());

        // The part hears a write with no signal and leaves the glass as it is.
        assert!(game.native_set_block(&dimension, [8, 70, 14], glass, 3));
        assert_eq!(game.world.block(8, 70, 14), glass);
        assert_eq!(block_on_places(), [([8, 70, 14], air, false)]);
    });
}
