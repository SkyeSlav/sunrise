use super::*;
use crate::lan::{Lan, NetConfig};
use mod_api::{
    Flow, ListedPlayerInfo, ModEntry, Mods, ServerHost, ServerListPingEvent, Text,
    testing::{FakeHost, with_dispatch},
    text::{TextContent, TextStyle},
};
use serde_json::{Value, json};
use std::thread::{self, JoinHandle, ThreadId};
use sunrise_protocol::connection::Pipe;

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    protocol: i32,
    host: String,
    motd: String,
    thread: ThreadId,
}

#[derive(Default)]
struct StatusMod(Arc<Mutex<Vec<Seen>>>);

#[mod_api::sunrise_mod]
impl mod_api::Hooks for StatusMod {
    fn server_list_ping(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ServerListPingEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().push(Seen {
            protocol: event.protocol(),
            host: event.virtual_host().into(),
            motd: match &event.motd.content {
                TextContent::Literal(text) => text.clone(),
                _ => String::new(),
            },
            thread: thread::current().id(),
        });
        match event.virtual_host() {
            "lobby.example" => {
                let mut style = TextStyle::default();
                style.color = Some("gold".into());
                *event.motd = Text::literal("Welcome").with_style(style);
                event.online = -2;
                event.max_players = 42;
                *event.sample = (1..=13)
                    .rev()
                    .map(|i| ListedPlayerInfo::new(format!("Player {i}"), i))
                    .collect();
            }
            "HiDdEn.example.\0marker" => event.hide_players = true,
            "silent.example" => return Flow::Cancel,
            "oversize.example" => {
                *event.motd = Text::literal("a".repeat(status::MAX_JSON_UTF16));
            }
            "invalid.example" => {
                event.motd.metadata = sunrise_core::nbt::Tag::Int(1);
            }
            _ => {}
        }
        Flow::Continue
    }
}

fn listener() -> Lan {
    Lan::start_with_spawn_preparation(
        Arc::new(SyncData::empty()),
        NetConfig {
            listen: false,
            ..NetConfig::loopback(0, "Original".into(), 8)
        },
    )
    .unwrap()
}

fn client(
    lan: &Lan,
    mods: Mods,
    host: &str,
    packets: &[status::Serverbound],
    snapshot: Option<Arc<status_response::Snapshot>>,
) -> (Connection, JoinHandle<Result<(), String>>) {
    let (client, server) = Pipe::pair();
    let mut client = Connection::local(client);
    client
        .reader
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let intention = handshake::Intention {
        protocol: 123,
        host: host.into(),
        port: 24444,
        intent: handshake::STATUS,
    };
    let shared = lan.shared.clone();
    // A precaptured snapshot drives the request phase directly, with no timing dependence
    // between changing the backing settings and capturing the handshake's snapshot.
    if snapshot.is_none() {
        client
            .send(&handshake::Serverbound::Intention(intention.clone()).encode())
            .unwrap();
    }
    for packet in packets {
        client.send(&packet.encode()).unwrap();
    }
    let worker = thread::spawn(move || {
        with_dispatch(mods, || {
            let mut server = Connection::local(server);
            if let Some(snapshot) = snapshot {
                serve_status(&mut server, &shared, intention, snapshot)
            } else {
                serve_connection(server, MEMORY_ADDRESS, shared, None)
            }
        })
    });
    (client, worker)
}

fn respond(lan: &Lan, mods: Mods) {
    let request = lan
        .status_requests
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    with_dispatch(mods, || {
        request.respond(|call| mod_api::dispatch_server_list_ping(&mut FakeHost::new(), call));
    });
}

fn read_status(client: &mut Connection) -> Value {
    let (id, body) = client.recv().unwrap();
    let status::Clientbound::StatusResponse(response) =
        status::Clientbound::decode(id, &body).unwrap()
    else {
        panic!("expected a status response");
    };
    serde_json::from_str(&response.json).unwrap()
}

const REQUEST: status::Serverbound = status::Serverbound::StatusRequest(Empty);

