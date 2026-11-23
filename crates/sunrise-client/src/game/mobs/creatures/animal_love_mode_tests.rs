use super::*;
use crate::game::{
    execution_role::ExecutionRole,
    native_entity_refs::tests::{
        bind_acting_guest, bind_local_player, fixture_root, run_fresh_cases, with_game,
        with_game_role,
    },
};
use mod_api::{
    EntityEnterLoveModeEvent, EntityEnterLoveModePreEvent, EntityRef, Flow, Hand, Hooks, LevelRef,
    ModEntry, PlayerRef, ServerHost,
};
use std::sync::Mutex;
use sunrise_client::{lan::playerdata::uuid_ints, motion::PlayerMotion};
use sunrise_core::nbt::Tag;

const CONN: u32 = 12;
const PLAYER_ID: u32 = 2;
const FEEDER: u128 = 7;
const CHILD: &str = "SUNRISE_ANIMAL_LOVE_MODE_CASE";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Allow,
    Veto,
    Discard,
    Wound,
    ReplaceAndVeto,
}

#[derive(Debug, PartialEq)]
enum Heard {
    Pre {
        animal: u32,
        level: LevelRef,
        animal_level: Option<LevelRef>,
        player: Option<PlayerRef>,
        held: Option<(String, u32)>,
        health: Option<f32>,
        love: Option<i32>,
    },
    Post {
        animal: u32,
        kind: String,
        player: Option<u128>,
        love: Option<i32>,
    },
}

struct Probe {
    hand: Hand,
    answer: Answer,
    heard: Vec<Heard>,
}

struct Listener(Arc<Mutex<Probe>>);

fn love_ticks(host: &mut dyn ServerHost, animal: EntityRef) -> Option<i32> {
    let nbt = animal.nbt(host)?;
    match nbt.get(&"InLove".into())? {
        Tag::Int(ticks) => Some(*ticks),
        _ => None,
    }
}

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn entity_enter_love_mode_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityEnterLoveModePreEvent,
    ) -> Flow {
        let mut probe = self.0.lock().unwrap();
        let held = event.player.and_then(|player| {
            host.living(player.entity())?
                .with_hand_item(host, probe.hand, |stack| {
                    Some((stack.item().id(host)?.to_owned(), stack.count()))
                })?
        });
        probe.heard.push(Heard::Pre {
            animal: event.animal.id(),
            level: event.level,
            animal_level: event.animal.level(host),
            player: event.player,
            held,
            health: host
                .living(event.animal)
                .and_then(|living| living.health(host)),
            love: love_ticks(host, event.animal),
        });
        let (answer, hand) = (probe.answer, probe.hand);
        drop(probe);
        match answer {
            Answer::Allow => Flow::Continue,
            Answer::Veto => Flow::Cancel,
            Answer::Discard => {
                assert!(event.animal.discard(host));
                Flow::Continue
            }
            Answer::Wound => {
                let living = host.living(event.animal).unwrap();
                let health = living.health(host).unwrap();
                assert!(living.set_health(host, health - 1.0));
                Flow::Continue
            }
            Answer::ReplaceAndVeto => {
                let living = host.living(event.player.unwrap().entity()).unwrap();
                let replacement = ItemStack::new("minecraft:stick", 2).try_into_api().unwrap();
                assert_eq!(
                    living.set_hand_item(host, hand, Some(replacement)),
                    Ok(true)
                );
                Flow::Cancel
            }
        }
    }

    fn entity_enter_love_mode(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityEnterLoveModeEvent<'_>,
    ) {
        let love = host
            .entity(event.entity)
            .and_then(|animal| love_ticks(host, animal));
        self.0.lock().unwrap().heard.push(Heard::Post {
            animal: event.entity,
            kind: event.kind.to_owned(),
            player: event.player,
            love,
        });
    }
}

fn setup(game: &mut Game<'_>) {
    game.build_registries();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.set_mode_id(0);
    game.selected = 0;
    game.lan_acting = Some(CONN);
    game.lan_acting_name = Some("Feeder".into());
    game.lan_acting_permission = Some(0);
    bind_local_player(game, PLAYER_ID, FEEDER);
    bind_acting_guest(game);
    game.audio.capture = Some(Vec::new());
}

fn animal(game: &mut Game<'_>, kind: &str, x: f64) -> Mob {
    game.next_entity += 1;
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    Mob::new(game.next_entity, kind, Vec3::new(x, 64.0, 1.5), 0.0)
}

