use super::*;
use crate::game::{
    native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
};
use mod_api::{
    AdvancementProgress, DataPackFilesEvent, Hooks, ModEntry, Mods, PlayerAdvancementProgressEvent,
    ServerHost,
};
use std::sync::Mutex;

const ID: &str = "testmod:progress";
const DATE: &str = "2020-01-02 03:04:05 +0000";

#[derive(Debug)]
struct Observation {
    player: u128,
    criterion: String,
    kind: ProgressType,
    progress: AdvancementProgress,
    rewarded: Option<bool>,
}

struct Observer(Arc<Mutex<Vec<Observation>>>);
#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
        if matches!(
            std::env::var("SUNRISE_PROGRESS_SNAPSHOT_CHILD").as_deref(),
            Ok("command-reload" | "nested-reload")
        ) {
            event.files.push((
                "advancement/testmod:progress".into(),
                br#"{
                "criteria": {
                    "alpha": {"trigger": "minecraft:tick"},
                    "gamma": {"trigger": "minecraft:impossible"}
                },
                "requirements": [["alpha"], ["gamma"]],
                "rewards": {"recipes": ["testmod:reward"], "experience": 7,
                    "loot": ["testmod:reward"], "function": "testmod:reward"}
            }"#
                .to_vec(),
            ));
            event.files.push((
                "recipe/testmod:reward".into(),
                br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:diamond","count":1}}"#.to_vec(),
            ));
            event.files.push((
                "loot_table/testmod:reward".into(),
                br#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}]}]}"#.to_vec(),
            ));
        }
    }

    fn player_advancement_progress(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerAdvancementProgressEvent<'_>,
    ) {
        if event.advancement != ID {
            return;
        }
        // Read through the Host during dispatch: no progress storage borrow is retained.
        let rewarded = host
            .player(event.player)
            .and_then(|player| player.has_discovered_recipe(host, "testmod:reward"));
        self.0.lock().unwrap().push(Observation {
            player: event.player,
            criterion: event.criterion.to_owned(),
            kind: event.progress_type,
            progress: event.progress.clone(),
            rewarded,
        });
    }
}

