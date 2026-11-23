// Client reads of vanilla state for mods (#4677), on a window joined to a server through
// memory and fed the server's packets as it would receive them.
use crate::game::{Game, Screen, native_entity_refs::tests::with_client_game};
use mod_api::ClientHost;
use sunrise_client::chunk::Chunk;
use sunrise_protocol::{
    connection::{ConnReader, ConnWriter},
    packets::play::{self, Clientbound as C},
};

/// The window's own player.
const PLAYER: u32 = 7;

fn spawn() -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// The server's teleport that makes the local player, on a stone floor, with the level loaded.
fn stand(game: &mut Game<'_>, teleport: i32) {
    game.streaming
        .insert_remote(&mut game.world, Chunk::empty(0, 0));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 63, z, stone).unwrap();
        }
    }
    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
        teleport,
        position: [0.5, 64.0, 0.5],
        delta: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        relatives: 0,
    }))
    .unwrap();
    let remote = game.remote.as_mut().unwrap();
    remote.loaded_sent = true;
    remote.level_load.finish();
    game.screen = Screen::Playing;
}

/// Joins as entity [`PLAYER`] in the overworld.
fn join(game: &mut Game<'_>) -> (ConnReader, ConnWriter) {
    let connection = super::connect_menu_test(game);
    game.handle_packet(C::Login(play::Login {
        player_id: PLAYER as i32,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 2,
        simulation_distance: 2,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: spawn(),
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
    stand(game, 1);
    connection
}

/// The server adds entity `id` of `kind` beside the player.
fn add(game: &mut Game<'_>, id: u32, kind: &str) {
    let kind = game
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind)
        .unwrap();
    game.handle_packet(C::AddEntity(play::AddEntity {
        id: id as i32,
        uuid: 0x4677_0000 + u128::from(id),
        kind,
        position: [2.5, 64.0, 2.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

/// `EntityRef::tick_count` of entity `id`, read through a client host.
fn tick_count(game: &Game<'_>, id: u32) -> Option<i32> {
    game.entity(id)?.tick_count(game as &dyn ClientHost)
}

/// `Entity.tickCount` on a client is its copy's own counter: `ClientLevel.tickNonPassenger`'s
/// `commonTick` counts each client tick from the entity's arrival, and the new `LocalPlayer`
/// that `handleRespawn` makes starts again at 0.
#[test]
#[ignore = "daily"]
fn a_client_counts_its_copies_ticks_and_a_respawned_player_starts_at_zero() {
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::a_client_counts_its_copies_ticks_and_a_respawned_player_starts_at_zero",
        || {
            with_client_game(|game| {
                let _connection = join(game);
                // Every kind counts, not only those the server read covers.
                let kinds = [
                    (20, "minecraft:pig"),
                    (21, "minecraft:minecart"),
                    (22, "minecraft:oak_boat"),
                    (23, "minecraft:arrow"),
                    (24, "minecraft:armor_stand"),
                    (25, "minecraft:item_display"),
                ];
                for (id, kind) in kinds {
                    add(game, id, kind);
                }
                let counts = |game: &Game<'_>| {
                    [PLAYER, 20, 21, 22, 23, 24, 25].map(|id| tick_count(game, id))
                };
                assert_eq!(counts(game), [Some(0); 7]);
                for _ in 0..3 {
                    game.tick_remote().unwrap();
                }
                assert_eq!(counts(game), [Some(3); 7]);

                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn(),
                    keep: 0,
                }))
                .unwrap();
                stand(game, 2);
                let mut expected = [Some(3); 7];
                expected[0] = Some(0);
                assert_eq!(counts(game), expected);
                game.tick_remote().unwrap();
                let mut expected = [Some(4); 7];
                expected[0] = Some(1);
                assert_eq!(counts(game), expected);
            });
        },
    );
}

/// The count follows the client's entity pass, not the player's motion: an admitted local player
/// counts before `LocalPlayer.tick`'s loaded-only update and while it sleeps, a frozen level
/// still ticks the player and the cart carrying it (`TickRateManager.isEntityFrozen`). A root
/// counts at its own visit, after its `EntityTickEvent.Pre`; `PlayerTickEvent.Pre`, inside the
/// player's tick, already sees the player's new count.
#[test]
#[ignore = "daily"]
fn the_client_count_follows_the_entity_tick_not_the_motion() {
    use mod_api::{EntityTickListenedEvent, EntityTickPreEvent, HostSide, PlayerTickPreEvent};
    use std::sync::{Arc, Mutex};
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::the_client_count_follows_the_entity_tick_not_the_motion",
        || {
            /// The counts each callback read, by entity id.
            struct Seen(Arc<Mutex<Vec<(u32, Option<i32>)>>>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Seen {
                fn entity_tick_listened(
                    &self,
                    _: HostSide<'_>,
                    event: &mut EntityTickListenedEvent,
                ) {
                    event.listened = true;
                }
                fn entity_tick_pre(&self, host: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
                    let HostSide::Client(host) = host else {
                        return;
                    };
                    for &id in event.entities {
                        let count = host.entity(id).and_then(|e| e.tick_count(host));
                        self.0.lock().unwrap().push((id, count));
                    }
                }
                fn player_tick_pre(&self, host: HostSide<'_>, event: &mut PlayerTickPreEvent) {
                    let HostSide::Client(host) = host else {
                        return;
                    };
                    let count = event.player.entity().tick_count(host);
                    self.0
                        .lock()
                        .unwrap()
                        .push((event.player.entity().id(), count));
                }
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![mod_api::ModEntry::new("seen", Box::new(Seen(seen.clone())))].into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let _connection = join(game);
                    add(game, 20, "minecraft:pig");
                    add(game, 21, "minecraft:minecart");
                    let counts = |game: &Game<'_>| [PLAYER, 20, 21].map(|id| tick_count(game, id));
                    // Before the client has loaded: the player counts, its tick does not run.
                    game.remote.as_mut().unwrap().loaded_sent = false;
                    game.tick_remote().unwrap();
                    assert_eq!(counts(game), [Some(1); 3]);
                    game.remote.as_mut().unwrap().loaded_sent = true;
                    // Asleep.
                    game.sleeping = Some([0, 64, 0]);
                    game.tick_remote().unwrap();
                    assert_eq!(counts(game), [Some(2); 3]);
                    game.sleeping = None;
                    // A frozen level: the player and the cart it rides, not the pig.
                    game.handle_packet(C::TickingState(play::TickingState {
                        rate: 20.0,
                        frozen: true,
                    }))
                    .unwrap();
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle: 21,
                        passengers: vec![PLAYER as i32],
                    }))
                    .unwrap();
                    game.tick_remote().unwrap();
                    assert_eq!(counts(game), [Some(3), Some(2), Some(3)]);

                    let seen = seen.lock().unwrap();
                    // The pig's Pre read 0 then 1 (before its visit counts it), the player's
                    // (once loaded) 2 then 3.
                    let of = |id: u32| {
                        seen.iter()
                            .filter(|(seen, _)| *seen == id)
                            .map(|(_, count)| *count)
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(of(20), [Some(0), Some(1)]);
                    assert_eq!(of(PLAYER), [Some(2), Some(3)]);
                });
            });
        },
    );
}

