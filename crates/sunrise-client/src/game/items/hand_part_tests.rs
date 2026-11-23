#![cfg(test)]
//! A stick's `ItemPart` answers `should_cause_reequip_animation` and
//! `should_cause_block_break_reset` (`IItemExtension.shouldCauseReequipAnimation`,
//! `shouldCauseBlockBreakReset`) on the local player's client.
use super::Game;
use mod_api::{
    ClientHost, Hooks, ItemPart, Registrar, RegistrationContext, ShouldCauseBlockBreakResetEvent,
    ShouldCauseReequipAnimationEvent,
};
use std::sync::Mutex;
use sunrise_client::items::ItemStack;

/// What the part answers: `None` leaves vanilla's answer.
static REEQUIP: Mutex<Option<bool>> = Mutex::new(None);
static RESET: Mutex<Option<bool>> = Mutex::new(None);
/// What the part was asked: `(old count, stack count, hand, slot, slot changed, vanilla's
/// answer)` and, for a break, whether the break's stack had components.
type Asked = (u32, u32, mod_api::Hand, mod_api::EquipmentSlot, bool, bool);
static REEQUIP_ASKED: Mutex<Vec<Asked>> = Mutex::new(Vec::new());
static RESET_ASKED: Mutex<Vec<bool>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Stick;
#[mod_api::sunrise_part]
impl ItemPart for Stick {
    fn should_cause_reequip_animation(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseReequipAnimationEvent<'_>,
    ) {
        REEQUIP_ASKED.lock().unwrap().push((
            event.old.count(),
            event.stack.count(),
            event.hand,
            event.slot,
            event.slot_changed,
            event.reequip,
        ));
        if let Some(answer) = *REEQUIP.lock().unwrap() {
            event.reequip = answer;
        }
    }
    fn should_cause_block_break_reset(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseBlockBreakResetEvent<'_>,
    ) {
        RESET_ASKED
            .lock()
            .unwrap()
            .push(event.stack.components().is_some());
        if let Some(answer) = *RESET.lock().unwrap() {
            event.reset = answer;
        }
    }
}
struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:stick");
        r.item_part(key, Stick);
    }
}

/// Runs `body` on a game whose stick has the part.
fn with_stick_part(body: impl FnOnce(&mut Game<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("hands", Box::new(Parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(body);
    });
}

/// Puts `main` in slot 0, selects it, and ticks until the attack strength is
/// full and the hand is up.
fn settle(game: &mut Game<'_>, main: ItemStack) {
    game.inventory[0] = Some(main);
    game.selected = 0;
    for _ in 0..20 {
        game.tick_hand();
    }
    assert_eq!(game.heights.main, 1.0);
    REEQUIP_ASKED.lock().unwrap().clear();
}

/// The main hand's height over the next six ticks, and the stick count it
/// shows after each.
fn dip(game: &mut Game<'_>) -> Vec<(f32, u32)> {
    (0..6)
        .map(|_| {
            game.tick_hand();
            let shown = game.visible_hands[0].1.as_ref().map_or(0, |s| s.count);
            (game.heights.main, shown)
        })
        .collect()
}

fn close(got: &[(f32, u32)], want: &[(f32, u32)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.0 - w.0).abs() < 1e-6 && g.1 == w.1,
            "{got:?} != {want:?}"
        );
    }
}

/// The calls the part has had since the last look.
fn asked() -> Vec<Asked> {
    REEQUIP_ASKED.lock().unwrap().drain(..).collect()
}

#[test]
#[ignore = "daily"]
fn a_part_decides_whether_the_hand_lowers_for_a_new_stack() {
    use crate::game::native_entity_refs::tests::run_fresh_cases;
    use mod_api::{EquipmentSlot, Hand};

    if run_fresh_cases(
        "game::hand_part_tests::a_part_decides_whether_the_hand_lowers_for_a_new_stack",
        "SUNRISE_REEQUIP_PART_CASE",
        &["reequip"],
    ) {
        return;
    }
    with_stick_part(|game| {
        let stick = |count| ItemStack::new("minecraft:stick", count);
        // From one stick in slot 0 to an equal one in slot 1 while the off hand's stick
        // becomes five stone. The main hand is replaced at once (equal stacks) before the part
        // hears it, with the new slot; the off hand still shows the stick, so only the
        // stick's part is asked, with vanilla's `true`. Its `false` shows the stone at once.
        game.offhand = Some(stick(1));
        settle(game, stick(1));
        game.inventory[1] = Some(stick(1));
        game.selected = 1;
        game.offhand = Some(ItemStack::new("minecraft:stone", 5));
        *REEQUIP.lock().unwrap() = Some(false);
        game.tick_hand();
        assert_eq!(
            asked(),
            [
                (1, 1, Hand::Main, EquipmentSlot::MainHand, true, false),
                (1, 5, Hand::Off, EquipmentSlot::OffHand, false, true),
            ]
        );
        assert_eq!(game.heights.off, 1.0);
        assert_eq!(
            game.visible_hands[1].1.as_ref().map(|s| &*s.item),
            Some("minecraft:stone")
        );
        // The next check in the same slot has no slot change.
        game.tick_hand();
        assert!(!asked()[0].4);
        game.offhand = None;

        // An equal stick the hand already shows, with the part answering `true`: the
        // hand drops.
        *REEQUIP.lock().unwrap() = None;
        settle(game, stick(1));
        *REEQUIP.lock().unwrap() = Some(true);
        game.tick_hand();
        close(&[(game.heights.main, 1)], &[(0.6, 1)]);
        *REEQUIP.lock().unwrap() = None;

        // From one stick to two in the same slot: vanilla dips, the part's `false` shows
        // them at once.
        settle(game, stick(1));
        game.inventory[0] = Some(stick(2));
        game.hand_stack_replaced(0);
        close(&dip(game)[..2], &[(0.6, 1), (0.2, 1)]);
        assert_eq!(
            asked()[0],
            (1, 2, Hand::Main, EquipmentSlot::MainHand, false, true)
        );
        settle(game, stick(1));
        *REEQUIP.lock().unwrap() = Some(false);
        game.inventory[0] = Some(stick(2));
        game.hand_stack_replaced(0);
        close(&dip(game)[..1], &[(1.0, 2)]);
        *REEQUIP.lock().unwrap() = None;

        // An empty hand on either side asks nothing and leaves the slot history: a stick
        // put back after an empty hand in another slot still counts as the slot change
        // from the last asked check.
        settle(game, stick(1));
        game.inventory[0] = None;
        for _ in 0..3 {
            game.tick_hand();
        }
        assert!(game.visible_hands[0].1.is_none(), "taken at the bottom");
        game.inventory[2] = Some(stick(1));
        game.selected = 2;
        game.tick_hand();
        assert!(
            asked().is_empty(),
            "an empty hand on either side asks nothing"
        );
        for _ in 0..6 {
            game.tick_hand();
        }
        assert!(asked().iter().any(|call| call.4), "slot 0 to slot 2");
    });
}

