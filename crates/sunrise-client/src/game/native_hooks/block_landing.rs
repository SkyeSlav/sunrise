use super::*;
use sunrise_core::geometry::Vec3;

impl Game<'_> {
    /// `Block.fallOn`: entity `entity_id` landed on `state` at `pos` after falling
    /// `distance` blocks. Writable distance and multiplier feed `causeFallDamage` before
    /// sounds and hurt. A zero multiplier prevents both. A creative or
    /// fall-immune entity keeps its immunity. Players, mobs and dropped items
    /// land here with live `&mut dyn ServerHost`, `LevelRef` and `EntityRef`.
    pub(in crate::game) fn native_fall_on(
        &mut self,
        entity_id: u32,
        state: BlockStateId,
        pos: BlockPos,
        distance: f64,
        damage_multiplier: f32,
    ) -> (f64, f32) {
        if !sunrise_client::block_contact::fall_listens(state) {
            return (distance, damage_multiplier);
        }
        let (Some(level), Some(entity)) = (self.level(self.dim.id()), self.entity(entity_id))
        else {
            return (distance, damage_multiplier);
        };
        let mut answer_distance = distance;
        let mut answer_multiplier = damage_multiplier;
        self.fire_fall_on(super::fire::FallOnCall {
            level,
            entity,
            state,
            pos,
            distance: &mut answer_distance,
            damage_multiplier: &mut answer_multiplier,
        });
        (
            if answer_distance.is_finite() {
                answer_distance
            } else {
                distance
            },
            if answer_multiplier.is_finite() {
                answer_multiplier.max(0.0)
            } else {
                damage_multiplier
            },
        )
    }

    /// `Block.bounceOn`: entity `entity_id` bounced off `state` at `pos` after falling
    /// `fall_distance` blocks. `movement` is the movement the bounce leaves it, before
    /// `GameEvent.BOUNCE` and `Entity.setDeltaMovement`: change it to change what the
    /// entity keeps.
    pub(in crate::game) fn native_bounce_on(
        &mut self,
        entity_id: u32,
        state: BlockStateId,
        pos: BlockPos,
        fall_distance: f64,
        movement: Vec3,
    ) -> Vec3 {
        if !sunrise_client::block_contact::bounce_listens(state) {
            return movement;
        }
        let (Some(level), Some(entity)) = (self.level(self.dim.id()), self.entity(entity_id))
        else {
            return movement;
        };
        let mut answer = [movement.x, movement.y, movement.z];
        self.fire_bounce_on(super::fire::BounceOnCall {
            level,
            entity,
            state,
            pos,
            fall_distance,
            movement: &mut answer,
        });
        if answer.iter().all(|v| v.is_finite()) {
            Vec3::new(answer[0], answer[1], answer[2])
        } else {
            movement
        }
    }
}