/// A root counts at its own visit, after its `EntityTickEvent.Pre` let it through
/// (`tickNonPassenger`), so a canceled root does not count; a passenger counts before its own
/// tick whatever its Pre answers (`tickPassenger`).
#[test]
#[ignore = "daily"]
fn a_canceled_root_does_not_count_and_a_passenger_does() {
    use mod_api::{EntityTickListenedEvent, EntityTickPreEvent, HostSide};
    use std::sync::{Arc, Mutex};
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::a_canceled_root_does_not_count_and_a_passenger_does",
        || {
            /// Cancels pigs 20 (a root) and 22 (a passenger), and records what each Pre read.
            struct Cancel(Arc<Mutex<Vec<(u32, Option<i32>)>>>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Cancel {
                fn entity_tick_listened(
                    &self,
                    _: HostSide<'_>,
                    event: &mut EntityTickListenedEvent,
                ) {
                    event.listened = true;
                }
                fn entity_tick_pre(&self, host: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
                    let HostSide::Client(host) = host else {
                        return;
                    };
                    for &id in event.entities {
                        let count = host.entity(id).and_then(|e| e.tick_count(host));
                        self.0.lock().unwrap().push((id, count));
                        if id == 20 || id == 22 {
                            event.canceled.push(id);
                        }
                    }
                }
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![mod_api::ModEntry::new(
                "cancel",
                Box::new(Cancel(seen.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let _connection = join(game);
                    for id in [20, 21, 22, 23] {
                        add(game, id, "minecraft:pig");
                    }
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle: 23,
                        passengers: vec![22],
                    }))
                    .unwrap();
                    for _ in 0..2 {
                        game.tick_remote().unwrap();
                    }
                    let counts = [20, 21, 22, 23].map(|id| tick_count(game, id));
                    assert_eq!(counts, [Some(0), Some(2), Some(2), Some(2)]);
                    let seen = seen.lock().unwrap();
                    let of = |id: u32| {
                        seen.iter()
                            .filter(|(seen, _)| *seen == id)
                            .map(|(_, count)| *count)
                            .collect::<Vec<_>>()
                    };
                    // Told together, the family's Pre runs before any of its visits: each reads
                    // the count its last tick left.
                    assert_eq!(of(20), [Some(0), Some(0)]);
                    assert_eq!(of(21), [Some(0), Some(1)]);
                    assert_eq!(of(22), [Some(0), Some(1)]);
                    assert_eq!(of(23), [Some(0), Some(1)]);
                });
            });
        },
    );
}

