//! `PlayerRef::push_resource_pack` and `pop_resource_pack` on one player's play connection, and
//! `resource_pack_status` hearing each answer's pack id.
use crate::game::Game;
use crate::game::native_entity_refs::tests::run_fresh_cases;
use crate::game::native_guest_turn_tests::with_guests;
use crate::game::native_sound_refs::tests::packets;
use mod_api::{HookId, Hooks, ModEntry, ResourcePackRefusal, ResourcePackStatusEvent, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_client::lan::PlayerSender;
use sunrise_protocol::packets::{
    configuration::{ResourcePackPop, ResourcePackResponse},
    play::{Clientbound as C, Serverbound as S},
};

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

/// The pack packets among `out`.
fn pack_packets(out: Vec<C>) -> Vec<C> {
    out.into_iter()
        .filter(|packet| matches!(packet, C::ResourcePackPush(_) | C::ResourcePackPop(_)))
        .collect()
}

/// Records every answer it hears: player, pack id, status.
struct Answers(Arc<Mutex<Vec<(u128, u128, i32)>>>);
#[mod_api::__subscriptions]
impl Hooks for Answers {
    fn resource_pack_status(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ResourcePackStatusEvent<'_>,
    ) {
        self.0
            .lock()
            .unwrap()
            .push((event.player, event.pack_id, event.status));
    }
}

#[test]
#[ignore = "daily"]
fn resource_packs_reach_one_player_and_answers_name_their_pack() {
    if run_fresh_cases(
        "game::native_player_resource_packs_tests::resource_packs_reach_one_player_and_answers_name_their_pack",
        "SUNRISE_PLAYER_RESOURCE_PACK_CASE",
        &["server"],
    ) {
        return;
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new("packs", Box::new(Answers(heard.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        assert!(mod_api::__private::listened(HookId::ResourcePackStatus));
        with_guests(|game| {
            let target = listen(game, 12);
            let other = listen(game, 13);
            let player = game.player(12).unwrap();
            let prompt = mod_api::Text::literal("Get the pack");
            let url = "https://example.invalid/p.zip";
            assert_eq!(
                player.push_resource_pack(game, 0x1234, url, "", true, Some(prompt.clone())),
                Ok(())
            );
            assert_eq!(player.pop_resource_pack(game, Some(0x1234)), Ok(()));
            assert_eq!(player.pop_resource_pack(game, None), Ok(()));
            // Past the packet's bounds, by UTF-16 units: refused, nothing sent.
            let long_url = "é".repeat(32768);
            assert_eq!(
                player.push_resource_pack(game, 1, &long_url, "", false, None),
                Err(ResourcePackRefusal::UrlTooLong)
            );
            assert_eq!(
                player.push_resource_pack(game, 1, url, &"a".repeat(41), false, None),
                Err(ResourcePackRefusal::HashTooLong)
            );
            assert_eq!(
                player.push_resource_pack(
                    game,
                    1,
                    &"é".repeat(32767),
                    &"a".repeat(40),
                    false,
                    None
                ),
                Ok(())
            );
            let sent = pack_packets(packets(&target));
            let [
                C::ResourcePackPush(push),
                C::ResourcePackPop(first),
                C::ResourcePackPop(all),
                C::ResourcePackPush(bounded),
            ] = sent.as_slice()
            else {
                panic!("{sent:?}");
            };
            assert_eq!(
                (
                    push.id,
                    push.url.as_str(),
                    push.hash.as_str(),
                    push.required
                ),
                (0x1234, url, "", true)
            );
            assert!(push.prompt.is_some());
            assert_eq!(*first, ResourcePackPop { id: Some(0x1234) });
            assert_eq!(*all, ResourcePackPop { id: None });
            assert_eq!(bounded.hash, "a".repeat(40));
            assert!(
                pack_packets(packets(&other)).is_empty(),
                "another player is sent nothing"
            );

            // A player who isn't on is stale.
            let absent = mod_api::__private::game::player_ref(player.entity(), 999);
            assert_eq!(
                absent.pop_resource_pack(game, None),
                Err(ResourcePackRefusal::Stale)
            );

            // Each answer names its pack, also while the player is dead.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_connection(12, |guest| guest.dead = true)
                .unwrap();
            for (id, action) in [(0x101, 3), (0x202, 7)] {
                game.lan_guest_packet(12, S::ResourcePack(ResourcePackResponse { id, action }));
            }
            let uuid = player.uuid();
            assert_eq!(*heard.lock().unwrap(), [(uuid, 0x101, 3), (uuid, 0x202, 7)]);
        });
    });
}