fn assert_progress(progress: &AdvancementProgress, obtained: [bool; 3], groups: u32, done: bool) {
    assert_eq!(
        progress
            .criteria
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(
        progress
            .criteria
            .iter()
            .map(|c| c.obtained_at.is_some())
            .collect::<Vec<_>>(),
        obtained
    );
    assert_eq!(
        progress.requirements,
        vec![vec!["alpha", "beta"], vec!["gamma"]]
    );
    assert_eq!(progress.completed_requirements, groups);
    assert_eq!(progress.done, done);
}

#[test]
#[ignore = "daily"]
fn progress_events_capture_each_players_full_mutation_state() {
    if run_fresh_cases(
        "game::advancements::progress_tests::progress_events_capture_each_players_full_mutation_state",
        "SUNRISE_PROGRESS_SNAPSHOT_CHILD",
        &[
            "acting",
            "first",
            "reload",
            "command-reload",
            "nested-reload",
        ],
    ) {
        return;
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "observer",
        Box::new(Observer(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods);
    let scratch = std::env::current_dir().unwrap().join("progress-data");
    let dir = scratch.join("data/testmod/advancement");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("progress.json"),
        r#"{
        "criteria": {
            "alpha": {"trigger": "minecraft:tick"},
            "beta": {"trigger": "minecraft:tick"},
            "gamma": {"trigger": "minecraft:tick"}
        },
        "requirements": [["alpha", "beta"], ["gamma"]],
        "rewards": {"recipes": ["testmod:reward"]}
    }"#,
    )
    .unwrap();
    let case = std::env::var("SUNRISE_PROGRESS_SNAPSHOT_CHILD").unwrap();
    with_guests(|game| {
        if matches!(case.as_str(), "command-reload" | "nested-reload") {
            use crate::game::lan_actions::GuestCommand;
            game.world_root = scratch.join("world");
            let pack = game.world_root.join("datapacks/test");
            std::fs::create_dir_all(pack.join("data/testmod/function")).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"test","pack_format":121}}"#,
            )
            .unwrap();
            std::fs::write(
                pack.join("data/testmod/function/reward.mcfunction"),
                "scoreboard players add @s reward_hits 1\n",
            )
            .unwrap();
            std::fs::write(
                game.world_root.join("datapacks.json"),
                r#"{"enabled":["file/test"],"disabled":[]}"#,
            )
            .unwrap();
            game.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
            game.score.board.add_objective(
                "reward_hits",
                "dummy",
                serde_json::json!({"text":"reward_hits"}),
                sunrise_client::scoreboard::RenderType::Integer,
                false,
                None,
            );
            game.lan.as_mut().unwrap().hub.guests.update_all(|guest| {
                guest.advancements_loaded = true;
                guest.advancements.grant(ID, "gamma", DATE);
            });
            game.lan.as_mut().unwrap().hub.mods_loaded = true;
            let reload = |game: &mut Game<'_>| {
                assert!(game.run_guest_command(GuestCommand {
                    conn: 12,
                    command: "reload".into(),
                    permission: 4,
                    integrated: true,
                    quiet: false,
                }));
            };
            if case == "nested-reload" {
                game.as_guest(13, reload).unwrap();
            } else {
                reload(game);
            }
            assert!(
                game.fns.store.functions.contains_key("testmod:reward"),
                "reward function was not loaded"
            );
            let events = observed.lock().unwrap();
            let first: Vec<_> = events.iter().filter(|event| event.player == 12).collect();
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].criterion, "alpha");
            assert_eq!(first[0].kind, ProgressType::Grant);
            assert_eq!(
                first[0].rewarded,
                Some(false),
                "live state is read before rewards"
            );
            let progress = &first[0].progress;
            assert_eq!(
                progress
                    .criteria
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>(),
                ["alpha", "gamma"]
            );
            assert!(progress.criteria[0].obtained_at.is_some());
            assert_eq!(progress.criteria[1].obtained_at.as_deref(), Some(DATE));
            assert_eq!(progress.requirements, vec![vec!["alpha"], vec!["gamma"]]);
            assert_eq!(progress.completed_requirements, 2);
            assert!(progress.done);
            assert_eq!(events.len(), 2, "each guest receives its progress event");
            for event in events.iter() {
                if event.player == 12 {
                    assert_eq!(
                        event.progress.criteria[1].obtained_at.as_deref(),
                        Some(DATE)
                    );
                    assert!(event.progress.done);
                }
            }
            let kept = first[0].progress.clone();
            drop(events);
            for conn in [12, 13] {
                let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                assert_eq!(guest.xp_total, if conn == 12 { 7 } else { 0 });
                {
                    let item = "minecraft:stone";
                    assert_eq!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .filter(|stack| stack.item.as_ref() == item)
                            .map(|stack| stack.count as usize)
                            .sum::<usize>(),
                        usize::from(conn == 12),
                        "{conn}: {item}"
                    );
                }
                if conn == 13 && case == "nested-reload" {
                    continue;
                }
                assert!(
                    guest.advancements.has(ID, "alpha"),
                    "alpha survives leaving the turn"
                );
                assert_eq!(guest.advancements.criteria[ID]["gamma"], DATE);
                assert!(
                    guest.recipe_book.knows("testmod:reward"),
                    "reward belongs to the same live player"
                );
            }
            assert_eq!(
                game.score
                    .board
                    .score("guest12", "reward_hits")
                    .map(|score| score.value),
                Some(1)
            );
            assert_eq!(
                game.score
                    .board
                    .score("guest13", "reward_hits")
                    .map(|score| score.value),
                None
            );
            if case == "nested-reload" {
                game.as_guest(13, |game| game.tick_advancements()).unwrap();
            }
            reload(game);
            assert_eq!(
                observed.lock().unwrap().len(),
                if case == "nested-reload" { 3 } else { 2 },
                "a repeated reload makes no duplicate grant"
            );
            for conn in [12, 13] {
                game.as_guest(conn, |game| game.tick_advancements())
                    .unwrap();
                let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                assert_eq!(
                    guest.xp_total,
                    if conn == 12 || case == "nested-reload" {
                        7
                    } else {
                        0
                    }
                );
                {
                    let item = "minecraft:stone";
                    assert_eq!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .filter(|stack| stack.item.as_ref() == item)
                            .map(|stack| stack.count as usize)
                            .sum::<usize>(),
                        usize::from(conn == 12 || case == "nested-reload"),
                        "{conn}: {item}"
                    );
                }
            }
            assert_eq!(kept.criteria[1].obtained_at.as_deref(), Some(DATE));
            assert!(kept.done);
        } else if case == "acting" {
            game.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
            // Another player has unrelated progress: the acting snapshot must not use it.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.advancements.grant(ID, "gamma", DATE))
                .unwrap();
            game.as_guest(12, |game| {
                game.adv.progress.grant(ID, "alpha", DATE);
                let index = game.adv.tree.by_id[ID];
                assert!(game.grant_advancement(index, Some("beta")));
                assert!(
                    !game.grant_advancement(index, Some("beta")),
                    "no duplicate event"
                );
                assert!(game.grant_advancement(index, Some("gamma")));
                assert!(game.revoke_advancement(index, Some("alpha")));
                assert!(game.revoke_advancement(index, Some("beta")));
                assert!(game.revoke_advancement(index, Some("gamma")));
                assert!(
                    !game.revoke_advancement(index, Some("gamma")),
                    "no no-op event"
                );
            })
            .unwrap();
            let events = observed.lock().unwrap();
            assert_eq!(events.len(), 5);
            assert!(events.iter().all(|event| event.player == 12));
            assert_eq!(
                events
                    .iter()
                    .map(|event| (event.criterion.as_str(), event.kind))
                    .collect::<Vec<_>>(),
                vec![
                    ("beta", ProgressType::Grant),
                    ("gamma", ProgressType::Grant),
                    ("alpha", ProgressType::Revoke),
                    ("beta", ProgressType::Revoke),
                    ("gamma", ProgressType::Revoke),
                ]
            );
            assert_progress(&events[0].progress, [true, true, false], 1, false);
            assert_progress(&events[1].progress, [true, true, true], 2, true);
            assert_progress(&events[2].progress, [false, true, true], 2, true);
            assert_progress(&events[3].progress, [false, false, true], 1, false);
            assert_progress(&events[4].progress, [false, false, false], 0, false);
            assert_eq!(
                events[0].progress.criteria[0].obtained_at.as_deref(),
                Some(DATE)
            );
            assert_eq!(
                events[0].progress.criteria[1].obtained_at,
                events[1].progress.criteria[1].obtained_at
            );
            assert_eq!(
                events[1].rewarded,
                Some(false),
                "completion is captured before rewards"
            );
        } else {
            let mut sync = sunrise_client::lan::data::SyncData::empty();
            sync.advancements = sunrise_client::lan::ServerAdvancements::load(
                &scratch,
                &sunrise_world::loot::PackOverlay::NONE,
                // Rewards must name a recipe registered in the server's index.
                sunrise_client::lan::recipes::BookProjections::single_shapeless("testmod:reward"),
            );
            let reload = case == "reload";
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update_all(|guest| {
                guest.advancements_loaded = reload;
            });
            host.hub
                .update_guest(13, |guest| guest.advancements.grant(ID, "gamma", DATE))
                .unwrap();
            host.hub.mods_loaded = reload;
            game.with_advancement_hub(|hub, owners| hub.reload_resources(Arc::new(sync), owners));
            if reload {
                game.run_native_advancement_grants();
            } else {
                game.world_root = game.options.game_dir.clone().unwrap();
                game.tick_server(false).unwrap();
            }
            let events = observed.lock().unwrap();
            assert_eq!(
                events.len(),
                4,
                "one mutation at a time, including each guest's prior progress"
            );
            assert!(
                events
                    .iter()
                    .all(|event| event.kind == ProgressType::Grant
                        && event.rewarded == Some(false))
            );
            let first: Vec<_> = events.iter().filter(|event| event.player == 12).collect();
            assert_eq!(
                first
                    .iter()
                    .map(|event| event.criterion.as_str())
                    .collect::<Vec<_>>(),
                ["alpha", "beta", "gamma"]
            );
            assert_progress(&first[0].progress, [true, false, false], 1, false);
            assert_progress(&first[1].progress, [true, true, false], 1, false);
            assert_progress(&first[2].progress, [true, true, true], 2, true);
            let second: Vec<_> = events.iter().filter(|event| event.player == 13).collect();
            assert_eq!(second.len(), 1);
            assert_eq!(second[0].criterion, "alpha");
            assert_progress(&second[0].progress, [true, false, true], 2, true);
            assert_eq!(
                second[0].progress.criteria[2].obtained_at.as_deref(),
                Some(DATE)
            );
            for conn in [12, 13] {
                assert!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(conn)
                        .unwrap()
                        .recipe_book
                        .knows("testmod:reward")
                );
            }
        }
    });
    std::fs::remove_dir_all(scratch).unwrap();
}

