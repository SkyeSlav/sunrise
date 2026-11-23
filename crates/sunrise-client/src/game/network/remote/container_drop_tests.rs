use super::super::*;
use super::{connect_play_test, spawn_menu_test};
use crate::game::native_entity_refs::tests::with_client_game;
use crate::game::native_hotbar_slot::hotbar_slot_tests::{logged_in, sent};
use sunrise_client::command::args::GameMode;
use sunrise_protocol::packets::configuration as config;

#[test]
#[ignore = "daily"]
fn creative_drop_guards_charge_only_sent_drops() {
    crate::game::in_own_process(
        "game::remote::tests::container_drop_tests::creative_drop_guards_charge_only_sent_drops",
        || {
            with_client_game(|game| {
                let (mut reader, _writer) = connect_play_test(game, |_, server| {
                    server
                        .send(
                            &config::Clientbound::UpdateEnabledFeatures(
                                config::UpdateEnabledFeatures { features: vec![] },
                            )
                            .encode(),
                        )
                        .unwrap();
                });
                // Acknowledging configuration can already drain the feature update.
                // Wait only if the connection still has its default flags.
                if !game.enabled_feature_ids().is_empty() {
                    game.wait_for_test_remote_event(|event| matches!(event, Event::Features(_)));
                    game.poll_remote_events(true).unwrap();
                }
                for _ in 0..64 {
                    game.drop_spam_throttler.increment();
                }
                spawn_menu_test(game);
                assert!(game.drop_spam_throttler.is_under_threshold());
                game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                    teleport: 1,
                    position: [0.5, 65.0, 0.5],
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                }))
                .unwrap();
                sent(&mut reader);
                game.set_local_mode(GameMode::Creative);
                let stone = || ItemStack::new("minecraft:stone", 1);
                let vanilla = ["minecraft:vanilla".to_owned()];
                let catalog = game.creative_screen.catalog();
                let items = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap();
                assert!(!items.names.is_empty());
                assert!(game.enabled_feature_ids().is_empty());
                for name in &items.names {
                    assert!(catalog.is_item_enabled(name, &vanilla), "{name}");
                    assert!(!catalog.is_item_enabled(name, &[]), "{name}");
                }

                game.screen = Screen::Creative;
                game.inventory[0] = Some(stone());
                game.remote_creative_slot_add(0);
                game.inventory[1] = Some(stone());
                game.send_creative_slots();
                game.inventory[2] = Some(stone());
                game.remote_send_state(0);
                game.remote_creative_throw(stone());
                game.inventory[0] = None;
                game.remote_creative_slot_add(0);
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(packets, [(36, None)], "only an empty slot clear is sent");

                game.remote.as_mut().unwrap().features = vanilla.into();
                game.set_local_mode(GameMode::Survival);
                game.inventory[0] = Some(stone());
                game.remote_creative_slot_add(0);
                game.remote_creative_throw(stone());
                game.set_local_mode(GameMode::Creative);
                game.screen = Screen::Inventory;
                game.remote_creative_throw(stone());
                game.screen = Screen::Creative;
                game.remote_creative_throw(ItemStack::new("minecraft:air", 1));
                game.remote_creative_throw(ItemStack::new("minecraft:air", 0));
                game.remote_creative_throw(ItemStack::new("minecraft:stone", 0));
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::SetCreativeModeSlot(_)))
                );

                // Any forbidden drop that charged the counter would move the burst boundary.
                for _ in 0..63 {
                    game.remote_creative_throw(stone());
                }
                assert!(game.drop_spam_throttler.is_under_threshold());
                game.remote_creative_throw(stone());
                assert!(!game.drop_spam_throttler.is_under_threshold());

                // With no screen the game-mode method may still send: only the click checks the cap.
                game.screen = Screen::Playing;
                game.remote_creative_throw(stone());
                game.remote_creative_slot_add(0);
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item.unwrap().count)),
                        _ => None,
                    })
                    .collect();
                let mut expected = vec![(-1, 1); 65];
                expected.push((36, 1));
                assert_eq!(packets, expected);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn creative_screen_throws_wait_for_loaded_player_tick() {
    crate::game::in_own_process(
        "game::remote::tests::container_drop_tests::creative_screen_throws_wait_for_loaded_player_tick",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.set_local_mode(GameMode::Creative);
                game.screen = Screen::Creative;
                let stone = ItemStack::new("minecraft:stone", 2);
                game.inventory[0] = Some(stone.clone());
                game.carried = Some(stone.clone());
                for _ in 0..64 {
                    game.drop_spam_throttler.increment();
                }
                game.remote.as_mut().unwrap().loaded_sent = false;
                assert_eq!(game.tick_remote_own_player().unwrap(), None);
                assert!(game.click_open_inventory(Some(0), 0, Input::Throw));
                game.throw_click(false, Input::Pickup);
                assert_eq!(game.inventory[0].as_ref(), Some(&stone));
                assert_eq!(game.carried.as_ref(), Some(&stone));
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::SetCreativeModeSlot(_)))
                );

                game.remote.as_mut().unwrap().loaded_sent = true;
                let _ = game.tick_remote_own_player().unwrap();
                game.throw_click(false, Input::Pickup);
                assert!(game.carried.is_none());
                assert!(!game.drop_spam_throttler.is_under_threshold());
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item.unwrap().count)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(packets, [(-1, 2)]);

                // A new screen doesn't reset the budget; a replacement LocalPlayer does.
                game.screen = Screen::Playing;
                game.screen = Screen::Creative;
                assert!(!game.drop_spam_throttler.is_under_threshold());
                let spawn = spawn_menu_test(game);
                assert!(
                    !game.drop_spam_throttler.is_under_threshold(),
                    "an existing player survives Login"
                );
                game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
                    .unwrap();
                assert!(game.drop_spam_throttler.is_under_threshold());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn creative_throw_packet_precedes_container_click_with_predicted_slots() {
    crate::game::in_own_process(
        "game::remote::tests::container_drop_tests::creative_throw_packet_precedes_container_click_with_predicted_slots",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.set_local_mode(GameMode::Creative);
                game.screen = Screen::Playing;
                game.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                for _ in 0..63 {
                    game.drop_spam_throttler.increment();
                }
                assert!(
                    game.remote_menu_ops(&[(Some(0), 0, Input::Throw), (Some(0), 0, Input::Throw)])
                );
                assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);
                let stone = game.item_id("minecraft:stone");
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter(|p| matches!(p, S::SetCreativeModeSlot(_) | S::ContainerClick(_)))
                    .collect();
                let [
                    S::SetCreativeModeSlot(drop),
                    S::ContainerClick(first),
                    S::ContainerClick(blocked),
                ] = packets.as_slice()
                else {
                    panic!(
                        "expected a creative drop before the first of two container clicks: {packets:?}"
                    );
                };
                assert_eq!(drop.slot, -1);
                let dropped = drop.item.as_ref().unwrap();
                assert_eq!((dropped.item, dropped.count), (stone, 1));
                assert_eq!((first.container, first.slot, first.input), (0, 36, 4));
                let [(36, Some(changed))] = first.changed.as_slice() else {
                    panic!(
                        "only the clicked hotbar slot should change: {:?}",
                        first.changed
                    );
                };
                assert_eq!((changed.item, changed.count), (stone, 2));
                assert!(first.carried.is_none());
                assert_eq!((blocked.container, blocked.slot, blocked.input), (0, 36, 4));
                assert!(blocked.changed.is_empty());
                assert!(blocked.carried.is_none());
            });
        },
    );
}