/// Each entity counts at its own visit, so a callback reading another entity sees the count its
/// visits so far left: told alone, a vehicle's Pre and Post read its passenger's old count (the
/// passenger's ride visit comes after them, and counts it even when its own Pre is canceled),
/// and the first root's callbacks read a later root's old count.
#[test]
#[ignore = "daily"]
fn callbacks_read_other_entities_counts_as_their_visits_left_them() {
    use mod_api::{EntityTickListenedEvent, EntityTickPostEvent, EntityTickPreEvent, HostSide};
    use std::sync::{Arc, Mutex};
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::callbacks_read_other_entities_counts_as_their_visits_left_them",
        || {
            type Seen = Vec<(&'static str, u32, [Option<i32>; 3])>;
            /// Pigs are told alone; each callback reads pigs 20, 21 and 22, and 21's Pre is
            /// canceled.
            struct Read(Arc<Mutex<Seen>>);
            impl Read {
                fn read(&self, host: HostSide<'_>, moment: &'static str, entities: &[u32]) {
                    let HostSide::Client(host) = host else {
                        return;
                    };
                    let counts =
                        [20, 21, 22].map(|id| host.entity(id).and_then(|e| e.tick_count(host)));
                    for &id in entities {
                        self.0.lock().unwrap().push((moment, id, counts));
                    }
                }
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Read {
                fn register(
                    &self,
                    registrar: &mut dyn mod_api::Registrar,
                    _: &mod_api::RegistrationContext,
                ) {
                    registrar.entity_tick_each("minecraft:pig");
                }
                fn entity_tick_listened(
                    &self,
                    _: HostSide<'_>,
                    event: &mut EntityTickListenedEvent,
                ) {
                    event.listened = true;
                }
                fn entity_tick_pre(&self, host: HostSide<'_>, event: &mut EntityTickPreEvent<'_>) {
                    self.read(host, "pre", event.entities);
                    if event.entities.contains(&21) {
                        event.canceled.push(21);
                    }
                }
                fn entity_tick_post(
                    &self,
                    host: HostSide<'_>,
                    event: &mut EntityTickPostEvent<'_>,
                ) {
                    self.read(host, "post", event.entities);
                }
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![mod_api::ModEntry::new("read", Box::new(Read(seen.clone())))].into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let _connection = join(game);
                    // Pig 20 carries pig 21; pig 22 is a later root.
                    for id in [20, 21, 22] {
                        add(game, id, "minecraft:pig");
                    }
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle: 20,
                        passengers: vec![21],
                    }))
                    .unwrap();
                    game.tick_remote().unwrap();
                    let counts = [20, 21, 22].map(|id| tick_count(game, id));
                    assert_eq!(counts, [Some(1), Some(1), Some(1)]);
                    assert_eq!(
                        *seen.lock().unwrap(),
                        [
                            ("pre", 20, [Some(0), Some(0), Some(0)]),
                            ("post", 20, [Some(1), Some(0), Some(0)]),
                            ("pre", 21, [Some(1), Some(1), Some(0)]),
                            ("pre", 22, [Some(1), Some(1), Some(0)]),
                            ("post", 22, [Some(1), Some(1), Some(1)]),
                        ]
                    );
                });
            });
        },
    );
}