#[test]
#[ignore = "daily"]
fn a_part_decides_whether_a_changed_stick_restarts_the_break() {
    crate::game::in_own_process(
        "game::hand_part_tests::a_part_decides_whether_a_changed_stick_restarts_the_break",
        || {
            with_stick_part(|game| {
                game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
                game.selected = 0;
                let began = game.main_hand_key();
                // The same stick, a few more of it or a little damage: the same break,
                // and the part sees it asked.
                assert!(game.same_destroying_item(&began));
                game.inventory[0].as_mut().unwrap().count = 3;
                assert!(game.same_destroying_item(&began));
                assert_eq!(*RESET_ASKED.lock().unwrap(), [false, false]);

                // The stick gains a component (a name): vanilla restarts the break.
                game.inventory[0].as_mut().unwrap().data_mut().custom_name =
                    Some(sunrise_core::nbt::Tag::String("Wand".into()));
                assert!(!game.same_destroying_item(&began));
                // The part's `false` keeps it going; it saw the stick the break began with.
                *RESET.lock().unwrap() = Some(false);
                assert!(game.same_destroying_item(&began));
                assert_eq!(*RESET_ASKED.lock().unwrap(), [false, false, false, false]);
                // A break begun with an empty hand asks nothing.
                RESET_ASKED.lock().unwrap().clear();
                assert!(!game.same_destroying_item(&None));
                assert!(RESET_ASKED.lock().unwrap().is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn without_a_part_the_break_compares_item_and_components() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
        game.selected = 0;
        let began = game.main_hand_key();
        // A use shrinks the stack in place: the same break.
        game.inventory[0].as_mut().unwrap().count = 5;
        assert!(game.same_destroying_item(&began));
        // The server's new stack with more wear: a damage component apart, so it restarts.
        game.inventory[0].as_mut().unwrap().damage = 2;
        assert!(!game.same_destroying_item(&began));
        game.inventory[0] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
        assert!(!game.same_destroying_item(&began));
        game.inventory[0] = None;
        assert!(!game.same_destroying_item(&began));
        assert!(game.same_destroying_item(&None));
    });
}

#[test]
#[ignore = "daily"]
fn remote_mining_component_update_restarts_unless_the_part_refuses() {
    crate::game::in_own_process(
        "game::hand_part_tests::remote_mining_component_update_restarts_unless_the_part_refuses",
        || {
            for keep in [false, true] {
                with_stick_part(|game| {
                    use std::sync::Arc;
                    use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
                    use sunrise_core::geometry::Vec3;
                    use sunrise_protocol::packets::play;
                    *RESET.lock().unwrap() = keep.then_some(false);
                    // Play packets are suppressed until login/configuration finishes.
                    let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                    peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                        .unwrap();
                    game.set_local_mode(sunrise_client::command::args::GameMode::Survival);
                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap();
                    player.ground.on_ground = true;
                    game.player = Some(player);
                    game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
                    game.selected = 0;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(0, 64, 0, stone).unwrap();
                    game.forced_hit = Some(crate::game::Hit {
                        pos: [0, 64, 0],
                        face: [0, 1, 0],
                        point: [0.5, 65.0, 0.5],
                        inside: false,
                    });
                    game.refresh_client_pick(1.0);
                    game.forced_hit = None;
                    for _ in 0..4 {
                        game.remote_continue_attack(true);
                    }
                    let before = game.mining.as_ref().unwrap().progress;
                    while peer.recv().is_ok() {}
                    game.inventory[0].as_mut().unwrap().data_mut().custom_name =
                        Some(sunrise_core::nbt::Tag::String("Wand".into()));
                    game.remote_continue_attack(true);
                    let mut actions = Vec::new();
                    while let Ok((id, bytes)) = peer.recv() {
                        if let Ok(play::Serverbound::PlayerAction(action)) =
                            play::Serverbound::decode(id, &bytes)
                        {
                            actions.push(action.action);
                        }
                    }
                    let after = game.mining.as_ref().unwrap().progress;
                    if keep {
                        assert!(actions.is_empty());
                        assert!((after - before - 1.0 / 150.0).abs() < 1e-6);
                    } else {
                        assert_eq!(actions, [2, 0]);
                        assert_eq!(after, 0.0);
                    }
                });
            }
        },
    );
}
