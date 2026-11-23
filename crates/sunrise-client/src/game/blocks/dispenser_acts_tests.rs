//! `dispenser_acts` at a real dispenser: an accepted global answer wins over the item's Part
//! and vanilla, a refused one leaves the choice to them, and an item the world's feature flags
//! disable is dropped as `DefaultDispenseItemBehavior` drops it with no one asked.
use crate::game::{Game, dispense_fixture, native_entity_refs::tests::run_fresh_cases};
use mod_api::*;
use std::sync::{Arc, Mutex};
use sunrise_client::{chunk::Chunk, creative::Creative, items::ItemStack};
use sunrise_core::geometry::Direction;

/// The dispenser, facing east.
const DISPENSER: [i32; 3] = [5, 64, 5];

/// What the global listener answers, and who was asked.
#[derive(Clone, Default)]
struct Calls {
    answer_event: Arc<Mutex<i32>>,
    asked: Arc<Mutex<Vec<&'static str>>>,
}

/// The arrow's own Part: it would shoot nothing and keep the stack, failing.
struct ArrowPart(Calls);
#[mod_api::sunrise_part]
impl ItemPart for ArrowPart {
    fn dispense_item(
        &self,
        _: &mut dyn ServerHost,
        event: &mut DispenseItemEvent<'_>,
    ) -> Option<Dispensed> {
        self.0.asked.lock().unwrap().push("part");
        Some((Some(event.stack.clone()), 0))
    }
}

/// Every dispenser's global answer: one item fewer, with the configured event.
struct Global(Calls);
#[mod_api::__subscriptions]
impl Hooks for Global {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let arrow = r.item_key("minecraft:arrow");
        r.item_part(arrow, ArrowPart(self.0.clone()));
    }
    fn dispenser_acts(
        &self,
        _: &mut dyn ServerHost,
        event: &mut DispenserActsEvent<'_>,
    ) -> Option<Dispensed> {
        self.0.asked.lock().unwrap().push("global");
        let mut left = event.stack.clone();
        left.set_count(event.stack.count() - 1).unwrap();
        Some((Some(left), *self.0.answer_event.lock().unwrap()))
    }
}

/// A dedicated server with a chunk at the origin and a dispenser facing east.
fn with_dispenser(calls: &Calls, test: impl FnOnce(&mut Game<'_>)) {
    let mods: Mods = vec![ModEntry::new("global", Box::new(Global(calls.clone())))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            let level = game.level("minecraft:overworld").unwrap();
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dispenser")
                .unwrap();
            let dispenser = game.with_props(base, &[("facing", "east")]).unwrap();
            assert_eq!(
                game.level_set_block(level, DISPENSER, dispenser, BlockUpdateFlags::NONE),
                Ok(true)
            );
            test(game);
        });
    });
}

/// The dispenser dispenses two arrows: what its slot holds then, and the level events.
fn dispense_two_arrows(game: &mut Game<'_>) -> (Option<(String, u32)>, Vec<String>) {
    let at = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
    dispense_fixture::start_recording();
    let left = game
        .dispense_item(at, Direction::East, ItemStack::new("minecraft:arrow", 2))
        .unwrap()
        .continue_value()
        .unwrap();
    let lines = dispense_fixture::stop_recording();
    let left = left.map(|stack| (stack.item.to_string(), stack.count));
    (left, lines)
}

/// The level events among `lines`, as `(event, data)`.
fn level_events(lines: &[String]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter_map(|line| {
            let mut words = line.strip_prefix("L ")?.split(' ');
            let event = words.next()?.to_owned();
            let _pos = words.next()?;
            Some((event, words.next()?.to_owned()))
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn dispenser_acts_chooses_before_the_parts_and_after_the_feature_flags() {
    if run_fresh_cases(
        "game::dispenser_acts_tests::dispenser_acts_chooses_before_the_parts_and_after_the_feature_flags",
        "SUNRISE_DISPENSER_ACTS_CASE",
        &["accepted", "refused", "disabled"],
    ) {
        return;
    }
    let case = std::env::var("SUNRISE_DISPENSER_ACTS_CASE").unwrap();
    let calls = Calls::default();
    with_dispenser(&calls, |game| match case.as_str() {
        // The fail sound, then the smoke; one arrow fewer, nothing shot or dropped, and the
        // arrow's Part never asked.
        "accepted" => {
            *calls.answer_event.lock().unwrap() = 1001;
            let (left, lines) = dispense_two_arrows(game);
            assert_eq!(left, Some(("minecraft:arrow".into(), 1)));
            assert_eq!(*calls.asked.lock().unwrap(), ["global"]);
            assert_eq!(
                level_events(&lines),
                [("1001".into(), "0".into()), ("2000".into(), "5".into())],
                "{lines:?}"
            );
            assert!(game.dropped.is_empty());
        }
        // Event 7 is no dispenser event: refused, so the arrow's Part answers.
        "refused" => {
            *calls.answer_event.lock().unwrap() = 7;
            let (left, lines) = dispense_two_arrows(game);
            assert_eq!(left, Some(("minecraft:arrow".into(), 2)));
            assert_eq!(*calls.asked.lock().unwrap(), ["global", "part"]);
            assert!(level_events(&lines).is_empty(), "{lines:?}");
        }
        // Arrows behind a flag the world lacks: one dropped, with click and smoke, no one asked.
        "disabled" => {
            *calls.answer_event.lock().unwrap() = 1000;
            game.creative_screen = Creative::from_json(&serde_json::json!({
                "schema": 4,
                "item_features": {"minecraft:arrow": ["test:not_enabled"]},
                "potion_features": {},
                "tabs": [{
                    "id": "minecraft:search", "title": "minecraft:search", "type": "SEARCH",
                    "build": []
                }]
            }))
            .unwrap();
            let (left, lines) = dispense_two_arrows(game);
            assert_eq!(left, Some(("minecraft:arrow".into(), 1)));
            assert!(calls.asked.lock().unwrap().is_empty());
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(&*game.dropped[0].stack.item, "minecraft:arrow");
            assert_eq!(
                level_events(&lines),
                [("1000".into(), "0".into()), ("2000".into(), "5".into())],
                "{lines:?}"
            );
        }
        _ => unreachable!("{case}"),
    });
}
