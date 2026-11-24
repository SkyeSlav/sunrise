//! `entity_tick_pre` and `entity_tick_post` for the server's item and glow item frames, paintings,
//! armor stands and thrown tridents: each is told alone around its own tick, and a Pre that
//! cancels skips the tick and the Post.
use super::{
    Game,
    native_entity_refs::tests::{prepare_entity_ticking_neighborhood, with_game},
};
use mod_api::{Hooks, HostSide};
use std::sync::{Arc, Mutex};
use sunrise_client::entities::ArmorStand;
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_gameplay::trident::ThrownTrident;

const AT: Vec3 = Vec3::new(4.5, 64.0, 4.5);
const STAND: u32 = 71;
const TRIDENT: u32 = 72;

/// What the listener heard, in order: the phase, the entity and its type.
type Heard = Arc<Mutex<Vec<(&'static str, u32, String)>>>;
/// The entities the listener cancels in Pre.
type Cancel = Arc<Mutex<Vec<u32>>>;

struct Listen {
    heard: Heard,
    cancel: Cancel,
}

#[mod_api::__subscriptions]
impl Hooks for Listen {
    fn entity_tick_listened(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickListenedEvent) {
        event.listened = true;
    }

    fn entity_tick_pre(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickPreEvent<'_>) {
        let cancel = self.cancel.lock().unwrap();
        let mut heard = self.heard.lock().unwrap();
        for (&id, kind) in event.entities.iter().zip(event.kinds.iter()) {
            heard.push(("pre", id, (*kind).to_owned()));
            if cancel.contains(&id) {
                event.canceled.push(id);
            }
        }
    }

    fn entity_tick_post(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        let mut heard = self.heard.lock().unwrap();
        for (&id, kind) in event.entities.iter().zip(event.kinds.iter()) {
            heard.push(("post", id, (*kind).to_owned()));
        }
    }
}

/// A server game with the listener, one of each family in an entity-ticking chunk (the frames,
/// the painting, the stand and the trident), and the listener's levers.
fn with_families(test: impl FnOnce(&mut Game<'_>, &Heard, &Cancel)) {
    let heard = Heard::default();
    let cancel = Cancel::default();
    let listen = Listen {
        heard: heard.clone(),
        cancel: cancel.clone(),
    };
    let entry = mod_api::ModEntry::new("tick-families", Box::new(listen));
    mod_platform::testing::with_mods(Arc::from([entry]), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.build_registries();
            assert!(game.as_console(|g| g.run_command("forceload add 0 0")));
            game.sync_tick_chunks();
            assert!(game.entity_ticking_area()(4, 4));
            game.next_entity = 100;
            assert!(game.as_console(|g| {
                g.run_command("summon minecraft:item_frame 8 100 8 {Facing:3b}")
                    && g.run_command("summon minecraft:glow_item_frame 10 100 8 {Facing:3b}")
                    && g.run_command("summon minecraft:painting 12 100 8 {facing:0b}")
            }));
            game.armor_stands.push(ArmorStand::new(STAND, AT, 0.0));
            game.tridents
                .push(ThrownTrident::remote(TRIDENT, AT, 0.0, 0.0));
            test(game, &heard, &cancel);
        });
    });
}

fn heard(heard: &Heard) -> Vec<(&'static str, u32, String)> {
    std::mem::take(&mut *heard.lock().unwrap())
}

fn told(id: u32, kind: &str) -> [(&'static str, u32, String); 2] {
    [("pre", id, kind.to_owned()), ("post", id, kind.to_owned())]
}

#[test]
#[ignore = "daily"]
fn item_and_glow_item_frames_are_told_their_tick() {
    with_families(|game, listened, _| {
        assert_eq!(game.item_frames.len(), 2);
        let ids = game
            .item_frames
            .iter()
            .map(|f| (f.id, f.kind()))
            .collect::<Vec<_>>();
        heard(listened);
        game.tick_item_frames();
        let expected = ids
            .iter()
            .flat_map(|&(id, kind)| told(id, kind))
            .collect::<Vec<_>>();
        assert_eq!(heard(listened), expected);
        assert!(game.item_frames.iter().all(|f| f.ticks == 1));
    });
}

#[test]
#[ignore = "daily"]
fn a_painting_is_told_its_tick() {
    with_families(|game, listened, _| {
        let id = game.paintings[0].id;
        heard(listened);
        game.tick_paintings();
        assert_eq!(heard(listened), told(id, "minecraft:painting"));
        assert_eq!(game.paintings[0].ticks, 1);
    });
}

#[test]
#[ignore = "daily"]
fn an_armor_stand_is_told_its_tick() {
    with_families(|game, listened, _| {
        heard(listened);
        game.tick_armor_stands().unwrap();
        assert_eq!(heard(listened), told(STAND, "minecraft:armor_stand"));
        assert_eq!(game.armor_stands[0].tick_count, 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_thrown_trident_is_told_its_tick() {
    with_families(|game, listened, _| {
        let age = game.tridents[0].age;
        heard(listened);
        game.tick_tridents();
        assert_eq!(heard(listened), told(TRIDENT, "minecraft:trident"));
        assert_eq!(game.tridents[0].age, age + 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_pre_skips_the_tick_and_the_post_of_each_family() {
    with_families(|game, listened, cancel| {
        let frames = game.item_frames.iter().map(|f| f.id).collect::<Vec<_>>();
        let painting = game.paintings[0].id;
        let age = game.tridents[0].age;
        cancel
            .lock()
            .unwrap()
            .extend(frames.iter().copied().chain([painting, STAND, TRIDENT]));
        heard(listened);
        game.tick_item_frames();
        game.tick_paintings();
        game.tick_armor_stands().unwrap();
        game.tick_tridents();
        let phases = heard(listened);
        assert_eq!(phases.len(), frames.len() + 3);
        assert!(phases.iter().all(|(phase, _, _)| *phase == "pre"));
        assert!(game.item_frames.iter().all(|f| f.ticks == 0));
        assert_eq!(game.paintings[0].ticks, 0);
        assert_eq!(game.armor_stands[0].tick_count, 0);
        assert_eq!(game.tridents[0].age, age);
    });
}