/// The two facts `Hud.nextContextualInfoState` reads, as a client mod reads them:
/// `LocalPlayer.jumpableVehicle` (a saddled mount that jumps, not a pig) and
/// `ClientWaypointManager.hasWaypoints` (any waypoint the server tracks for this client).
#[test]
#[ignore = "daily"]
fn the_contextual_bar_facts_read_the_jumpable_mount_and_the_tracked_waypoints() {
    use sunrise_client::locator_bar::ContextualBar;
    use sunrise_protocol::packets::play::{TrackedWaypoint, WaypointId, WaypointTarget};
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::the_contextual_bar_facts_read_the_jumpable_mount_and_the_tracked_waypoints",
        || {
            with_client_game(|game| {
                let _connection = join(game);
                let facts = |game: &Game<'_>| {
                    let host: &dyn ClientHost = game;
                    (host.jumpable_vehicle(), host.has_waypoints())
                };
                assert_eq!(facts(game), (None, false));
                let ride = |game: &mut Game<'_>, vehicle: u32, passengers: Vec<i32>| {
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle: vehicle as i32,
                        passengers,
                    }))
                    .unwrap();
                };
                for (id, kind) in [
                    (30, "minecraft:horse"),
                    (31, "minecraft:pig"),
                    (32, "minecraft:camel"),
                ] {
                    add(game, id, kind);
                    let mount = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
                    mount.extra.set_saddled(true);
                }
                ride(game, 30, vec![PLAYER as i32]);
                let horse = game.entity(30);
                assert!(horse.is_some());
                assert_eq!(facts(game), (horse, false));
                assert_eq!(game.contextual_bar(), ContextualBar::JumpableVehicle);
                // In the second seat the player does not control the horse
                // (`getControlledVehicle`), so it has no jumpable vehicle and no jump bar.
                add(game, 33, "minecraft:zombie");
                ride(game, 30, vec![33, PLAYER as i32]);
                assert_eq!(facts(game), (None, false));
                assert_ne!(game.contextual_bar(), ContextualBar::JumpableVehicle);
                // A saddle only steers a pig.
                ride(game, 30, Vec::new());
                ride(game, 31, vec![PLAYER as i32]);
                assert_eq!(facts(game), (None, false));
                // A camel jumps (dashes) unless it sits (`Camel.canJump`, `refuseToMove`).
                ride(game, 31, Vec::new());
                ride(game, 32, vec![PLAYER as i32]);
                let camel = game.entity(32);
                assert!(camel.is_some());
                assert_eq!(facts(game), (camel, false));
                game.mobs
                    .iter_mut()
                    .find(|m| m.id == 32)
                    .unwrap()
                    .extra
                    .camel_sitting = true;
                assert_eq!(facts(game), (None, false));
                assert_ne!(game.contextual_bar(), ContextualBar::JumpableVehicle);
                ride(game, 32, Vec::new());

                let waypoint = |operation| {
                    C::TrackedWaypoint(TrackedWaypoint {
                        operation,
                        id: WaypointId::Uuid(0x4677),
                        style: "minecraft:default".into(),
                        color: None,
                        target: WaypointTarget::Block([40, 64, 40]),
                    })
                };
                game.handle_packet(waypoint(0)).unwrap();
                assert_eq!(facts(game), (None, true));
                assert_eq!(game.contextual_bar(), ContextualBar::Locator);
                game.handle_packet(waypoint(1)).unwrap();
                assert_eq!(facts(game), (None, false));
            });
        },
    );
}

/// `handleRespawn` admits the new `LocalPlayer` at once: its count reads 0 from then, and the
/// client level ticks it, before the server's first teleport gives it a body.
#[test]
#[ignore = "daily"]
fn a_respawned_player_counts_from_its_admission() {
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::a_respawned_player_counts_from_its_admission",
        || {
            with_client_game(|game| {
                let _connection = join(game);
                game.tick_remote().unwrap();
                game.tick_remote().unwrap();
                assert_eq!(tick_count(game, PLAYER), Some(2));
                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn(),
                    keep: 0,
                }))
                .unwrap();
                assert!(game.player.is_none());
                let me = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), PLAYER);
                let count = |game: &Game<'_>| me.tick_count(game as &dyn ClientHost);
                assert_eq!(count(game), Some(0));
                game.tick_remote().unwrap();
                assert_eq!(count(game), Some(1));
                stand(game, 2);
                assert_eq!(count(game), Some(1));
                game.tick_remote().unwrap();
                assert_eq!(count(game), Some(2));
            });
        },
    );
}

