//! `PlayerRef::show_dialog`, `show_dialog_by_id` and `clear_dialog`: `ServerPlayer.openDialog`
//! and `/dialog clear` for one player, on that player's play connection.

use super::*;
use crate::game::native_entity_refs::tests::run_fresh_cases;
use crate::game::native_guest_turn_tests::with_guests;
use crate::game::native_sound_refs::tests::packets;
use sunrise_client::lan::PlayerSender;
use sunrise_core::nbt::{Compound, Tag};

/// Gives guest `conn` a sender whose packets the test reads.
fn listen(game: &mut Game<'_>, conn: u32) -> std::sync::mpsc::Receiver<sunrise_client::lan::Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(conn, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default())
        })
        .unwrap();
    rx
}

#[test]
#[ignore = "daily"]
fn dialogs_use_the_target_players_play_connection() {
    if run_fresh_cases(
        "game::native_server::player_dialog_tests::dialogs_use_the_target_players_play_connection",
        "SUNRISE_PLAYER_DIALOG_CASE",
        &["server"],
    ) {
        return;
    }
    with_guests(|game| {
        let target = listen(game, 12);
        let other = listen(game, 13);
        let player = game.player(12).unwrap();
        let notice = r#"{type: "minecraft:notice", title: "Triage"}"#;
        let stock = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .registry_id("minecraft:dialog", "minecraft:server_links")
            .expect("vanilla's server links dialog");

        // Inside the player's own turn the packets wait for the turn to end.
        game.as_guest(12, |game| {
            assert!(player.show_dialog(game, notice));
            assert!(player.show_dialog_by_id(game, "server_links"));
            assert!(player.show_dialog_by_id(game, "minecraft:server_links"));
            assert!(player.clear_dialog(game));
            // Only SNBT syntax is checked, not the dialog's fields.
            assert!(player.show_dialog(game, "42"));
            assert!(!player.show_dialog(game, "{type: "));
            assert!(!player.show_dialog_by_id(game, "minecraft:no_such_dialog"));
            assert!(!player.show_dialog_by_id(game, "Not An Id"));
            assert!(packets(&target).is_empty(), "sent inside the turn");
        })
        .unwrap();

        let mut title = Compound::default();
        title.insert("type".into(), Tag::String("minecraft:notice".into()));
        title.insert("title".into(), Tag::String("Triage".into()));
        let shown: Vec<_> = packets(&target)
            .into_iter()
            .filter_map(|packet| match packet {
                C::ShowDialog(show) => Some(Some(show.dialog)),
                C::ClearDialog(_) => Some(None),
                _ => None,
            })
            .collect();
        assert_eq!(
            shown,
            [
                Some(play::DialogHolder::Direct(Tag::Compound(title))),
                Some(play::DialogHolder::Registry(stock)),
                Some(play::DialogHolder::Registry(stock)),
                None,
                Some(play::DialogHolder::Direct(Tag::Int(42))),
            ]
        );
        assert!(
            packets(&other)
                .iter()
                .all(|packet| !matches!(packet, C::ShowDialog(_) | C::ClearDialog(_))),
            "another player hears nothing"
        );

        // Outside a turn, a connected player is sent the packet at once.
        assert!(player.clear_dialog(game));
        assert!(
            packets(&target)
                .iter()
                .any(|packet| matches!(packet, C::ClearDialog(_)))
        );

        // A player who isn't on answers false.
        let absent = mod_api::__private::game::player_ref(player.entity(), 999);
        assert!(!absent.show_dialog(game, notice));
        assert!(!absent.show_dialog_by_id(game, "server_links"));
        assert!(!absent.clear_dialog(game));
    });
}
