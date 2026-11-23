//! `PlayerRef::send_play_packets`: a mod's vanilla play packets reach one player's
//! connection as one ordered run, after the acting hook and only on commit.

use super::*;
use crate::game::native_entity_refs::tests::run_fresh_cases;
use crate::game::native_guest_turn_tests::with_guests;
use mod_api::{ClientboundPlayPacket, SendClientboundPacketsError as Refusal};
use sunrise_client::lan::{Out, PlayerSender};

/// Gives guest `conn` a sender whose bodies the test reads.
fn listen(game: &mut Game<'_>, conn: u32) -> std::sync::mpsc::Receiver<Out> {
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

/// Every packet body (VarInt id and payload) sent since the last look, in order.
fn bodies(rx: &std::sync::mpsc::Receiver<Out>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Ok(item) = rx.try_recv() {
        match item {
            Out::Packet(body) => out.push(body),
            Out::Packets(bodies) => out.extend(bodies),
            Out::Disconnect(_) => {}
        }
    }
    out
}

fn packet(name: &str, body: &[u8]) -> ClientboundPlayPacket {
    ClientboundPlayPacket::new(
        Identifier::parse(&format!("minecraft:{name}")).unwrap(),
        body.to_vec(),
    )
}

#[test]
#[ignore = "daily"]
fn sends_one_ordered_batch_to_the_intended_player_after_the_acting_hook() {
    if run_fresh_cases(
        "game::native_server::clientbound_play_tests::sends_one_ordered_batch_to_the_intended_player_after_the_acting_hook",
        "SUNRISE_CLIENTBOUND_PLAY_CASE",
        &["server"],
    ) {
        return;
    }
    with_guests(|game| {
        let acting = listen(game, 12);
        let other = listen(game, 13);
        let player = game.player(12).unwrap();
        let other_player = game.player(13).unwrap();
        // set_camera (0x5f) with entity 300, set_chunk_cache_radius (0x61) with 8.
        let run = vec![
            packet("set_camera", &[0xac, 0x02]),
            packet("set_chunk_cache_radius", &[8]),
        ];
        let bundled = vec![vec![0], vec![0x5f, 0xac, 0x02], vec![0x61, 0x08], vec![0]];

        // The acting player's run waits for the hook; an aborted transaction's never goes.
        game.as_guest(12, |game| {
            assert_eq!(player.send_play_packets(game, run.clone(), true), Ok(()));
            let aborted: Result<(), ()> = game.storage_transaction(|game, _| {
                assert_eq!(player.send_play_packets(game, run.clone(), false), Ok(()));
                Err(())
            });
            assert!(aborted.is_err());
            assert!(bodies(&acting).is_empty(), "sent inside the hook");
        })
        .unwrap();
        assert_eq!(bodies(&acting), bundled);

        // Outside a turn another player is sent theirs at once, unbundled when not asked.
        bodies(&other);
        assert_eq!(
            other_player.send_play_packets(game, run.clone(), false),
            Ok(())
        );
        assert_eq!(bodies(&other), [bundled[1].clone(), bundled[2].clone()]);
        assert!(bodies(&acting).is_empty());

        // A refusal sends nothing.
        let truncated = vec![run[0].clone(), packet("set_chunk_cache_radius", &[])];
        assert_eq!(
            other_player.send_play_packets(game, truncated, true),
            Err(Refusal::InvalidBody { index: 1 })
        );
        let absent = mod_api::__private::game::player_ref(player.entity(), 999);
        assert_eq!(
            absent.send_play_packets(game, run, true),
            Err(Refusal::Stale)
        );
        assert!(bodies(&other).is_empty());
        assert!(bodies(&acting).is_empty());
    });
}
