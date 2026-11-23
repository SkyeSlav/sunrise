//! The player limit a mod sets (`set_max_players`) is the one the server list, the Login
//! packet and the login check all read, not the fixed `max-players` the server started with.
use super::pending_move_tests::with_input_context;
use super::*;

const ADDRESS: &str = "127.0.0.1:25565";

/// The `max_players` the next Login packet to guest 0 carries.
fn login_limit(hub: &mut Hub, rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> i32 {
    while rx.try_recv().is_ok() {}
    hub.guests.update(0, |g| g.placed = false);
    with_input_context(|ctx| hub.place_new_player(1, ctx));
    let mut limit = None;
    while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
        let mut reader = sunrise_protocol::buf::Reader::new(&body);
        let Ok(id) = reader.varint() else { continue };
        if let Ok(C::Login(login)) = C::decode(id, reader.rest()) {
            limit = Some(login.max_players);
        }
    }
    limit.expect("a Login packet")
}

fn status(hub: &Hub) -> serde_json::Value {
    serde_json::from_str(&hub.lan.shared.status_snapshot().json().unwrap()).unwrap()
}

/// The hub starts with 2 places on the network and 20 in its own config, so a Login packet
/// that read the config would say 20.
#[test]
fn the_login_packet_and_the_status_answer_state_the_live_limit() {
    let (mut hub, rx) = hub_with_a_guest();
    assert_eq!(hub.config.max_players, 20);
    assert_eq!(login_limit(&mut hub, &rx), 2);
    hub.end_tick();
    assert_eq!(status(&hub)["players"]["max"], 2);
    hub.lan.shared.set_max_players(7);
    assert_eq!(login_limit(&mut hub, &rx), 7);
    hub.end_tick();
    assert_eq!(status(&hub)["players"]["max"], 7);
    hub.lan.shared.set_max_players(0);
    assert_eq!(login_limit(&mut hub, &rx), 0);
    hub.end_tick();
    assert_eq!(status(&hub)["players"]["max"], 0);
    hub.lan.shared.set_max_players(usize::MAX);
    assert_eq!(login_limit(&mut hub, &rx), i32::MAX);
}

/// A limit lowered below the players on refuses the next login as full and leaves both players
/// on; a raised limit takes logins again.
#[test]
fn a_lowered_limit_refuses_logins_and_keeps_the_players_on() {
    let (mut hub, _rx) = hub_with_a_guest();
    let address = ADDRESS.parse().unwrap();
    let shared = hub.lan.shared.clone();
    assert!(shared.try_admit(address, 11, "First").is_none());
    assert!(shared.try_admit(address, 12, "Second").is_none());
    assert!(
        shared.try_admit(address, 13, "Third").is_some(),
        "2 places, 2 on"
    );
    shared.set_max_players(1);
    assert_eq!(shared.online(), 2, "both stay on");
    hub.end_tick();
    let players = &status(&hub)["players"];
    assert_eq!(players["max"], 1);
    assert_eq!(players["online"], 1, "only the placed guest is listed");
    assert_eq!(players["sample"][0]["name"], "Bob");
    assert!(shared.try_admit(address, 13, "Third").is_some(), "full");
    shared.set_max_players(3);
    assert!(shared.try_admit(address, 13, "Third").is_none());
    assert_eq!(shared.online(), 3);
}

#[test]
fn status_anonymizes_opted_out_players_and_global_hiding_keeps_counts() {
    let (mut hub, _rx) = hub_with_a_guest();
    hub.guests
        .update(0, |guest| guest.info.allows_listing = false);
    hub.end_tick();
    assert_eq!(
        status(&hub)["players"],
        serde_json::json!({
            "max": 2,
            "online": 1,
            "sample": [{"name":"Anonymous Player","id":"00000000-0000-0000-0000-000000000000"}]
        }),
    );
    hub.lan.shared.set_hide_online_players(true);
    hub.end_tick();
    assert!(
        status(&hub)["players"].get("sample").is_some(),
        "still cached"
    );
    hub.lan.last_status = Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
    hub.end_tick();
    assert_eq!(
        status(&hub)["players"],
        serde_json::json!({"max":2,"online":1})
    );
}

#[test]
fn lowering_the_limit_to_zero_only_admits_players_with_a_bypass() {
    use crate::lan::NetConfig;
    use std::sync::Arc;
    use sunrise_protocol::text::translatable;

    const OP: u128 = 11;
    let config = NetConfig {
        announce: false,
        listen: false,
        bypasses_limit: Some(Arc::new(|uuid| uuid == OP)),
        ..NetConfig::loopback(0, "LiveLimitTest".into(), 2)
    };
    let lan = Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
    let address = ADDRESS.parse().unwrap();
    lan.shared.set_max_players(0);
    assert_eq!(
        lan.shared.try_admit(address, 12, "Player"),
        Some(translatable("multiplayer.disconnect.server_full", &[]))
    );
    assert!(lan.shared.try_admit(address, OP, "Operator").is_none());
    assert_eq!(lan.shared.online(), 1);
}
