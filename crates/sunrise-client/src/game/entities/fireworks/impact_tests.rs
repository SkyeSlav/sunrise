//! Real rocket tick coverage for impact payloads and canceled explosions.
use super::*;
use crate::game::native_projectiles::impact_probe::{Action, point, with_probe};
use sunrise_client::mobs::Mob;

fn launch(game: &mut Game<'_>, lifetime: i32) {
    let mut r = Rocket::remote(71, Vec3::new(0.5, 64.5, 0.5), Vec3::new(4.0, 0.0, 0.0));
    r.angled = true;
    r.owner = Some(Who::Player);
    r.left_owner = true;
    r.lifetime = lifetime;
    std::sync::Arc::make_mut(&mut r.explosions).push(FireworkExplosion::default());
    game.rockets.push(r);
}

#[test]
#[ignore = "daily"]
fn rocket_hits_report_geometry_and_cancellation_skips_the_explosion() {
    for entity in [false, true] {
        for action in [Action::Allow, Action::Cancel] {
            with_probe(action, true, |game, observed| {
                if entity {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs
                        .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
                } else {
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(2, 64, 0, stone).unwrap();
                }
                launch(game, 100);
                game.tick_rockets();
                let hits = observed.hits.lock().unwrap();
                assert_eq!(hits.len(), 1);
                let hit = &hits[0];
                assert_eq!(hit.projectile, 71);
                assert_eq!(hit.kind, "minecraft:firework_rocket");
                assert_eq!(hit.owner, 7);
                assert_eq!(hit.dimension, "minecraft:overworld");
                if entity {
                    assert_eq!(hit.entity, Some(72));
                    assert_eq!(hit.entity_kind, "minecraft:pig");
                    point(hit.at, [2.55, 64.5, 0.5]);
                } else {
                    assert_eq!(hit.entity, None);
                    assert_eq!(hit.block, [2, 64, 0]);
                    assert_eq!(hit.face, [-1, 0, 0]);
                    point(hit.at, [2.0, 64.5, 0.5]);
                }
                if matches!(action, Action::Cancel) {
                    assert_eq!(game.rockets.len(), 1);
                    assert!(game.entity(71).is_some());
                    assert_eq!(game.rockets[0].life, 1);
                    if entity {
                        assert_eq!(game.mobs[0].health, 10.0);
                    }
                } else {
                    assert!(game.rockets.is_empty());
                    if entity {
                        assert!(game.mobs[0].health < 10.0);
                    }
                }
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn rockets_skip_misses_deflections_and_a_closed_gate_but_keep_lifetime_expiry() {
    for case in 0..5 {
        let action = if case == 4 {
            Action::Allow
        } else {
            Action::Cancel
        };
        with_probe(action, case != 2, |game, observed| {
            if case == 1 {
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:breeze")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
            } else if case >= 2 {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
            }
            launch(game, if case >= 3 { 0 } else { 100 });
            if case == 4 {
                // A plain rocket still reports its block hit, but does not
                // explode on that hit even when it is allowed.
                game.rockets[0].explosions = Default::default();
                game.rockets[0].lifetime = 100;
            }
            game.tick_rockets();
            assert_eq!(observed.hits.lock().unwrap().len(), usize::from(case >= 3));
            match case {
                1 => {
                    assert_eq!(game.rockets.len(), 1);
                    assert_eq!(game.rockets[0].velocity.x, -2.0);
                }
                2 | 3 => assert!(game.rockets.is_empty()),
                _ => assert_eq!(game.rockets.len(), 1),
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_crowd_of_rockets_ticks_in_place_keeping_order_and_identity() {
    with_probe(Action::Allow, true, |game, _| {
        for id in 0..6 {
            let mut r = Rocket::remote(
                100 + id,
                Vec3::new(0.5 + f64::from(id) * 4.0, 90.5, 0.5),
                Vec3::new(0.0, 0.2, 0.0),
            );
            r.angled = true;
            r.left_owner = true;
            r.lifetime = 100;
            game.rockets.push(r);
        }
        let joins: Vec<_> = game.rockets.iter().map(|r| (r.id, r.join)).collect();
        for tick in 1..=3 {
            game.tick_rockets();
            // Continuing flights keep every resident's identity and order.
            assert_eq!(
                game.rockets
                    .iter()
                    .map(|r| (r.id, r.join))
                    .collect::<Vec<_>>(),
                joins
            );
            assert!(game.rockets.iter().all(|r| r.life == tick));
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        }
        // Expired rockets permanently leave the list, the others stay put.
        game.rockets[2].lifetime = 0;
        game.tick_rockets();
        assert_eq!(game.rockets.len(), 5);
        assert!(game.rockets.iter().all(|r| r.id != 102));
        assert_eq!(
            game.rockets.iter().map(|r| r.id).collect::<Vec<_>>(),
            [100, 101, 103, 104, 105]
        );
    });
}

#[test]
#[ignore = "daily"]
fn rocket_section_listener_reads_the_advanced_position() {
    with_probe(Action::ReadSectionPosition, false, |game, observed| {
        launch(game, 100);
        game.rockets[0].position = Vec3::new(0.5, 79.5, 0.5);
        game.rockets[0].velocity = Vec3::new(0.0, 1.0, 0.0);
        game.tick_rockets();

        let pos = game.rockets[0].position;
        assert!(pos.y >= 80.0);
        assert_eq!(
            *observed.section_positions.lock().unwrap(),
            [Some([pos.x, pos.y, pos.z])]
        );
    });
}

#[test]
#[ignore = "daily"]
fn rocket_impact_listener_can_save_the_advanced_rocket() {
    crate::game::in_own_process(
        "game::fireworks::impact::tests::rocket_impact_listener_can_save_the_advanced_rocket",
        || {
            with_probe(Action::SaveOnImpact, true, |game, observed| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
                launch(game, 100);
                game.tick_rockets();

                let hits = observed.hits.lock().unwrap();
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].projectile, 71);
                let saved = game.save.load_entities(0, 0).unwrap();
                let rocket = saved
                    .iter()
                    .find(|tag| {
                        sunrise_client::entity_nbt::kind(tag).as_deref()
                            == Some("minecraft:firework_rocket")
                    })
                    .expect("impact save includes the resident rocket");
                let saved_position = sunrise_client::entity_nbt::position(rocket).unwrap();
                assert_eq!(
                    [saved_position.x, saved_position.y, saved_position.z].map(f64::to_bits),
                    hits[0].projectile_pos.map(f64::to_bits),
                );
                assert!(hits[0].projectile_pos[0] > 0.5);
            });
        },
    );
}
