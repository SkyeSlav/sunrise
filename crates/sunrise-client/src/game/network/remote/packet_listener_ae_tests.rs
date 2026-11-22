use super::super::{C, Game, ItemStack, S, Screen, Tag, Vec3, play};
use crate::game::{
    native_entity_refs::tests::with_client_game,
    native_hotbar_slot::hotbar_slot_tests::{logged_in, sent},
};
use sunrise_client::particles::ParticleType;

fn with_isolated_client(name: &str, body: impl FnOnce(&mut Game<'_>)) {
    let test = format!("game::remote::tests::packet_listener_ae_tests::{name}");
    crate::game::in_own_process(&test, || with_client_game(body));
}

fn registry_id(game: &Game<'_>, registry: &str, name: &str) -> i32 {
    game.remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

fn roundtrip(packet: C) -> C {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    C::decode(reader.varint().unwrap(), reader.rest()).unwrap()
}

fn deliver(game: &mut Game<'_>, packet: C) {
    game.handle_packet(roundtrip(packet)).unwrap();
}

fn add_entity(game: &Game<'_>, id: i32, uuid: u128, kind: &str, position: [f64; 3]) -> C {
    C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind: registry_id(game, "minecraft:entity_type", kind),
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    })
}

#[test]
#[ignore = "daily"]
fn add_player_requires_info_before_replacing_a_tracked_entity() {
    with_isolated_client(
        "add_player_requires_info_before_replacing_a_tracked_entity",
        |game| {
            let _reader = logged_in(game);
            let id = 42;
            let uuid = 123;
            let player_at = [8.0, 65.0, 8.0];
            let cart_at = [2.0, 64.0, 2.0];
            let player = add_entity(game, id, uuid, "minecraft:player", player_at);
            deliver(game, player.clone());
            assert_eq!(game.remote.as_ref().unwrap().tracked(id), None);
            assert!(
                !game
                    .remote
                    .as_ref()
                    .unwrap()
                    .level_players
                    .contains_key(&id)
            );

            deliver(
                game,
                add_entity(game, id, 124, "minecraft:minecart", cart_at),
            );
            let cart = game
                .carts
                .iter()
                .find(|cart| cart.id == id as u32)
                .unwrap()
                .join;
            deliver(game, player.clone());
            assert_eq!(
                game.remote.as_ref().unwrap().tracked(id),
                Some(("minecraft:minecart", cart_at))
            );
            assert_eq!(
                game.carts
                    .iter()
                    .find(|cart| cart.id == id as u32)
                    .unwrap()
                    .join,
                cart
            );

            deliver(
                game,
                C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1 | 8,
                    entries: vec![play::PlayerInfo {
                        uuid,
                        name: Some("Unlisted".into()),
                        listed: Some(false),
                        ..Default::default()
                    }],
                }),
            );
            let remote = game.remote.as_ref().unwrap();
            assert!(!remote.listed.contains(&uuid));
            assert_eq!(remote.tracked(id), Some(("minecraft:minecart", cart_at)));
            assert!(!remote.level_players.contains_key(&id));

            deliver(game, player.clone());
            let remote = game.remote.as_ref().unwrap();
            assert_eq!(remote.tracked(id), Some(("minecraft:player", player_at)));
            assert_eq!(remote.level_player(id), Some((uuid, Some("Unlisted"))));
            assert!(game.carts.iter().all(|cart| cart.id != id as u32));

            deliver(
                game,
                C::RemoveEntities(play::RemoveEntities { ids: vec![id] }),
            );
            deliver(
                game,
                C::PlayerInfoRemove(play::PlayerInfoRemove { uuids: vec![uuid] }),
            );
            deliver(game, player);
            assert_eq!(game.remote.as_ref().unwrap().tracked(id), None);
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_restores_inventory_under_chat_without_echo() {
    with_isolated_client(
        "server_close_restores_inventory_under_chat_without_echo",
        |game| {
            let mut reader = logged_in(game);
            let chest = registry_id(game, "minecraft:menu", "minecraft:generic_9x3");
            let open_chest = C::OpenScreen(play::OpenScreen {
                container: 12,
                menu: chest,
                title: Tag::String("Test menu".into()),
            });
            let inventory_carry = ItemStack::new("minecraft:stone", 2);
            for container in [12, 37] {
                game.carried = Some(inventory_carry.clone());
                deliver(game, open_chest.clone());
                assert_eq!(game.screen, Screen::Inventory);
                game.carried = Some(ItemStack::new("minecraft:dirt", 3));
                game.begin_chat_screen("draft from a container", false);
                assert_eq!(game.screen, Screen::Chat);
                assert_eq!(game.remote_menu().0, 12);
                game.inv_drag = Some((0, vec![1]));
                deliver(game, C::ContainerClose(play::ContainerClose { container }));
                assert_eq!(game.remote_menu().0, 0, "packet id {container}");
                assert!(
                    game.carried
                        .as_ref()
                        .is_some_and(|stack| stack.item == inventory_carry.item
                            && stack.count == inventory_carry.count)
                );
                assert_eq!(game.screen, Screen::Playing);
                assert!(game.ui.chat_screen.is_none());
                assert!(game.inv_drag.is_none());
                game.open_chat_method("");
                assert_eq!(
                    game.ui.chat_screen.as_ref().unwrap().value(),
                    "draft from a container"
                );
                game.close_chat(crate::game::chat::ChatExit::Intentional);
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|packet| matches!(packet, S::ContainerClose(_)))
                );
            }

            game.set_screen(Screen::Paused);
            deliver(
                game,
                C::ContainerClose(play::ContainerClose { container: 0 }),
            );
            assert_eq!(game.screen, Screen::Playing);
            assert!(
                !sent(&mut reader)
                    .iter()
                    .any(|p| matches!(p, S::ContainerClose(_)))
            );

            deliver(game, open_chest);
            game.close_container();
            let closes: Vec<_> = sent(&mut reader)
                .into_iter()
                .filter_map(|packet| match packet {
                    S::ContainerClose(close) => Some(close.container),
                    _ => None,
                })
                .collect();
            assert_eq!(closes, [12]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn hit_emitters_follow_vehicles_and_finish_at_the_removed_body() {
    with_isolated_client(
        "hit_emitters_follow_vehicles_and_finish_at_the_removed_body",
        |game| {
            let _reader = logged_in(game);
            let boat = 51;
            let cart = 52;
            game.particles.camera = [16.0, 65.0, 0.0];
            game.particles.particle_level = 0;
            game.particles.seed_randoms(719, 11, 12);
            game.particles.seed_particle_randoms(719);
            let animate = |game: &mut Game<'_>, id, action| {
                deliver(game, C::Animate(play::Animate { id, action }))
            };
            game.particles.clear();
            animate(game, boat, 1);
            assert_eq!(game.particles.pending_len(), 0);

            deliver(
                game,
                add_entity(game, boat, 151, "minecraft:oak_boat", [4.0, 65.0, 0.0]),
            );
            deliver(
                game,
                add_entity(game, cart, 152, "minecraft:minecart", [24.0, 65.0, 0.0]),
            );
            animate(game, boat, 1);
            animate(game, cart, 2);
            assert!(
                game.particles.pending_len() > 0,
                "Animate must emit on vehicles immediately"
            );
            let assert_emission_near = |game: &Game<'_>, ty, x: f64, width: f64| {
                assert!(
                    game.particles
                        .live()
                        .any(|p| p.ty == ty && (p.previous[0] - x).abs() <= width / 4.0),
                    "missing {ty:?} burst at x={x}"
                );
            };
            game.tick_particle_engine();
            assert_emission_near(game, ParticleType::Crit, 4.0, 1.375);
            assert_emission_near(game, ParticleType::EnchantedHit, 24.0, f64::from(0.98_f32));

            // Start fresh emitters before moving the live copies, as the entity tick does.
            game.particles.clear();
            animate(game, boat, 1);
            animate(game, cart, 2);
            game.boats.update(0, |body| body.position.x = 8.0);
            game.carts.update(0, |body| body.position.x = 28.0);
            game.tick_particle_engine();
            assert_emission_near(game, ParticleType::Crit, 8.0, 1.375);
            assert_emission_near(game, ParticleType::EnchantedHit, 28.0, f64::from(0.98_f32));

            // The body may move after the emitter's last tick, then be removed.
            game.boats.update(0, |body| body.position.x = 12.0);
            deliver(
                game,
                C::RemoveEntities(play::RemoveEntities { ids: vec![boat] }),
            );
            deliver(
                game,
                add_entity(game, boat, 153, "minecraft:oak_boat", [16.0, 65.0, 0.0]),
            );
            game.tick_particle_engine();
            assert_emission_near(game, ParticleType::Crit, 12.0, 1.375);
            assert!(
                !game
                    .particles
                    .live()
                    .any(|p| p.ty == ParticleType::Crit && p.previous[0] >= 15.0)
            );
            assert_eq!(game.particles.pending_len(), 0);
            let made = game.particles.made().clone();
            game.tick_particle_engine();
            assert_eq!(
                game.particles.made(),
                &made,
                "emitters expire after three bursts"
            );

            let stand = 53;
            let crystal = 54;
            for (id, kind, x) in [
                (stand, "minecraft:armor_stand", 5.0),
                (crystal, "minecraft:end_crystal", 21.0),
            ] {
                deliver(game, add_entity(game, id, id as u128, kind, [x, 65.0, 0.0]));
            }
            game.particles.clear();
            animate(game, stand, 1);
            animate(game, crystal, 2);
            game.tick_particle_engine();
            assert_emission_near(game, ParticleType::Crit, 5.0, 0.5);
            assert_emission_near(game, ParticleType::EnchantedHit, 21.0, 2.0);
        },
    );
}

#[test]
#[ignore = "daily"]
fn explosion_validates_impulse_and_result_without_suppressing_effects() {
    with_isolated_client(
        "explosion_validates_impulse_and_result_without_suppressing_effects",
        |game| {
            let _reader = logged_in(game);
            game.streaming
                .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
            game.particles.particle_level = 0;
            game.particles.seed_randoms(719, 11, 12);
            game.particles.seed_particle_randoms(719);
            let center = [8.5, 65.0, 8.5];
            game.particles.camera = center;
            let particle = |key| sunrise_protocol::entity_data::Particle {
                id: registry_id(game, "minecraft:particle_type", key),
                data: None,
            };
            let explosion = play::Explode {
                center,
                radius: 2.0,
                block_count: 4,
                knockback: None,
                particle: particle("minecraft:explosion"),
                block_particles: vec![play::ExplosionParticle {
                    particle: particle("minecraft:smoke"),
                    scaling: 1.0,
                    speed: 0.25,
                    weight: 1,
                }],
                sound: (0, Some(("minecraft:entity.generic.explode".into(), None))),
                play_sound: true,
            };
            let start = [2.0_f64, -0.0, 1.5];
            let large = [f64::MAX, 1.0, -0.0];
            for (initial, impulse, expected, sync) in [
                (start, Some([f64::NAN, 1.0, -2.0]), start, false),
                (start, None, start, false),
                (start, Some([1.0, f64::INFINITY, -2.0]), start, false),
                (start, Some([1.0, 2.0, f64::NEG_INFINITY]), start, false),
                (start, Some([1.0, 0.0, -2.0]), [3.0, 0.0, -0.5], true),
                (large, Some([f64::MAX, -1.0, 1.0]), large, true),
            ] {
                game.particles.clear();
                game.audio.capture = Some(Vec::new());
                let player = game.player.as_mut().unwrap();
                let [x, y, z] = initial;
                player.velocity = Vec3::new(x, y, z);
                player.needs_sync = false;
                let packet = roundtrip(C::Explode(play::Explode {
                    knockback: impulse,
                    ..explosion.clone()
                }));
                let C::Explode(decoded) = &packet else {
                    unreachable!()
                };
                assert_eq!(
                    decoded.knockback.map(|v| v.map(f64::to_bits)),
                    impulse.map(|v| v.map(f64::to_bits))
                );
                game.handle_packet(packet).unwrap();
                let player = game.player.as_ref().unwrap();
                let Vec3 { x, y, z } = player.velocity;
                assert_eq!(
                    [x, y, z].map(f64::to_bits),
                    expected.map(f64::to_bits),
                    "impulse {impulse:?}"
                );
                assert_eq!(player.needs_sync, sync, "impulse {impulse:?}");
                let sound = &game.audio.capture.as_ref().unwrap()[..];
                assert!(
                    matches!(sound, [sound] if sound.event == "minecraft:entity.generic.explode" && sound.position == center)
                );
                assert!(game.particles.pending_len() > 0);
                game.tick_particle_engine();
                assert!(
                    game.particles
                        .live()
                        .any(|p| p.ty == ParticleType::Explosion)
                );
                assert!(game.particles.live().any(|p| p.ty == ParticleType::Smoke));
            }
        },
    );
}
