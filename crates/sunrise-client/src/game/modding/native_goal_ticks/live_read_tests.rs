use super::*;
use crate::game::native_entity_refs::tests::{fixture_resident_column, with_game};
use mod_api::{ActiveEffect, AttributeOperation, EntityReadError};
use std::ops::ControlFlow;
use sunrise_core::geometry::Vec3;

struct OneMob<'a>(&'a Mob);
impl mobs::LiveMobs for OneMob<'_> {
    fn find(&self, id: u32) -> Option<&Mob> {
        (self.0.id == id).then_some(self.0)
    }

    fn for_each(&self, visitor: &mut dyn FnMut(&Mob)) {
        visitor(self.0);
    }
}

/// Run the public target getter with the actor and the other mob outside `Game`.
pub(in crate::game) fn read_borrowed_target(
    game: &mut Game<'_>,
    warden: MobRef,
    other: u32,
    tick_conn: u32,
) -> Option<LivingRef> {
    let (_, view) = game
        .lan_guest_views()
        .into_iter()
        .find(|(conn, _)| *conn == tick_conn)
        .unwrap();
    let identity = game.guest_identity(tick_conn).unwrap();
    let mut loan = game.mobs.lift();
    let index = loan
        .iter()
        .position(|mob| mob.id == warden.entity().id())
        .unwrap();
    let result = loan
        .with_one_and_rest(index, |actor, before, after| {
            let live = OneMob(
                before
                    .iter()
                    .chain(after)
                    .find(|mob| mob.id == other)
                    .unwrap(),
            );
            let mut world = game.mob_world_base(&[]);
            world.goal_host = Some(game);
            world.live_mobs = Some(&live);
            world.player = Some(view);
            world.player_identity = Some(identity);
            warden.target(&GoalCx {
                m: actor,
                w: &world,
                host: game,
            })
        })
        .unwrap();
    game.mobs
        .restore(loan, sunrise_client::join_seq::RestoreOrder::SurvivorsFirst);
    result
}

