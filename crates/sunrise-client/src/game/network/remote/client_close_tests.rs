use super::super::{C, Game, ItemStack, S, Screen, Tag, play};
use crate::game::{
    native_entity_refs::tests::with_client_game,
    native_hotbar_slot::hotbar_slot_tests::{logged_in, sent},
};

fn isolated(name: &str, body: impl FnOnce()) {
    let (_, module) = module_path!().split_once("::").unwrap();
    let test = format!("{module}::{name}");
    crate::game::in_own_process(&test, body);
}

fn deliver(game: &mut Game<'_>, packet: C) {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    game.handle_packet(C::decode(reader.varint().unwrap(), reader.rest()).unwrap())
        .unwrap();
}

fn chest(game: &Game<'_>) -> C {
    C::OpenScreen(play::OpenScreen {
        container: 12,
        menu: game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:generic_9x3")
            .unwrap(),
        title: Tag::String("Test menu".into()),
    })
}

fn close(game: &mut Game<'_>) {
    deliver(
        game,
        C::ContainerClose(play::ContainerClose { container: 37 }),
    );
}

#[test]
#[ignore = "daily"]
fn server_close_restores_inventory_under_chat_without_echo() {
    isolated(
        "server_close_restores_inventory_under_chat_without_echo",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                let open_chest = chest(game);
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
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_runs_sign_and_inventory_removal_without_echo() {
    isolated(
        "server_close_runs_sign_and_inventory_removal_without_echo",
        || {
            with_client_game(|game| {
                use sunrise_client::{containers::BlockEntity, signs::SignData};

                let mut reader = logged_in(game);
                game.world
                    .block_entities
                    .insert((1, 65, 2), BlockEntity::Sign(SignData::default()));
                deliver(
                    game,
                    C::OpenSignEditor(play::OpenSignEditor {
                        pos: [1, 65, 2],
                        front: false,
                    }),
                );
                assert!(game.sign_edit.is_some());
                game.sign_input(None, Some("unsent"));
                game.sign_input(Some(winit::keyboard::KeyCode::Enter), None);
                game.sign_input(None, Some("next line"));
                close(game);
                assert!(game.sign_edit.is_none());
                let packets = sent(&mut reader);
                let mut updates = packets.iter().filter_map(|packet| match packet {
                    S::SignUpdate(update) => Some(update),
                    _ => None,
                });
                let sign = updates.next().unwrap();
                assert_eq!(sign.pos, [1, 65, 2]);
                assert!(!sign.front);
                assert_eq!(sign.lines, ["unsent", "next line", "", ""]);
                assert!(updates.next().is_none());
                assert!(!packets.iter().any(|p| matches!(p, S::ContainerClose(_))));

                game.open_player_inventory();
                assert_eq!(game.screen, Screen::Inventory);
                assert_eq!(game.remote_menu().0, 0);
                game.remote.as_mut().unwrap().craft_result =
                    Some(ItemStack::new("minecraft:stick", 4));
                game.craft_grid[0] = Some(ItemStack::new("minecraft:oak_planks", 2));
                close(game);
                assert_eq!(game.screen, Screen::Playing);
                assert!(game.remote.as_ref().unwrap().craft_result.is_none());
                let ingredient = game.craft_grid[0].as_ref().unwrap();
                assert_eq!(ingredient.count, 2);
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::ContainerClose(_)))
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_notifies_the_outgoing_receiver_and_only_opens_real_screens() {
    isolated(
        "server_close_notifies_the_outgoing_receiver_and_only_opens_real_screens",
        || {
            use mod_api::{
                ClientHost, Hooks, ModEntry, ScreenClosingEvent, ScreenOpening, ScreenOpeningEvent,
            };
            use std::sync::{Arc, Mutex};

            #[derive(Debug, PartialEq)]
            enum Event {
                Open(&'static str),
                Close {
                    id: &'static str,
                    menu: Option<String>,
                    title: Option<mod_api::Text>,
                    slots: usize,
                },
            }
            struct Observe(Arc<Mutex<Vec<Event>>>);
            #[mod_api::__subscriptions]
            impl Hooks for Observe {
                fn screen_opening(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    self.0.lock().unwrap().push(Event::Open(event.new.id));
                    // A spurious gameplay Opening would keep an already-deactivated menu.
                    (event.new.id == "playing").then_some(ScreenOpening::Cancel)
                }
                fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                    self.0.lock().unwrap().push(Event::Close {
                        id: event.screen.id,
                        menu: event.screen.menu.map(str::to_owned),
                        title: event.screen.title.cloned(),
                        slots: event.screen.slots,
                    });
                }
            }
            let events = Arc::new(Mutex::new(Vec::new()));
            let take = || std::mem::take(&mut *events.lock().unwrap());
            let mods = vec![ModEntry::new(
                "screen_probe",
                Box::new(Observe(events.clone())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_client_game(|game| {
                    let mut reader = logged_in(game);
                    let open = chest(game);
                    deliver(game, open.clone());
                    let slots = game.slot_count();
                    assert!(slots > 36);
                    let title = game.test_observed_title().unwrap().as_ref().clone();
                    take();
                    close(game);
                    assert_eq!(game.screen, Screen::Playing);
                    assert_eq!(
                        take(),
                        [Event::Close {
                            id: "inventory",
                            menu: Some("minecraft:generic_9x3".into()),
                            title: Some(title),
                            slots,
                        }]
                    );

                    deliver(game, open);
                    game.open_sign_editor((1, 65, 2), true);
                    assert_eq!(game.screen, Screen::Inventory);
                    take();
                    close(game);
                    assert_eq!(game.screen, Screen::Playing);
                    assert_eq!(
                        take(),
                        [Event::Close {
                            id: "sign_editor",
                            menu: None,
                            title: None,
                            slots: 0
                        }]
                    );

                    game.set_screen(Screen::Paused);
                    game.player
                        .as_mut()
                        .unwrap()
                        .set_health(0.0, false)
                        .unwrap();
                    game.remote.as_mut().unwrap().show_death_screen = true;
                    let paused = Event::Close {
                        id: "paused",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    take();
                    close(game);
                    assert_eq!(game.screen, Screen::Dead);
                    assert_eq!(take(), [Event::Open("dead"), paused]);

                    game.remote.as_mut().unwrap().show_death_screen = false;
                    let dead = Event::Close {
                        id: "dead",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    let _ = sent(&mut reader);
                    close(game);
                    assert_eq!(game.screen, Screen::Playing);
                    assert_eq!(take(), [dead]);
                    assert!(
                        sent(&mut reader)
                            .iter()
                            .any(|p| matches!(p, S::ClientCommand(command) if command.action == 0))
                    );

                    game.set_screen(Screen::Paused);
                    game.remote.as_mut().unwrap().logged_in = false;
                    let paused = Event::Close {
                        id: "paused",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    take();
                    close(game);
                    assert_eq!(game.title.as_ref().unwrap().mode.id(), "title");
                    assert_eq!(take(), [Event::Open("title"), paused]);
                });
            });
        },
    );
}