fn click(game: &mut Game<'_>, id: u32, hand: Hand, item: &str, count: u32) {
    game.with_hand(hand, |game| {
        *game.held_mut() = Some(ItemStack::new(item, count));
        assert!(
            game.interact_entity(Some(InteractTarget {
                id: id as i32,
                location: [0.0; 3],
            }))
            .unwrap(),
            "the feed accepts the click"
        );
    });
}

struct Feed {
    kind: &'static str,
    item: &'static str,
    count_at_pre: u32,
    heals: (f32, f32), // before and after the veto point
    eat_sound: Option<&'static str>,
}

const FEEDS: &[Feed] = &[
    Feed {
        kind: "minecraft:cow",
        item: "minecraft:wheat",
        count_at_pre: 1,
        heals: (0.0, 0.0),
        eat_sound: None,
    },
    Feed {
        kind: "minecraft:turtle",
        item: "minecraft:seagrass",
        count_at_pre: 1,
        heals: (0.0, 0.0),
        eat_sound: None,
    },
    Feed {
        kind: "minecraft:panda",
        item: "minecraft:bamboo",
        count_at_pre: 1,
        heals: (0.0, 0.0),
        eat_sound: None,
    },
    Feed {
        kind: "minecraft:horse",
        item: "minecraft:golden_carrot",
        count_at_pre: 2,
        heals: (0.0, 4.0),
        eat_sound: Some("minecraft:entity.horse.eat"),
    },
    Feed {
        kind: "minecraft:llama",
        item: "minecraft:hay_block",
        count_at_pre: 2,
        heals: (0.0, 10.0),
        eat_sound: Some("minecraft:entity.llama.eat"),
    },
    Feed {
        kind: "minecraft:camel",
        item: "minecraft:cactus",
        count_at_pre: 2,
        heals: (2.0, 0.0),
        eat_sound: Some("minecraft:entity.camel.eat"),
    },
    Feed {
        kind: "minecraft:nautilus",
        item: "minecraft:cod_bucket",
        count_at_pre: 1,
        heals: (0.0, 0.0),
        eat_sound: Some("minecraft:entity.nautilus.eat"),
    },
];

fn eligible_feed(game: &mut Game<'_>, probe: &Mutex<Probe>, feed: &Feed, answer: Answer) {
    let Feed {
        kind,
        item,
        count_at_pre,
        heals: (before, after),
        eat_sound,
    } = *feed;
    let bucket = kind == "minecraft:nautilus";
    let (hand, count, remaining) = if bucket {
        (Hand::Off, 1, "minecraft:water_bucket")
    } else {
        (Hand::Main, 2, item)
    };
    *probe.lock().unwrap() = Probe {
        hand,
        answer,
        heard: Vec::new(),
    };
    game.mobs.clear();
    game.lan_entity_events.clear();
    game.audio.capture.as_mut().unwrap().clear();
    let mut mob = animal(game, kind, 1.5);
    mob.extra.tame = true;
    mob.extra.owner = Some([0, 0, 0, 99]); // The owner is not the feeder.
    let initial_health = mob.attrs.max_health - before - after;
    mob.health = initial_health;
    let id = mob.id;
    game.mobs.push(mob);
    // A neighbour must not receive entry if the listener removes the target.
    let neighbour = animal(game, "minecraft:cow", 2.5);
    let neighbour_id = neighbour.id;
    game.mobs.push(neighbour);
    let level = game.level("minecraft:overworld").unwrap();
    let player = game.native_acting_player().unwrap();
    assert_eq!((player.entity().id(), player.uuid()), (PLAYER_ID, FEEDER));

    click(game, id, hand, item, count);
    let remaining_stack = if answer == Answer::ReplaceAndVeto {
        ItemStack::new("minecraft:stick", 2)
    } else {
        ItemStack::new(remaining, 1)
    };
    game.with_hand(hand, |game| {
        assert_eq!(game.held(), Some(&remaining_stack), "{kind} {answer:?}");
    });
    let allowed = answer == Answer::Allow;
    let mut expected = vec![Heard::Pre {
        animal: id,
        level,
        animal_level: Some(level),
        player: Some(player),
        held: Some((remaining.to_owned(), count_at_pre)),
        health: Some(initial_health + before),
        love: Some(0),
    }];
    if allowed {
        expected.push(Heard::Post {
            animal: id,
            kind: kind.to_owned(),
            player: Some(FEEDER),
            love: Some(600),
        });
    }
    assert_eq!(probe.lock().unwrap().heard, expected, "{kind} {answer:?}");
    let resident = game.mobs.iter().find(|mob| mob.id == id && !mob.removed);
    if answer == Answer::Discard {
        assert!(resident.is_none());
    } else {
        let mob = resident.unwrap();
        let expected_health =
            initial_health + before + after - if answer == Answer::Wound { 1.0 } else { 0.0 };
        assert_eq!(mob.health, expected_health, "{kind} {answer:?}");
        assert_eq!(mob.extra.in_love, if allowed { 600 } else { 0 });
        assert_eq!(mob.extra.love_cause, allowed.then(|| uuid_ints(FEEDER)));
        if kind == "minecraft:panda" {
            assert!(
                mob.extra.main_hand_id().is_none(),
                "veto does not make the panda sit and eat"
            );
        }
        if let Some(event) = eat_sound {
            assert!(
                game.audio
                    .capture
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|s| s.event == event),
                "{kind} still eats"
            );
        }
    }
    assert_eq!(
        game.mobs
            .iter()
            .find(|m| m.id == neighbour_id)
            .unwrap()
            .extra
            .in_love,
        0
    );
    assert_eq!(
        game.love_cause.get(&id).copied(),
        allowed.then_some(Some(CONN))
    );
    let hearts: Vec<_> = game
        .lan_entity_events
        .iter()
        .filter(|(_, event)| *event == 18)
        .copied()
        .collect();
    assert_eq!(
        hearts,
        if allowed {
            vec![(id as i32, 18)]
        } else {
            Vec::new()
        }
    );
}