/// `LocalPlayer.jumpableVehicle` asks nothing of the mount's health: a saddled horse the player
/// controls stays its jumpable vehicle, and the jump bar's, while it dies.
#[test]
#[ignore = "daily"]
fn a_dying_mount_stays_the_jumpable_vehicle() {
    use sunrise_client::locator_bar::ContextualBar;
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::a_dying_mount_stays_the_jumpable_vehicle",
        || {
            with_client_game(|game| {
                let _connection = join(game);
                add(game, 30, "minecraft:horse");
                game.mobs
                    .iter_mut()
                    .find(|m| m.id == 30)
                    .unwrap()
                    .extra
                    .set_saddled(true);
                game.handle_packet(C::SetPassengers(play::SetPassengers {
                    vehicle: 30,
                    passengers: vec![PLAYER as i32],
                }))
                .unwrap();
                let horse = game.entity(30);
                assert!(horse.is_some());
                game.mobs.iter_mut().find(|m| m.id == 30).unwrap().health = 0.0;
                assert_eq!((&*game as &dyn ClientHost).jumpable_vehicle(), horse);
                assert_eq!(game.contextual_bar(), ContextualBar::JumpableVehicle);
            });
        },
    );
}

/// NeoForge's `ICommonPacketListener.hasChannel` (#4677): the send test of
/// `send_payload_to_server` for the current phase, against a server that negotiated a
/// mod's optional channel and against a vanilla server that offered nothing.
#[test]
#[ignore = "daily"]
fn has_channel_follows_what_the_server_negotiated() {
    use mod_api::{
        Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadAccepted, PayloadBytes,
        PayloadHandleError, PayloadId, PayloadSendError, Registrar,
    };
    use std::sync::Arc;
    crate::game::in_own_process(
        "game::remote::tests::client_vanilla_reads_tests::has_channel_follows_what_the_server_negotiated",
        || {
            const CHANNEL: &str = "channels:to_server";
            struct Handler;
            impl IPayloadHandler for Handler {
                fn handle(
                    &self,
                    _: PayloadBytes,
                    _: &mut dyn IPayloadContext,
                ) -> Result<(), PayloadHandleError> {
                    Ok(())
                }
            }
            struct Mod;
            #[mod_api::__subscriptions]
            impl Hooks for Mod {
                fn register(
                    &self,
                    registrar: &mut dyn Registrar,
                    _: &mod_api::RegistrationContext,
                ) {
                    registrar
                        .payload_handlers()
                        .registrar("1")
                        .optional()
                        .play_to_server(PayloadId(CHANNEL.into()), Arc::new(Handler));
                }
            }
            let id = PayloadId(CHANNEL.into());
            let other = PayloadId("channels:undeclared".into());
            let mods = vec![ModEntry::new("channels", Box::new(Mod))].into();
            mod_platform::testing::with_mods(mods, || {
                // A server that declares the channel too.
                with_client_game(|game| {
                    assert!(!(&*game as &dyn ClientHost).has_channel(&id));
                    let local = sunrise_client::raw_payloads::Local::capture(
                        sunrise_client::raw_payloads::Side::Server,
                    )
                    .unwrap();
                    let _connection = super::connect_play_test(game, |reader, writer| {
                        super::offer_test_payloads(reader, writer, &local.manifest);
                    });
                    let host: &mut dyn ClientHost = game;
                    assert!(host.has_channel(&id));
                    assert!(!host.has_channel(&other));
                    assert_eq!(
                        host.send_payload_to_server(&id, PayloadBytes(vec![1])),
                        Ok(PayloadAccepted::Queued)
                    );
                });
                // A vanilla server: no offer, so the optional channel is absent.
                with_client_game(|game| {
                    let _connection = super::connect_play_test(game, |_, _| {});
                    let host: &mut dyn ClientHost = game;
                    assert!(!host.has_channel(&id));
                    assert_eq!(
                        host.send_payload_to_server(&id, PayloadBytes(vec![1])),
                        Err(PayloadSendError::UnsupportedPeer(id.clone()))
                    );
                });
            });
        },
    );
}