#[test]
#[ignore = "daily"]
fn scoped_host_reads_live_other_mob_before_stale_snapshot() {
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut actor = Mob::new(9001, kind.clone(), Vec3::ZERO, 0.0);
        let mut other = Mob::new(9002, kind.clone(), Vec3::ZERO, 0.0);
        let stale = Mob::new(9003, kind, Vec3::ZERO, 0.0);
        let snapshot = [
            crate::game::creatures::seen(&other, game.level.player_uuid, game.items()),
            crate::game::creatures::seen(&stale, game.level.player_uuid, game.items()),
        ];
        other.yaw = 37.0;
        other.head_pitch = -12.0;
        other.attrs.max_health = 73.0;
        other.add_effect("minecraft:night_vision", 80, 1);
        sunrise_client::mobs::finalize::add_modifier(
            &mut other,
            sunrise_client::mobs::finalize::Attr::MovementSpeed,
            "test:loaned_speed",
            0.25,
            sunrise_client::mobs::finalize::Op::AddMultipliedTotal,
        );
        let live = OneMob(&other);
        let mut world = game.mob_world_base(&snapshot);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let epoch = game.native_lifecycle.epoch().unwrap();
        let entity = EntityRef::new(epoch, other.id);
        let own = LivingRef::new(EntityRef::new(epoch, actor.id));
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(entity.rotation(&context), Some([37.0, -12.0]));
        let living = LivingRef::new(entity);
        assert_eq!(living.max_health(&context), Some(73.0));
        assert_eq!(
            own.visit_effects(&context, &mut |_| panic!("ticking mob has no effects")),
            Ok(ControlFlow::Continue(()))
        );
        let mut effects = Vec::new();
        assert_eq!(
            living.visit_effects(&context, &mut |effect| {
                effects.push(effect);
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        let night_vision =
            mod_platform::__private::part_tests::effect_key("minecraft:night_vision");
        assert_eq!(
            effects,
            [ActiveEffect::new(night_vision, 80, 1, false, true, true)]
        );
        let speed = "minecraft:movement_speed";
        assert_eq!(
            own.visit_attributes(&context, &mut |id| {
                if id == speed {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            }),
            Ok(ControlFlow::Break(()))
        );
        let mut modifiers = Vec::new();
        assert_eq!(
            living.visit_attribute_modifiers(&context, speed, &mut |m| {
                modifiers.push((m.id.to_owned(), m.amount, m.operation));
                ControlFlow::Break(())
            }),
            Ok(ControlFlow::Break(()))
        );
        assert_eq!(
            modifiers,
            [(
                "test:loaned_speed".to_owned(),
                0.25,
                AttributeOperation::AddMultipliedTotal
            )]
        );
        let stale_living = LivingRef::new(EntityRef::new(epoch, stale.id));
        assert_eq!(
            stale_living.visit_effects(&context, &mut |_| panic!("unloaned effects visited")),
            Err(EntityReadError::Stale)
        );
        assert!(context.lookup_entity(other.id).is_some());
        assert!(context.lookup_entity(stale.id).is_none());
        assert!(context.lookup_entity(9999).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn scoped_host_resolves_other_real_players_from_the_game() {
    use crate::game::native_entity_refs::tests::collect_entity_visit;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_client::lan::{Joined, PlayerSender, guest::Guest};
    use sunrise_protocol::packets::configuration::ClientInformation;

    with_game(|game| {
        crate::game::native_item_places::tests::open_to_lan(game);
        for id in [12, 13] {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: id,
                    uuid: u128::from(id),
                    name: format!("guest{id}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                id as i32,
                [f64::from(id), 64.0, 5.0],
                10,
            );
            guest.placed = true;
            if id == 13 {
                guest.vehicle = Some(9002);
                guest.vehicle_order = 0;
                let mut body = sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    Vec3::new(f64::from(id), 64.0, 5.0),
                )
                .unwrap();
                body.entity_id = id as i32;
                body.set_modifier(
                    "minecraft:max_health",
                    "test:guest_health",
                    2.0,
                    "add_value",
                    true,
                )
                .unwrap();
                guest.motion = Some(Box::new(body));
            }
            game.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut actor = Mob::new(9001, kind.clone(), Vec3::ZERO, 0.0);
        actor.extra.mount = Some(9002);
        actor.extra.vehicle_order = 0;
        let vehicle = Mob::new(9002, kind.clone(), Vec3::ZERO, 0.0);
        let live = OneMob(&vehicle);
        // The player's child remains in Game, so backtracking must cross from Game
        // to the player and then the loaned vehicle before visiting the actor.
        let mut carried = Mob::new(9003, kind, Vec3::ZERO, 0.0);
        carried.extra.mount = Some(13);
        fixture_resident_column(&mut game.world, (0, 0));
        game.mobs.push(carried);
        // Select guest 12 with a PlayerView too: BOTH the selected real player
        // and the other guest stay in Game and delegate every read straight to
        // `self.host` rather than being intercepted by a partial PlayerView.
        let selected_view = game
            .lan_guest_views()
            .into_iter()
            .find(|(conn, _)| *conn == 12)
            .map(|(_, view)| view);
        let mut world = game.mob_world_base(&[]);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        world.player = selected_view;
        world.player_identity = Some(([0, 0, 0, 12], 12));
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        let selected_player = context.lookup_player(12).expect("selected guest is online");
        let selected_living = context.lookup_living(selected_player.entity()).unwrap();
        assert!(
            selected_living.health(&context).is_some(),
            "selected real player reads live health from Game"
        );
        assert!(
            selected_player.entity().rotation(&context).is_some(),
            "selected real player reads live rotation from Game"
        );
        let player = context.lookup_player(13).expect("other guest is online");
        assert_eq!(player.entity().id(), 13);
        assert_eq!(context.lookup_entity(13), Some(player.entity()));
        let living = context.lookup_living(player.entity()).unwrap();
        let mut modifiers = Vec::new();
        assert_eq!(
            living.visit_attribute_modifiers(&context, "minecraft:max_health", &mut |m| {
                modifiers.push((m.id.to_owned(), m.amount, m.operation));
                ControlFlow::Continue(())
            }),
            Ok(ControlFlow::Continue(()))
        );
        assert_eq!(
            modifiers,
            [(
                "test:guest_health".to_owned(),
                2.0,
                AttributeOperation::AddValue
            )]
        );
        assert_eq!(
            player.entity().position(&context),
            player.entity().position(game)
        );
        assert!(context.lookup_player(999).is_none());

        let vehicle = EntityRef::new(player.entity().epoch(), 9002);
        let active = EntityRef::new(vehicle.epoch(), 9001);
        let carried = EntityRef::new(vehicle.epoch(), 9003);
        assert_eq!(
            collect_entity_visit(|visit| vehicle.visit_passengers(&context, visit)),
            Ok(vec![player.entity(), active]),
            "player seats precede loaned mobs at equal boarding places"
        );
        assert_eq!(
            collect_entity_visit(|visit| vehicle.visit_indirect_passengers(&context, visit)),
            Ok(vec![player.entity(), carried, active]),
            "a player lends Game's riders before the loaned vehicle's next seat"
        );

        // A temporarily hidden player still owns its seat and the link back to the root.
        game.held_out.push(player.entity().id());
        let mut world = game.mob_world_base(&[]);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(
            collect_entity_visit(|visit| vehicle.visit_indirect_passengers(&context, visit)),
            Ok(vec![carried, active])
        );
        assert_eq!(game.held_out.pop(), Some(player.entity().id()));
    });
}

/// The body yaw is a mob's own raw `yBodyRot`, not a normalized facing or a
/// player's: the scoped goal view and the game host agree, and a stale handle
/// or another kind has none.
#[test]
#[ignore = "daily"]
fn body_yaw_is_the_raw_mob_yaw_and_none_for_others() {
    use crate::game::native_guest_turn_tests::with_guests;

    with_guests(|game| {
        fixture_resident_column(&mut game.world, (0, 0));
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(40, kind.clone(), Vec3::ZERO, 0.0);
        mob.body_yaw = -725.5;
        game.mobs.push(mob);
        let entity = game.entity(40).expect("the mob's handle");
        assert_eq!(entity.body_yaw(game), Some(-725.5));
        assert!(game.entity_set_head_yaw(entity, 45.0));
        assert_eq!(entity.body_yaw(game), Some(-725.5));

        let stale = EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert_eq!(stale.body_yaw(game), None);

        let players: Vec<_> = [12, 13]
            .map(|conn| game.player(conn).expect("guest player").entity())
            .into();
        for player in &players {
            assert_eq!(player.body_yaw(game), None);
        }
        game.others.push(crate::game::commands::OtherEntity {
            attachment_lifetime: Default::default(),
            knot_check: None,
            eye: None,
            join: Default::default(),
            display: None,
            interaction: None,
            id: 41,
            kind: "minecraft:marker".into(),
            position: Vec3::ZERO,
            yaw: 30.0,
            pitch: 0.0,
        });
        let marker = game.entity(41).expect("the marker's handle");
        assert_eq!(marker.body_yaw(game), None);

        let mut dragon = crate::game::dragon::Dragon::new(Vec3::ZERO, 540.75, 0);
        dragon.id = 42;
        game.dragon.replace(dragon);
        let dragon = game.entity(42).expect("the dragon's handle");
        assert!(game.mob(dragon).is_some());
        assert_eq!(dragon.body_yaw(game), Some(540.75));
        let stale_dragon = EntityRef::new(stale.epoch(), dragon.id());
        assert_eq!(stale_dragon.body_yaw(game), None);
        let part = game.entity(43).expect("the dragon head's handle");
        assert_eq!(part.body_yaw(game), None);

        let other = game
            .mobs
            .iter()
            .find(|mob| mob.id == 40)
            .expect("the mob row");
        let snapshot = [crate::game::creatures::seen(
            other,
            game.level.player_uuid,
            game.items(),
        )];
        let live = OneMob(other);
        let mut actor = Mob::new(9001, kind, Vec3::ZERO, 0.0);
        actor.body_yaw = 815.25;
        let own = EntityRef::new(entity.epoch(), actor.id);
        let mut world = game.mob_world_base(&snapshot);
        world.goal_host = Some(game);
        world.live_mobs = Some(&live);
        let context = GoalCx {
            m: &mut actor,
            w: &world,
            host: game,
        };
        assert_eq!(own.body_yaw(&context), Some(815.25));
        assert_eq!(entity.body_yaw(&context), Some(-725.5));
        assert_eq!(dragon.body_yaw(&context), Some(540.75));
        assert_eq!(stale_dragon.body_yaw(&context), None);
        assert_eq!(part.body_yaw(&context), None);
        assert_eq!(stale.body_yaw(&context), None);
        for player in players {
            assert_eq!(player.body_yaw(&context), None);
        }
        assert_eq!(marker.body_yaw(&context), None);
        context.m.removed = true;
        assert_eq!(own.body_yaw(&context), None);

        let role = game.options.execution_role;
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        assert_eq!(entity.body_yaw(game), None);
        assert_eq!(dragon.body_yaw(game), None);
        game.options.execution_role = role;

        game.mobs
            .iter_mut()
            .find(|mob| mob.id == 40)
            .unwrap()
            .removed = true;
        assert_eq!(entity.body_yaw(game), None);
        game.dragon.discard();
        assert_eq!(dragon.body_yaw(game), None);
    });
}