const IMPORT: &str = r#"{"testmod:progress":{"criteria":{"alpha":"1970-01-01 01:00:01 +0100"}}}"#;

#[derive(Default)]
struct SnapshotImport {
    progress_notices: std::sync::atomic::AtomicUsize,
    earned_notices: std::sync::atomic::AtomicUsize,
    readback: Mutex<Option<String>>,
}

struct SnapshotImporter(Arc<SnapshotImport>);

#[mod_api::__subscriptions]
impl Hooks for SnapshotImporter {
    fn player_advancement_progress(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerAdvancementProgressEvent<'_>,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        let prior = self.0.progress_notices.fetch_add(1, Relaxed);
        if event.player == 12 && event.advancement == ID && prior == 0 {
            let player = host.player(event.player).unwrap();
            assert!(player.set_advancements(host, IMPORT));
            *self.0.readback.lock().unwrap() = player.advancements(host);
        }
    }

    fn player_advancement_done(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::PlayerAdvancementDoneEvent<'_>,
    ) {
        self.0
            .earned_notices
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn advancement_packets(
    rx: &std::sync::mpsc::Receiver<sunrise_client::lan::Out>,
) -> Vec<sunrise_protocol::packets::play::UpdateAdvancements> {
    use sunrise_client::lan::Out;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        packets::play::{UpdateAdvancements, ids},
    };
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(bytes) => vec![bytes],
            Out::Packets(packets) => packets,
            Out::Disconnect(_) => panic!("fixture client disconnected"),
        })
        .filter_map(|body| {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            assert_ne!(id, ids::SYSTEM_CHAT, "snapshot announced an advancement");
            (id == ids::UPDATE_ADVANCEMENTS).then(|| UpdateAdvancements::read(&mut r).unwrap())
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn snapshot_replacement_is_silent_and_resets_one_player() {
    use std::sync::{atomic::Ordering::Relaxed, mpsc};
    use sunrise_client::lan::{
        PlayerSender, ServerAdvancements, data::SyncData, recipes::BookProjections,
    };

    const FLAG: &str = "SUNRISE_ADVANCEMENT_IMPORT_CHILD";
    if run_fresh_cases(
        "game::advancements::progress_tests::snapshot_replacement_is_silent_and_resets_one_player",
        FLAG,
        &["command", "trigger", "first-tick", "reload"],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let observed = Arc::new(SnapshotImport::default());
    let mods: Mods = vec![ModEntry::new(
        "importer",
        Box::new(SnapshotImporter(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods);

    let scratch = std::env::current_dir().unwrap().join("advancement-import");
    let dir = scratch.join("data/testmod/advancement");
    std::fs::create_dir_all(&dir).unwrap();
    let display = serde_json::json!({
        "title": "Progress", "description": "Test progress",
        "icon": {"id":"minecraft:stone"}, "announce_to_chat": true, "show_toast": true
    });
    for (file, definition) in [
        (
            "progress.json",
            serde_json::json!({
                "display": display,
                "criteria": {"alpha": {"trigger": "minecraft:tick"}},
                "rewards": {"experience": 7, "recipes": ["testmod:reward"]}
            }),
        ),
        (
            "branch.json",
            serde_json::json!({
                "display": display,
                "parent": ID,
                "criteria": {"marker": {"trigger": "minecraft:impossible"}}
            }),
        ),
    ] {
        std::fs::write(dir.join(file), definition.to_string()).unwrap();
    }
    let sync = || {
        let mut sync = SyncData::empty();
        sync.advancements = ServerAdvancements::load(
            &scratch,
            &sunrise_world::loot::PackOverlay::NONE,
            BookProjections::single_shapeless("testmod:reward"),
        );
        Arc::new(sync)
    };
    with_guests(|game| {
        assert!(game.is_logical_server());
        game.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
        let (tx12, rx12) = mpsc::channel();
        let (tx13, rx13) = mpsc::channel();
        game.with_advancement_hub(|hub, owners| hub.reload_resources(sync(), owners));
        game.run_native_advancement_grants();
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.mods_loaded = true;
        hub.update_guest(12, |guest| {
            guest.sender = PlayerSender::new(tx12, Arc::default());
            guest.advancements.grant("testmod:branch", "marker", DATE);
            guest.advancements_loaded = case != "first-tick";
        });
        hub.update_guest(13, |guest| {
            guest.sender = PlayerSender::new(tx13, Arc::default());
            guest.advancements.grant(ID, "alpha", DATE);
            // Player 13 has first-tick progress, not a loaded save, so the
            // independent reload does not stage this player's snapshot.
            guest.advancements_sent = true;
        });
        hub.flush_progress(13);
        if case != "first-tick" {
            hub.flush_progress(12);
        }
        advancement_packets(&rx12);
        advancement_packets(&rx13);

        let player = game.player(12).unwrap();
        let other = game.player(13).unwrap();
        let other_snapshot = other.advancements(game).unwrap();
        let expected = serde_json::json!({"DataVersion": 5023,
            ID: {"criteria": {"alpha": "1970-01-01 01:00:01 +0100"}, "done": true}
        });
        match case.as_str() {
            "command" | "trigger" => game
                .as_guest(12, |game| {
                    if case == "command" {
                        assert!(game.grant_advancement(game.adv.tree.by_id[ID], Some("alpha")));
                    } else {
                        game.trigger("minecraft:tick", |_, _| true);
                    }
                    // Advancements and their dirty state are still lent to this turn.
                    game.lan.as_mut().unwrap().hub.flush_progress(12);
                    assert!(advancement_packets(&rx12).is_empty());
                })
                .unwrap(),
            "first-tick" => {
                game.lan.as_mut().unwrap().hub.flush_progress(12);
                game.run_native_advancement_grants();
            }
            "reload" => {
                game.with_advancement_hub(|hub, owners| hub.reload_resources(sync(), owners));
                game.run_native_advancement_grants();
            }
            _ => unreachable!(),
        }
        assert_eq!(observed.progress_notices.load(Relaxed), 1);
        assert_eq!(observed.earned_notices.load(Relaxed), 0);
        assert_eq!(
            serde_json::from_str::<Value>(observed.readback.lock().unwrap().as_deref().unwrap())
                .unwrap(),
            expected,
        );
        assert_eq!(
            serde_json::from_str::<Value>(&player.advancements(game).unwrap()).unwrap(),
            expected
        );
        assert_eq!(
            other.advancements(game).as_deref(),
            Some(other_snapshot.as_str())
        );
        assert!(game.toast_manager.queued.is_empty());
        assert!(game.adv.announced.is_empty());
        assert!(advancement_packets(&rx12).is_empty());
        assert!(advancement_packets(&rx13).is_empty());

        let hub = &mut game.lan.as_mut().unwrap().hub;
        let guest = hub.guest(12).unwrap();
        assert_eq!(guest.xp_total, 0);
        assert!(!guest.recipe_book.knows("testmod:reward"));
        hub.flush_progress(12);
        hub.flush_progress(13);
        let packets = advancement_packets(&rx12);
        let [reset] = packets.as_slice() else {
            panic!("expected one reset: {packets:?}")
        };
        assert!(reset.reset && reset.removed.is_empty());
        assert_eq!(
            reset
                .added
                .iter()
                .map(|a| a.id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([ID, "testmod:branch"])
        );
        assert_eq!(
            reset.progress.iter().find(|(id, _)| id == ID).unwrap().1,
            vec![("alpha".into(), Some(1000))]
        );
        assert!(advancement_packets(&rx13).is_empty());

        game.next_entity = 100;
        let level = game.level(game.dim.id()).unwrap();
        let fake = game.fake_player(level, 1234, "machine").unwrap();
        let entity = player.entity();
        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(entity.epoch().get() + 1),
                entity.id(),
            ),
            player.uuid(),
        );
        let before = player.advancements(game).unwrap();
        for refused in [fake, stale] {
            assert!(refused.advancements(game).is_none());
            assert!(!refused.set_advancements(game, "{}"));
        }
        assert!(!player.set_advancements(game, r#"{"test:missing":{"done":3}}"#));
        use crate::game::execution_role::ExecutionRole;
        game.options.execution_role = ExecutionRole::Client;
        assert!(player.advancements(game).is_none());
        assert!(!player.set_advancements(game, "{}"));
        game.options.execution_role = ExecutionRole::IntegratedServer;
        game.as_guest(12, |game| {
            game.as_guest(13, |game| {
                assert!(player.advancements(game).is_none());
                assert!(!player.set_advancements(game, "{}"));
            })
            .unwrap();
        })
        .unwrap();
        assert_eq!(player.advancements(game).as_deref(), Some(before.as_str()));
        game.lan.as_mut().unwrap().hub.flush_progress(12);
        assert!(advancement_packets(&rx12).is_empty());

        assert!(player.set_advancements(game, "{}"));
        assert_eq!(
            serde_json::from_str::<Value>(&player.advancements(game).unwrap()).unwrap(),
            serde_json::json!({"DataVersion":5023})
        );
        assert!(advancement_packets(&rx12).is_empty());
        game.lan.as_mut().unwrap().hub.flush_progress(12);
        let packets = advancement_packets(&rx12);
        let [reset] = packets.as_slice() else {
            panic!("expected one empty reset: {packets:?}")
        };
        assert!(
            reset.reset
                && reset.added.is_empty()
                && reset.removed.is_empty()
                && reset.progress.is_empty()
        );
        assert!(advancement_packets(&rx13).is_empty());
        assert_eq!(
            other.advancements(game).as_deref(),
            Some(other_snapshot.as_str())
        );

        game.as_guest(12, |game| {
            assert!(game.grant_advancement(game.adv.tree.by_id[ID], Some("alpha")));
        })
        .unwrap();
        assert_eq!(observed.progress_notices.load(Relaxed), 2);
        assert_eq!(observed.earned_notices.load(Relaxed), 1);
        assert_eq!(
            game.lan.as_ref().unwrap().hub.guest(12).unwrap().xp_total,
            7
        );
    });
}
