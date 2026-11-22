//! `ClientboundSetCameraPacket` (`ClientPacketListener.handleSetCamera`) and
//! `Camera.alignWithEntity`: a server's camera follows any entity the level
//! holds, not only mobs and armor stands.
use crate::game::boats::Boat;
use crate::game::remote_camera::CameraEntities;
use crate::game::tnt::PrimedTnt;
use sunrise_client::entities::ItemFrame;
use sunrise_core::geometry::Vec3;

fn boat(id: u32) -> Boat {
    Boat::new(id, "oak".into(), Vec3::new(10.5, 64.0, -3.5), 90.0)
}

fn frame(id: u32, pos: [i32; 3], facing: u8) -> ItemFrame {
    ItemFrame {
        id,
        pos,
        facing,
        glow: false,
        item: None,
        rotation: 0,
        drop_chance: 1.0,
        fixed: false,
        invisible: false,
        check_interval: 0,
        ticks: 0,
        attachment_lifetime: Default::default(),
        join: Default::default(),
        custom_name: None,
        random: sunrise_core::random::LegacyRandom::new(0),
    }
}

fn tnt(id: u32) -> PrimedTnt {
    let at = Vec3::new(4.5, 64.0, 7.5);
    PrimedTnt {
        rotation: [135.0, 0.0],
        join: Default::default(),
        attachment_lifetime: Default::default(),
        id,
        used_portal: false,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        fuse: 80,
        ticks: 0,
        ground: Default::default(),
        fall_distance: 0.0,
        stuck: Vec3::ZERO,
        in_water: false,
        owner: None,
        guest: None,
        no_gravity: false,
        power: 4.0,
        block_state: None,
    }
}

/// `ClientboundSetCameraPacket.getEntity` (`Level.getEntity`): the level holds
/// the id whatever kind of entity it names, and `Camera.alignWithEntity` reads
/// that entity — a boat's interpolated position, view rotation and
/// `EntityType.BOAT`'s eyes at 0.5625, with no living entity's scale; a
/// frame's position and zero eyes; primed TNT's 0.15.
#[test]
fn a_server_camera_takes_any_entity_the_level_holds() {
    let mut boat = boat(9);
    boat.previous = Vec3::new(10.0, 64.0, -3.5);
    boat.previous_yaw = 45.0;
    let frame = frame(11, [2, 64, -5], 3);
    let tnt = tnt(13);
    let displays = indexmap::IndexMap::new();
    let interactions = std::collections::HashMap::from([(
        15,
        crate::game::interactions::RemoteInteraction {
            position: Vec3::new(0.5, 300.0, 0.5),
            width: 0.1,
            height: 1.9059,
            response: false,
            vehicle: None,
        },
    )]);
    let entities = CameraEntities {
        mobs: &[],
        stands: &[],
        boats: std::slice::from_ref(&boat),
        frames: std::slice::from_ref(&frame),
        tnts: std::slice::from_ref(&tnt),
        displays: &displays,
        interactions: &interactions,
        tracked: None,
    };

    let view = entities.view(9).expect("a boat is a camera");
    assert_eq!(view.position, [10.5, 64.0, -3.5]);
    assert_eq!(view.previous, [10.0, 64.0, -3.5]);
    assert_eq!((view.y_rot_o, view.y_rot), (45.0, 90.0));
    assert_eq!(view.eye_height, 0.5625);
    assert!(view.living.is_none(), "a boat has no camera distance");

    let view = entities.view(11).expect("an item frame is a camera");
    // `ItemFrame.createBoundingBox`: the block's centre pulled 0.46875 out of the
    // wall it hangs on (south: z - 0.46875), which `HangingEntity` makes its pos.
    assert_eq!(view.position, [2.5, 64.5, -4.96875]);
    assert_eq!((view.eye_height, view.y_rot), (0.0, 0.0));

    let view = entities.view(13).expect("primed TNT is a camera");
    assert_eq!(view.position, [4.5, 64.0, 7.5]);
    assert_eq!((view.eye_height, view.y_rot), (0.15, 135.0));

    let view = entities.view(15).expect("an interaction is a camera");
    assert_eq!(view.position, [0.5, 300.0, 0.5]);
    assert_eq!(view.eye_height.to_bits(), (1.9059_f32 * 0.85_f32).to_bits());
    assert!(view.living.is_none());

    // An id the level does not hold changes nothing.
    assert!(!entities.holds(10));
    assert!(entities.view(10).is_none());
}

/// `HangingEntity.setDirection`: a frame turns `get2DDataValue() * 90` in yaw
/// (west 90, north 180, east 270), and on the ceiling or the floor
/// `-90 * getAxisDirection().getStep()` in pitch with no yaw.
#[test]
fn a_frame_camera_turns_with_its_facing() {
    let view = |facing| {
        let frame = frame(1, [0; 3], facing);
        let displays = indexmap::IndexMap::new();
        let interactions = std::collections::HashMap::new();
        let entities = CameraEntities {
            mobs: &[],
            stands: &[],
            boats: &[],
            frames: std::slice::from_ref(&frame),
            tnts: &[],
            displays: &displays,
            interactions: &interactions,
            tracked: None,
        };
        let view = entities.view(1).unwrap();
        (view.y_rot, view.x_rot)
    };
    assert_eq!(view(4), (90.0, 0.0));
    assert_eq!(view(2), (180.0, 0.0));
    assert_eq!(view(5), (270.0, 0.0));
    assert_eq!(view(1), (0.0, -90.0));
    assert_eq!(view(0), (0.0, 90.0));
}

#[test]
fn a_server_camera_follows_a_tracked_display() {
    use crate::game::text_displays::{RemoteTextDisplay, Shows};
    let display = RemoteTextDisplay::new(Shows::Text, [3.0, 70.0, -4.0], 90.0).with_pitch(15.0);
    let mut displays = indexmap::IndexMap::new();
    displays.insert(42, display);
    let interactions = std::collections::HashMap::new();
    let entities = CameraEntities {
        mobs: &[],
        stands: &[],
        boats: &[],
        frames: &[],
        tnts: &[],
        displays: &displays,
        interactions: &interactions,
        tracked: None,
    };
    assert!(entities.holds(42));
    let view = entities
        .view(42)
        .expect("a display is a valid camera entity");
    assert_eq!(view.position, [3.0, 70.0, -4.0]);
    assert_eq!(view.previous, [3.0, 70.0, -4.0]);
    assert_eq!((view.y_rot_o, view.y_rot), (90.0, 90.0));
    assert_eq!((view.x_rot_o, view.x_rot), (15.0, 15.0));
    assert_eq!(view.eye_height, 0.0);
    assert!(view.living.is_none());
}