#[cfg(test)]
mod landing_crossing_tests {
    use super::*;
    use mod_api::{
        BlockPart, BlockUpdateFlags, BounceOnEvent, FallOnEvent, Hooks, ModEntry, Registrar,
        ServerHost,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_client::chunk::Chunk;

    #[derive(Clone, Default)]
    struct LiveLandingPart {
        seen: Arc<Mutex<Vec<String>>>,
        corrupt_numbers: Arc<Mutex<bool>>,
    }

    #[mod_api::sunrise_part]
    impl BlockPart for LiveLandingPart {
        fn fall_on(&self, host: &mut dyn ServerHost, event: &mut FallOnEvent) {
            let kind = event.entity.kind(host).unwrap().id(host).unwrap();
            self.seen
                .lock()
                .unwrap()
                .push(format!("fall:{kind}:{}", event.distance));
            let dirt = host.block_state_id("minecraft:dirt").unwrap();
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            event
                .level
                .set_block(
                    host,
                    pos,
                    dirt,
                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                )
                .unwrap();
            assert_eq!(event.level.block_state(host, pos), Some(dirt));
            if *self.corrupt_numbers.lock().unwrap() {
                event.distance = f64::NAN;
                event.damage_multiplier = -3.0;
            } else {
                event.distance = 12.0;
                event.damage_multiplier = 0.25;
            }
        }

        fn bounce_on(&self, host: &mut dyn ServerHost, event: &mut BounceOnEvent<'_>) {
            let kind = event.entity.kind(host).unwrap().id(host).unwrap();
            self.seen
                .lock()
                .unwrap()
                .push(format!("bounce:{kind}:{}", event.fall_distance));
            let glass = host.block_state_id("minecraft:glass").unwrap();
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            event
                .level
                .set_block(
                    host,
                    pos,
                    glass,
                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                )
                .unwrap();
            assert_eq!(event.level.block_state(host, pos), Some(glass));
            if *self.corrupt_numbers.lock().unwrap() {
                *event.movement = [f64::NAN, 1.0, 0.0];
            } else {
                *event.movement = [0.25, 0.75, -0.5];
            }
        }
    }

    struct LiveLandingMod(LiveLandingPart);
    #[mod_api::__subscriptions]
    impl Hooks for LiveLandingMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, self.0.clone());
        }
    }

    #[test]
    #[ignore = "daily"]
    fn fall_on_and_bounce_on_lend_live_server_host_and_apply_writes_immediately() {
        crate::game::in_own_process(
            "game::native_hooks::block_landing::landing_crossing_tests::fall_on_and_bounce_on_lend_live_server_host_and_apply_writes_immediately",
            || {
                let part = LiveLandingPart::default();
                let mods: mod_api::Mods = vec![ModEntry::new(
                    "live_landing",
                    Box::new(LiveLandingMod(part.clone())),
                )]
                .into();
                mod_platform::testing::with_mods(mods.clone(), || {
                    crate::game::api_values::install_parts(&mods).unwrap();
                    crate::game::native_hooks::install_block_contact_hooks();
                    crate::game::native_entity_refs::tests::with_game(|game| {
                        let stone = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:stone")
                            .unwrap();
                        let dirt = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:dirt")
                            .unwrap();
                        let glass = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:glass")
                            .unwrap();
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        let pos = BlockPos::new(2, 64, 2);
                        game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
                        let mob_id = game
                            .spawn_mob(
                                "minecraft:pig",
                                [2.5, 65.0, 2.5],
                                false,
                                crate::game::creatures::SpawnSite::Plain,
                            )
                            .unwrap();

                        assert_eq!(
                            game.native_fall_on(mob_id, stone, pos, 5.0, 1.0),
                            (12.0, 0.25)
                        );
                        assert_eq!(
                            game.world.block(pos.x, pos.y, pos.z),
                            dirt,
                            "fall_on write must apply synchronously, not at batch end"
                        );

                        game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
                        assert_eq!(
                            game.native_bounce_on(
                                mob_id,
                                stone,
                                pos,
                                6.0,
                                Vec3::new(0.0, 0.4, 0.0)
                            ),
                            Vec3::new(0.25, 0.75, -0.5)
                        );
                        assert_eq!(
                            game.world.block(pos.x, pos.y, pos.z),
                            glass,
                            "bounce_on write must apply synchronously"
                        );

                        *part.corrupt_numbers.lock().unwrap() = true;
                        assert_eq!(
                            game.native_fall_on(mob_id, stone, pos, 5.0, 1.0),
                            (5.0, 0.0),
                            "NaN distance keeps original; negative multiplier clamps to 0"
                        );
                        assert_eq!(
                            game.native_bounce_on(
                                mob_id,
                                stone,
                                pos,
                                6.0,
                                Vec3::new(0.1, 0.2, 0.3)
                            ),
                            Vec3::new(0.1, 0.2, 0.3),
                            "non-finite bounce movement keeps original"
                        );
                        assert_eq!(
                            part.seen.lock().unwrap().as_slice(),
                            [
                                "fall:minecraft:pig:5",
                                "bounce:minecraft:pig:6",
                                "fall:minecraft:pig:5",
                                "bounce:minecraft:pig:6",
                            ]
                        );
                    });
                });
            },
        );
    }

    #[derive(Clone, Default)]
    struct BatchAndOrderProbe {
        log: Arc<Mutex<Vec<String>>>,
    }

    #[mod_api::sunrise_part]
    impl BlockPart for BatchAndOrderProbe {
        fn fall_on(&self, host: &mut dyn ServerHost, event: &mut FallOnEvent) {
            let farmland = host.block_state_id("minecraft:farmland").unwrap();
            let dirt = host.block_state_id("minecraft:dirt").unwrap();
            let glass = host.block_state_id("minecraft:glass").unwrap();
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            let live_now = event.level.block_state(host, pos);
            if event.state == farmland {
                assert_eq!(
                    live_now,
                    Some(dirt),
                    "farmland trample already turned the block to dirt before farmland's fall_on"
                );
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("farmland_fall:{}", event.entity.id()));
                event
                    .level
                    .set_block(
                        host,
                        pos,
                        glass,
                        BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                    )
                    .unwrap();
                event.damage_multiplier = 0.0;
            } else if event.state == glass {
                assert_eq!(live_now, Some(glass));
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("glass_fall:{}", event.entity.id()));
                event.damage_multiplier = 0.0;
            }
        }

        fn bounce_on(&self, host: &mut dyn ServerHost, event: &mut BounceOnEvent<'_>) {
            let glass = host.block_state_id("minecraft:glass").unwrap();
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            self.log
                .lock()
                .unwrap()
                .push(format!("bounce_on:{}", event.entity.id()));
            event
                .level
                .set_block(
                    host,
                    pos,
                    glass,
                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                )
                .unwrap();
            *event.movement = [0.25, 0.9, -0.25];
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for BatchAndOrderProbe {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            for block in [
                "minecraft:farmland",
                "minecraft:glass",
                "minecraft:slime_block",
            ] {
                let key = r.block_key(block);
                r.block_part(key, self.clone());
            }
        }

        fn vanilla_game_event(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::VanillaGameEventEvent<'_>,
        ) -> mod_api::Flow {
            if event.event == "minecraft:bounce" {
                let level = host.level("minecraft:overworld").unwrap();
                let glass = host.block_state_id("minecraft:glass").unwrap();
                assert_eq!(
                    level.block_state(host, [5, 64, 5]),
                    Some(glass),
                    "bounce_on write must already be live before BOUNCE game event"
                );
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("game_event:{}", event.event));
            }
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn fall_on_after_trample_is_visible_in_batch_and_bounce_on_precedes_bounce_event() {
        crate::game::in_own_process(
            "game::native_hooks::block_landing::landing_crossing_tests::fall_on_after_trample_is_visible_in_batch_and_bounce_on_precedes_bounce_event",
            || {
                let probe = BatchAndOrderProbe::default();
                let mods: mod_api::Mods =
                    vec![ModEntry::new("batch_landing", Box::new(probe.clone()))].into();
                mod_platform::testing::with_mods(mods.clone(), || {
                    crate::game::api_values::install_parts(&mods).unwrap();
                    crate::game::native_hooks::install_block_contact_hooks();
                    crate::game::native_entity_refs::tests::with_game(|game| {
                        game.options.dedicated = Some(Vec::new());
                        game.streaming
                            .insert_remote(&mut game.world, Chunk::empty(0, 0));
                        let reg = game.pack.assets.registry;
                        let farmland = reg.default_state("minecraft:farmland").unwrap();
                        let glass = reg.default_state("minecraft:glass").unwrap();
                        let slime = reg.default_state("minecraft:slime_block").unwrap();
                        game.world.set(1, 64, 1, farmland).unwrap();
                        game.world.set(5, 64, 5, slime).unwrap();
                        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                        let mut first = sunrise_client::mobs::Mob::new(
                            101,
                            kind.clone(),
                            Vec3::new(1.5, 65.1, 1.5),
                            0.0,
                        );
                        first.ai.goals = Default::default();
                        first.ai.targets = Default::default();
                        first.fall_distance = 6.0;
                        first.velocity = Vec3::new(0.0, -0.5, 0.0);
                        let mut second = sunrise_client::mobs::Mob::new(
                            102,
                            kind.clone(),
                            Vec3::new(1.5, 65.1, 1.5),
                            0.0,
                        );
                        second.ai.goals = Default::default();
                        second.ai.targets = Default::default();
                        second.fall_distance = 6.0;
                        second.velocity = Vec3::new(0.0, -0.5, 0.0);
                        let mut bouncer = sunrise_client::mobs::Mob::new(
                            103,
                            kind,
                            Vec3::new(5.5, 65.1, 5.5),
                            0.0,
                        );
                        bouncer.ai.goals = Default::default();
                        bouncer.ai.targets = Default::default();
                        bouncer.fall_distance = 4.0;
                        bouncer.velocity = Vec3::new(0.0, -0.8, 0.0);
                        game.mobs.push(first);
                        game.mobs.push(second);
                        game.mobs.push(bouncer);
                        game.ticked_entities_through = u32::MAX;
                        game.tick_mobs().unwrap();
                        assert_eq!(game.world.block(1, 64, 1), glass);
                        assert_eq!(game.world.block(5, 64, 5), glass);
                        let log = probe.log.lock().unwrap().clone();
                        assert_eq!(
                            log,
                            [
                                "farmland_fall:101",
                                "glass_fall:102",
                                "bounce_on:103",
                                "game_event:minecraft:bounce",
                            ]
                        );
                        let bounced = game.mobs.iter().find(|m| m.id == 103).unwrap();
                        assert!(
                            bounced.velocity.y > 0.5,
                            "modified bounce_on velocity must be committed: {:?}",
                            bounced.velocity
                        );
                    });
                });
            },
        );
    }
}
