use super::{Game, GuiDraw, TrackedWaypoint};
use crate::game::{
    in_own_process, native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
    remote::RemoteDimensionEntry,
};
use std::time::Duration;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::{level_loading::Reason, locator_bar::WaypointStyles, motion::PlayerMotion};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{
        self, Clientbound as C, SpawnInfo, WaypointId, WaypointTarget, waypoint_operation,
    },
};

fn connect(game: &mut Game<'_>, position: Vec3) -> Pipe {
    game.native_lifecycle = NativeLifecycle::default();
    let (client, server) = Pipe::pair();
    game.connect_local(client, "LocatorTest").unwrap();
    game.enter_remote_dimension(
        &SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        RemoteDimensionEntry::Login,
        Reason::Other,
    );
    let remote = game.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = 1;
    remote.level_load.finish();
    game.player = Some(PlayerMotion::new(game.motion, position).unwrap());
    game.previous_position = position;
    server
}

fn add_armor_stand(game: &mut Game<'_>, id: i32, uuid: u128, position: [f64; 3]) {
    let kind = game
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id("minecraft:armor_stand")
        .unwrap();
    game.apply_packet(C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind,
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

fn track_block(game: &mut Game<'_>, uuid: u128, color: i32, position: [i32; 3]) {
    game.handle_waypoint(&TrackedWaypoint {
        operation: waypoint_operation::TRACK,
        id: WaypointId::Uuid(uuid),
        style: "minecraft:test".into(),
        color: Some(-16_777_216 | color),
        target: WaypointTarget::Block(position),
    });
}

#[test]
#[ignore = "daily"]
fn spectated_locator_uses_camera_entity_for_identity_and_distance() {
    in_own_process(
        "game::locator_bar::tests::spectated_locator_uses_camera_entity_for_identity_and_distance",
        || {
            with_client_game(|game| {
                let _server = connect(game, Vec3::new(0.0, 64.0, 0.0));
                let local_uuid = game.own_uuid();
                add_armor_stand(game, 20, 2, [100.0, 64.0, 0.0]);
                game.apply_packet(C::SetCamera(play::SetCamera { camera_id: 20 }))
                    .unwrap();

                // Keep all three witness waypoints visible by placing the render camera
                // behind them; the entity itself still faces south at x = 100.
                game.locator_camera = Some(([-10.0, 66.0, 0.0], -90.0, 45.0, 70.0));
                game.waypoint_styles = WaypointStyles::load(&ResourceSource::indexed(
                    [(
                        "assets/minecraft/waypoint_style/test.json".into(),
                        Resource::bytes(
                            br#"{"near_distance":30,"far_distance":80,"sprites":["near","middle","far"]}"#
                                .to_vec(),
                        ),
                    )]
                    .into(),
                ));
                track_block(game, 3, 3, [122, 64, 6]);
                track_block(game, 2, 2, [100, 64, 0]);
                track_block(game, local_uuid, 1, [0, 64, 0]);
                track_block(game, 4, 4, [150, 64, 0]);

                let assert_draws = |game: &Game<'_>| {
                    let draws = game.locator_bar_draws(427, 240);
                    let sprites: Vec<_> = draws
                        .iter()
                        .filter_map(|d| match d {
                            GuiDraw::Sprite {
                                color, sprite, x, ..
                            } => Some((color & 0xff, sprite.as_ref(), *x)),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        sprites,
                        [
                            (1, "hud/locator_bar_dot/far", 209),
                            (255, "hud/locator_bar_arrow_up", 210),
                            (4, "hud/locator_bar_dot/middle", 209),
                            (255, "hud/locator_bar_arrow_up", 210),
                            (3, "hud/locator_bar_dot/near", 213),
                            (255, "hud/locator_bar_arrow_up", 214),
                        ]
                    );
                };
                assert_draws(game);
                // Removing the entity from the level does not replace Minecraft's camera.
                game.apply_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![20] }))
                    .unwrap();
                assert_draws(game);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn block_waypoints_use_live_non_player_eyes_within_manhattan_three() {
    in_own_process(
        "game::locator_bar::tests::block_waypoints_use_live_non_player_eyes_within_manhattan_three",
        || {
            with_client_game(|game| {
                let _server = connect(game, Vec3::new(100.0, 64.0, 0.0));
                add_armor_stand(game, 30, 3, [120.2, 64.0, 5.8]);
                game.armor_stands
                    .iter_mut()
                    .find(|a| a.id == 30)
                    .unwrap()
                    .previous = Vec3::new(119.2, 64.0, 5.8);
                // Offset the witness yaw slightly so the two relevant angles land in
                // different pixels, without changing the waypoint or entity positions.
                game.locator_camera = Some(([96.0, 66.0, 0.0], -90.25, 0.0, 70.0));
                game.clock.reset();
                game.clock.set_frozen(false);
                assert!(game.clock.tick_world_runs());
                game.clock.end_frame(false);
                assert_eq!(game.clock.advance(Duration::from_millis(25), 50.0), 0);

                let dot_x = |game: &Game<'_>| {
                    let draws = game.locator_bar_draws(427, 240);
                    let [GuiDraw::Sprite { x, w: 9, .. }] = draws.as_slice() else {
                        panic!("expected one dot, got {draws:?}");
                    };
                    *x
                };
                let (block, eyes) = game.waypoint_entity(3).unwrap();
                assert_eq!(block, [120, 64, 5]);
                let interpolated_eye = [
                    0x405d_eccc_cccc_cccd,
                    0x4050_71c2_8f80_0000,
                    0x4017_3333_3333_3333,
                ];
                assert_eq!(eyes.map(f64::to_bits), interpolated_eye);
                track_block(game, 3, 3, [122, 64, 6]);
                assert_eq!(dot_x(game), 229, "distance 3: interpolated eye");
                track_block(game, 3, 3, [123, 64, 6]);
                assert_eq!(dot_x(game), 228, "distance 4: block centre");

                // The real DeltaTracker.Timer returns 1.0 for a frozen non-player.
                game.clock.set_frozen(true);
                assert!(!game.clock.tick_world_runs());
                // Minecraft.runTick updates the timer's frozen flag after rendering.
                assert_eq!(
                    game.waypoint_entity(3).unwrap().1.map(f64::to_bits),
                    interpolated_eye
                );
                game.clock.end_frame(false);
                let (_, eyes) = game.waypoint_entity(3).unwrap();
                assert_eq!(
                    eyes.map(f64::to_bits),
                    [
                        0x405e_0ccc_cccc_cccd,
                        0x4050_71c2_8f80_0000,
                        0x4017_3333_3333_3333
                    ],
                );
                assert_eq!(dot_x(game), 228, "distance 4: centre ignores partial tick");
                track_block(game, 3, 3, [122, 64, 6]);
                assert_eq!(dot_x(game), 228, "distance 3: frozen eye");
                game.apply_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![30] }))
                    .unwrap();
                assert_eq!(dot_x(game), 229, "entity absent: block centre");
            });
        },
    );
}
