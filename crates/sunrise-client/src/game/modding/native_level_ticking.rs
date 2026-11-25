//! Ticking eligibility through the same predicates as the server's tick phases.
use super::Game;
use mod_api::LevelRef;
use sunrise_client::{ticket_levels::ChunkTickets, world::World};

impl Game<'_> {
    fn native_tick_level(&self, level: LevelRef) -> Option<(&World, &ChunkTickets)> {
        if !self.is_logical_server() || !self.level_handle_current(level) {
            return None;
        }
        if level.dimension() == self.dim.key() {
            return Some((&self.world, &self.tickets));
        }
        let parked = self.levels.parked_of(level.dimension_id())?;
        Some((&parked.world, &parked.tickets))
    }

    fn native_tick_position(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        Some(
            self.level_in_world_bounds(level, pos)?
                && self.chunk(level, [pos[0] >> 4, pos[2] >> 4]).is_some(),
        )
    }

    pub(super) fn level_entity_ticking(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        let (_, tickets) = self.native_tick_level(level)?;
        Some(
            self.native_tick_position(level, pos)?
                && tickets.levels().entity_ticking_at(pos[0], pos[2]),
        )
    }

    pub(super) fn level_should_tick_blocks_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        let (world, tickets) = self.native_tick_level(level)?;
        Some(
            self.native_tick_position(level, pos)?
                && sunrise_gameplay::block_entity_ticks::block_ticking_at(
                    &tickets.levels(),
                    &world.entities_pending,
                    pos[0],
                    pos[2],
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game};
    use mod_api::DimensionKey;
    use sunrise_client::ticket_levels::Sources;
    use sunrise_core::position::ChunkPos;

    fn assert_reads(game: &Game<'_>, level: LevelRef, pos: [i32; 3], expected: Option<bool>) {
        assert_eq!(level.entity_ticking(game, pos), expected);
        assert_eq!(level.should_tick_blocks_at(game, pos), expected);
    }

    #[test]
    #[ignore = "daily"]
    fn ticking_reads_follow_live_tickets_bounds_and_residency() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(-1, -1));
            game.tickets.update(Some(Sources {
                players: vec![(-1, -1)],
                view_distance: 2,
                simulation_distance: 2,
                ..Default::default()
            }));
            let before = game.world.chunks.len();
            for pos in [[-1, 64, -1], [-17, 64, -17], [-1, -64, -1], [-1, 319, -1]] {
                assert_reads(game, level, pos, Some(true));
            }
            // Resident x/z and live tickets isolate the height check.
            for pos in [[-1, -65, -1], [-1, 320, -1], [30_000_000, 64, -1]] {
                assert_reads(game, level, pos, Some(false));
            }
            // A missing center chunk inside the live ticket range.
            game.world.chunks.remove(&(-1, -1)).unwrap();
            assert_reads(game, level, [-1, 64, -1], Some(false));
            assert_eq!(game.world.chunks.len(), before - 1);
            game.world.entities_pending.insert((-2, -2));
            assert_eq!(
                level.should_tick_blocks_at(game, [-17, 64, -17]),
                Some(false)
            );
            game.world.entities_pending.clear();
            assert_reads(game, level, [-17, 64, -17], Some(true));
            game.tickets.update(None);
            assert_reads(game, level, [-17, 64, -17], Some(true));
            game.tickets.update(Some(Sources::default()));
            assert_reads(game, level, [-17, 64, -17], Some(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn ticking_reads_refuse_each_unavailable_handle_independently() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.tickets.update(None);
            assert_reads(game, level, [0, 64, 0], Some(true));
            let stale = LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                DimensionKey::OVERWORLD,
            );
            assert_reads(game, stale, [0, 64, 0], None);
            let unopened = LevelRef::new(level.epoch(), DimensionKey::NETHER);
            assert_reads(game, unopened, [0, 64, 0], None);
            game.enter_level(crate::game::Dim::Nether).unwrap();
            assert_reads(game, level, [0, 64, 0], Some(true));
        });
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.tickets.update(None);
            assert_reads(game, level, [0, 64, 0], Some(true));
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert_reads(game, level, [0, 64, 0], None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn parked_level_ticking_reads_reflect_mod_forced_chunks() {
        use mod_api::{ChunkRef, Hooks, ModEntry, Registrar, TicketController, TicketOwner};
        use std::sync::{Arc, Mutex};
        use sunrise_client::chunk::Chunk;

        struct Loader(Arc<Mutex<Option<TicketController>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Loader {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let controller = r.register_ticket_controller("loader", None);
                *self.0.lock().unwrap() = Some(controller);
            }
        }

        let issued = Arc::new(Mutex::new(None));
        let mods = vec![ModEntry::new(
            "parked_tick",
            Box::new(Loader(issued.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                mod_api::testing::with_installed_ticket_controllers(|| {
                    let controller = issued.lock().unwrap().unwrap();
                    game.enter_level(crate::game::Dim::Nether).unwrap();
                    let nether = game.level("minecraft:the_nether").unwrap();
                    for cx in 0..=2 {
                        game.world
                            .chunks
                            .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
                    }
                    assert!(game.chunk_set_mod_forced(
                        ChunkRef::new(nether, [0, 0]),
                        controller,
                        TicketOwner::Block([0, 64, 0]),
                        true,
                        false,
                    ));
                    game.update_chunk_tickets();
                    // Park the Nether while Overworld is in place.
                    game.enter_level(crate::game::Dim::Overworld).unwrap();
                    assert_eq!(game.dim, crate::game::Dim::Overworld);

                    // Inside the mod-forced chunk: both entity and block ticking are enabled.
                    assert_eq!(nether.should_tick_blocks_at(game, [8, 64, 8]), Some(true));
                    assert_eq!(nether.entity_ticking(game, [8, 64, 8]), Some(true));
                    // One chunk away: entity ticking stops immediately; unloaded neighbor does not block-tick.
                    assert_eq!(nether.entity_ticking(game, [24, 64, 8]), Some(false));
                    assert_eq!(nether.should_tick_blocks_at(game, [40, 64, 8]), Some(false));
                    game.levels
                        .parked_world_mut_of("minecraft:the_nether")
                        .unwrap()
                        .chunks
                        .remove(&(1, 0));
                    assert_eq!(nether.should_tick_blocks_at(game, [24, 64, 8]), Some(false));
                });
            });
        });
    }
}