#[test]
fn ping_edits_the_captured_status_once_without_changing_the_next_response() {
    let mut lan = listener();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new("status", Box::new(StatusMod(seen.clone())))].into();
    let captured = lan.shared.status_snapshot();
    lan.shared.set_motd("Current".into());
    lan.refresh_status(std::iter::empty());
    let (mut first, worker) = client(
        &lan,
        mods.clone(),
        "lobby.example",
        &[REQUEST, REQUEST],
        Some(captured),
    );
    respond(&lan, mods.clone());
    let edited = read_status(&mut first);
    assert_eq!(
        edited["description"],
        json!({"text":"Welcome","color":"gold"})
    );
    assert_eq!(edited["version"]["protocol"], 777);
    assert_eq!(edited["players"]["online"], -2);
    assert_eq!(edited["players"]["max"], 42);
    let sample = edited["players"]["sample"].as_array().unwrap();
    assert_eq!(sample.len(), 13);
    for (row, i) in sample.iter().zip((1..=13).rev()) {
        assert_eq!(row["name"], format!("Player {i}"));
        assert_eq!(row["id"], format!("00000000-0000-0000-0000-{i:012x}"));
    }
    assert!(
        first.recv().unwrap_err().is_end_of_stream(),
        "duplicate closes"
    );
    worker.join().unwrap().unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [Seen {
            protocol: 123,
            host: "lobby.example".into(),
            motd: "Original".into(),
            thread: thread::current().id(),
        }]
    );

    // No subscriber on the connection worker: it sends the current normal snapshot directly.
    let (mut next, worker) = client(
        &lan,
        Arc::from([]),
        "next.example",
        &[REQUEST, REQUEST],
        None,
    );
    let normal = read_status(&mut next);
    assert_eq!(normal["description"], "Current");
    assert_eq!(normal["players"], json!({"max":8,"online":0}));
    worker.join().unwrap().unwrap();
    assert!(lan.next_status_request().is_none());
}

#[test]
fn hiding_players_omits_the_section_and_preserves_raw_handshake_metadata() {
    let lan = listener();
    let host = "HiDdEn.example.\0marker";
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new("status", Box::new(StatusMod(seen.clone())))].into();
    let (mut client, worker) = client(&lan, mods.clone(), host, &[REQUEST, REQUEST], None);
    respond(&lan, mods);
    assert!(read_status(&mut client).get("players").is_none());
    worker.join().unwrap().unwrap();
    let calls = seen.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].host, host);
    assert_eq!(calls[0].protocol, 123);
}

#[test]
fn disabled_status_and_standalone_latency_ping_do_not_dispatch() {
    let lan = listener();
    let mods: Mods = vec![ModEntry::new("status", Box::new(StatusMod::default()))].into();
    lan.shared.set_enable_status(false);
    let (mut disabled, worker) = client(&lan, mods.clone(), "silent.example", &[REQUEST], None);
    assert!(disabled.recv().unwrap_err().is_end_of_stream());
    worker.join().unwrap().unwrap();
    lan.shared.set_enable_status(true);
    let (mut latency, worker) = client(
        &lan,
        mods,
        "silent.example",
        &[status::Serverbound::PingRequest(status::Ping {
            time: -733,
        })],
        None,
    );
    let (id, body) = latency.recv().unwrap();
    let status::Clientbound::PongResponse(pong) = status::Clientbound::decode(id, &body).unwrap()
    else {
        panic!("expected a pong");
    };
    assert_eq!(pong.time, -733);
    assert!(latency.recv().unwrap_err().is_end_of_stream());
    worker.join().unwrap().unwrap();
    assert!(lan.next_status_request().is_none());
}

#[test]
fn cancellation_and_unencodable_replies_write_no_status_frame() {
    let lan = listener();
    let mods: Mods = vec![ModEntry::new("status", Box::new(StatusMod::default()))].into();
    for (host, expected_error) in [
        ("silent.example", None),
        ("oversize.example", Some("vanilla string limit")),
        ("invalid.example", Some("malformed text fields")),
    ] {
        let (mut client, worker) = client(&lan, mods.clone(), host, &[REQUEST, REQUEST], None);
        respond(&lan, mods.clone());
        assert!(client.recv().unwrap_err().is_end_of_stream(), "{host}");
        let result = worker.join().unwrap();
        if let Some(expected_error) = expected_error {
            assert!(result.unwrap_err().contains(expected_error), "{host}");
        } else {
            result.unwrap();
        }
    }
}