fn excluded_feeds(game: &mut Game<'_>, probe: &Mutex<Probe>) {
    probe.lock().unwrap().heard.clear();
    game.mobs.clear();
    let mut baby = animal(game, "minecraft:cow", 1.5);
    baby.extra.age = -24_000;
    let baby_id = baby.id;
    game.mobs.push(baby);
    let mut turtle = animal(game, "minecraft:turtle", 2.5);
    turtle.extra.flag = true;
    let turtle_id = turtle.id;
    game.mobs.push(turtle);
    click(game, baby_id, Hand::Main, "minecraft:wheat", 2);
    assert_eq!(game.held().unwrap().count, 1);
    assert_eq!(game.mobs[0].extra.age, -21_600);
    click(game, turtle_id, Hand::Main, "minecraft:seagrass", 2);
    assert_eq!(game.held().unwrap().count, 2);
    assert!(game.mobs.iter().all(|mob| mob.extra.in_love == 0));
    assert!(probe.lock().unwrap().heard.is_empty());
}

#[test]
#[ignore = "daily"]
fn love_mode_pre_gates_entry_without_rejecting_the_feed() {
    if run_fresh_cases(
        "game::creatures::animal_love_mode_tests::love_mode_pre_gates_entry_without_rejecting_the_feed",
        CHILD,
        &["integrated", "dedicated"],
    ) {
        return;
    }
    let probe = Arc::new(Mutex::new(Probe {
        hand: Hand::Main,
        answer: Answer::Allow,
        heard: Vec::new(),
    }));
    let mods = vec![ModEntry::new(
        "love-mode-test",
        Box::new(Listener(probe.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        if std::env::var(CHILD).as_deref() == Ok("dedicated") {
            with_game_role(fixture_root(), ExecutionRole::DedicatedServer, |game| {
                game.options.dedicated = Some(Vec::new());
                setup(game);
                eligible_feed(game, &probe, &FEEDS[0], Answer::Allow);
            });
        } else {
            with_game(|game| {
                setup(game);
                for feed in FEEDS {
                    for answer in [Answer::Allow, Answer::Veto] {
                        eligible_feed(game, &probe, feed, answer);
                    }
                }
                for (kind, answer) in [
                    ("minecraft:cow", Answer::Discard),
                    ("minecraft:nautilus", Answer::Wound),
                    ("minecraft:horse", Answer::ReplaceAndVeto),
                ] {
                    let feed = FEEDS.iter().find(|feed| feed.kind == kind).unwrap();
                    eligible_feed(game, &probe, feed, answer);
                }
                excluded_feeds(game, &probe);
            });
        }
    });
}
