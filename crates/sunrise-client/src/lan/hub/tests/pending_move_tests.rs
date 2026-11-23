//! Movement packet observation, vetoes, and how they order the guest's inbox.
use super::*;
pub(super) use crate::lan::guest::InboxItem;

pub(super) fn with_input_context<T>(run: impl FnOnce(&mut TickCtx<'_>) -> T) -> T {
    let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    run(&mut TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    })
}

pub(super) fn at(x: f64) -> S {
    S::MovePlayerPos(play::MovePlayerPos {
        x,
        y: 64.0,
        z: 0.0,
        flags: 1,
    })
}

/// A report that moves and claims no ground.
pub(super) fn walking(x: f64) -> S {
    S::MovePlayerPos(play::MovePlayerPos {
        x,
        y: 64.0,
        z: 0.0,
        flags: 0,
    })
}

pub(super) fn turned(yaw: f32) -> S {
    S::MovePlayerRot(play::MovePlayerRot {
        yaw,
        pitch: 0.0,
        flags: 1,
    })
}

/// A hub at tick 10 whose guest 0 stands at `(0, 64, 0)`, entered its level at tick 0 and listens (or not).
pub(super) fn standing(listened: bool, inbox: Vec<S>) -> Hub {
    let (mut hub, _rx) = hub_with_a_guest();
    hub.moves_listened = listened;
    // Long after the guest entered its level (tick 0): its first tick there is long past.
    hub.ticks = 10;
    hub.guests.update(0, |g| {
        g.load_timeout = 0;
        g.position = [0.0, 64.0, 0.0];
        g.first_good = g.position;
        g.yaw = 0.0;
        g.pitch = 0.0;
        g.last_reported = (g.position, [0.0, 0.0]);
        g.ticks = 1;
        g.inbox.extend(inbox.into_iter().map(InboxItem::uncredited));
    });
    hub
}

pub(super) fn drain(hub: &mut Hub) -> Vec<HostEvent> {
    with_input_context(|ctx| hub.drain_packets(ctx))
}

pub(super) fn moves(events: &[HostEvent]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            HostEvent::PlayerMove(conn) => Some(*conn),
            _ => None,
        })
        .collect()
}

pub(super) fn tick_end() -> S {
    S::ClientTickEnd(sunrise_protocol::packets::Empty)
}

fn move_packets(events: &[HostEvent]) -> Vec<(u32, MovePacketReport)> {
    events
        .iter()
        .filter_map(|e| match e {
            HostEvent::PlayerMovePacket(conn, report) => Some((*conn, *report)),
            _ => None,
        })
        .collect()
}

#[test]
fn observes_all_four_packet_forms_even_on_the_first_level_tick() {
    let mut hub = standing(
        true,
        vec![
            at(0.0),
            S::MovePlayerRot(play::MovePlayerRot {
                yaw: 370.0,
                pitch: 100.0,
                flags: 0,
            }),
            S::MovePlayerStatus(play::MovePlayerStatus { flags: 1 }),
            tick_end(),
            S::MovePlayerPosRot(play::MovePlayerPosRot {
                x: 0.5,
                y: 64.0,
                z: 0.0,
                yaw: 20.0,
                pitch: -15.0,
                flags: 0,
            }),
        ],
    );
    hub.move_packets_listened = true;
    hub.ticks = 1;
    hub.guests.update(0, |g| {
        g.level_entered = 0;
        (g.yaw, g.pitch) = (15.0, 5.0);
    });
    let (conn, player) = (hub.guests[0].conn, hub.guests[0].uuid);
    for (position, [yaw, pitch], [on_ground, has_position, has_rotation], remaining) in [
        ([0.0, 64.0, 0.0], [15.0, 5.0], [true, true, false], 4),
        ([0.0, 64.0, 0.0], [10.0, 100.0], [false, false, true], 3),
        ([0.0, 64.0, 0.0], [10.0, 90.0], [true, false, false], 2),
        ([0.5, 64.0, 0.0], [20.0, -15.0], [false, true, true], 0),
    ] {
        let events = drain(&mut hub);
        assert_eq!(
            move_packets(&events),
            [(
                conn,
                MovePacketReport {
                    player,
                    position,
                    yaw,
                    pitch,
                    on_ground,
                    has_position,
                    has_rotation
                }
            )],
        );
        assert!(moves(&events).is_empty(), "the first tick is not vetoable");
        assert!(hub.packets_paused());
        assert_eq!(hub.guests[0].inbox.len(), remaining);
    }
    assert!(move_packets(&drain(&mut hub)).is_empty());
    assert!(!hub.packets_paused());
    assert!(!hub.guests[0].disconnect_pending);
}

#[test]
fn observes_passenger_look_but_not_the_vehicle_report() {
    let mut hub = standing(
        true,
        vec![
            turned(30.0),
            S::MoveVehicle(play::MoveVehicleRequest {
                position: [1.0, 64.0, 0.0],
                yaw: 40.0,
                pitch: 0.0,
                on_ground: true,
            }),
        ],
    );
    hub.move_packets_listened = true;
    hub.guests.update(0, |g| {
        g.vehicle = Some(5);
        g.on_ground = false;
    });
    let (conn, player) = (hub.guests[0].conn, hub.guests[0].uuid);
    let events = drain(&mut hub);
    assert_eq!(
        move_packets(&events),
        [(
            conn,
            MovePacketReport {
                player,
                position: [0.0, 64.0, 0.0],
                yaw: 30.0,
                pitch: 0.0,
                on_ground: true,
                has_position: false,
                has_rotation: true,
            }
        )]
    );
    assert!(moves(&events).is_empty());
    assert_eq!(hub.guests[0].inbox.len(), 1);
    assert_eq!(hub.guests[0].position, [0.0, 64.0, 0.0]);
    assert_eq!(look(&hub), [30.0, 0.0]);
    assert!(!hub.guests[0].on_ground);

    let events = drain(&mut hub);
    assert!(move_packets(&events).is_empty());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HostEvent::Packet(_, p) if matches!(&**p, S::MoveVehicle(_))))
    );
}

#[test]
fn invalid_or_unloaded_reports_and_teleport_handling_are_not_observed() {
    let mut invalid = standing(false, vec![turned(f32::NAN)]);
    invalid.move_packets_listened = true;
    assert!(move_packets(&drain(&mut invalid)).is_empty());
    assert!(invalid.guests[0].disconnect_pending);

    let mut unloaded = standing(false, vec![turned(30.0)]);
    unloaded.move_packets_listened = true;
    unloaded.guests.update(0, |g| g.load_timeout = 2);
    assert!(move_packets(&drain(&mut unloaded)).is_empty());
    assert_eq!(look(&unloaded), [0.0, 0.0]);

    let mut teleporting = standing(false, vec![turned(30.0)]);
    teleporting.move_packets_listened = true;
    let at = [3.0, 64.0, 0.0];
    teleporting.teleport(0, Some(at));
    assert!(move_packets(&drain(&mut teleporting)).is_empty());
    assert_eq!(look(&teleporting), [30.0, 0.0]);
    let (id, _) = teleporting.guests[0].awaiting_teleport.unwrap();
    teleporting.guests.update(0, |g| {
        g.inbox
            .push_back(InboxItem::uncredited(S::AcceptTeleportation(
                play::AcceptTeleportation {
                    id,
                    x: at[0],
                    y: at[1],
                    z: at[2],
                    yaw: 30.0,
                    pitch: 0.0,
                },
            )));
    });
    assert!(move_packets(&drain(&mut teleporting)).is_empty());
    assert!(teleporting.guests[0].awaiting_teleport.is_none());
    assert_eq!(teleporting.guests[0].position, at);
}

#[test]
fn only_the_first_position_report_in_a_client_tick_is_observed() {
    let mut hub = standing(false, vec![at(0.5), at(0.75)]);
    hub.move_packets_listened = true;
    let events = drain(&mut hub);
    assert!(
        matches!(move_packets(&events).as_slice(), [(_, report)] if report.position == [0.5, 64.0, 0.0])
    );
    assert!(!hub.guests[0].disconnect_pending);
    assert!(move_packets(&drain(&mut hub)).is_empty());
    assert!(hub.guests[0].disconnect_pending);
    assert_eq!(hub.guests[0].position, [0.5, 64.0, 0.0]);
}

#[test]
fn movement_requiring_a_server_correction_is_not_observed() {
    let mut hub = standing(false, vec![at(50.0)]);
    hub.move_packets_listened = true;
    assert!(move_packets(&drain(&mut hub)).is_empty());
    assert_eq!(hub.guests[0].position, [0.0, 64.0, 0.0]);
    assert!(hub.guests[0].awaiting_teleport.is_some());
}

/// The guest's accepted look as `[yaw, pitch]`.
fn look(hub: &Hub) -> [f32; 2] {
    [hub.guests[0].yaw, hub.guests[0].pitch]
}

#[test]
fn loading_blocks_movement_and_attacks_until_the_load_timeout_expires() {
    for (timeout, accepted) in [(2, false), (0, true)] {
        let mut hub = standing(false, vec![S::Attack(play::Attack { id: 7 }), walking(1.0)]);
        hub.guests.update(0, |g| {
            g.loaded = false;
            g.load_timeout = timeout;
        });
        let mut events = drain(&mut hub);
        // Delivering an attack yields before the following movement packet.
        events.extend(drain(&mut hub));
        assert_eq!(
            events.iter().any(|event| matches!(event,
            HostEvent::Packet(_, packet) if matches!(&**packet, S::Attack(_)))),
            accepted
        );
        assert_eq!(
            hub.guests[0].position,
            [if accepted { 1.0 } else { 0.0 }, 64.0, 0.0]
        );
        assert!(!hub.guests[0].loaded);
    }
}

#[test]
fn two_reports_in_one_tick_are_heard_in_order_each_from_the_one_before_when_both_pass() {
    let mut hub = standing(true, vec![turned(10.0), turned(20.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn], "the first is held");
    assert_eq!(hub.guests[0].inbox.len(), 1, "the second keeps its place");
    assert_eq!(
        look(&hub),
        [0.0, 0.0],
        "nothing is applied before the ruling"
    );
    let first = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (first.from_rotation, first.to_rotation),
        ([0.0, 0.0], [10.0, 0.0])
    );
    hub.resolve_move(conn, true);
    assert_eq!(look(&hub), [10.0, 0.0]);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let second = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (second.from_rotation, second.to_rotation),
        ([10.0, 0.0], [20.0, 0.0])
    );
    hub.resolve_move(conn, true);
    assert_eq!(look(&hub), [20.0, 0.0]);
    assert!(hub.guests[0].inbox.is_empty());
}

#[test]
fn a_veto_of_the_first_of_two_reports_in_one_tick_corrects_and_the_second_is_not_heard() {
    let mut hub = standing(true, vec![turned(10.0), turned(20.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    hub.resolve_move(conn, false);
    assert_eq!(look(&hub), [0.0, 0.0], "the vetoed report is not applied");
    assert!(hub.guests[0].awaiting_teleport.is_some());
    // The second waited behind it, and meets the unconfirmed correction: only the server's own
    // handling of a report that arrives then, no hearing. While a teleport is awaited a report
    // keeps its rotation and loses its position, as the awaiting check does.
    assert!(moves(&drain(&mut hub)).is_empty());
    assert!(hub.guests[0].pending_move.is_none());
    assert!(hub.guests[0].inbox.is_empty());
    assert_eq!(look(&hub), [20.0, 0.0]);
}

#[test]
fn two_reports_in_one_tick_with_no_listener_are_committed_at_once_in_order() {
    let mut hub = standing(false, vec![turned(10.0), turned(20.0)]);
    let events = drain(&mut hub);
    assert!(moves(&events).is_empty());
    assert!(move_packets(&events).is_empty());
    assert!(!hub.packets_paused());
    assert_eq!(look(&hub), [20.0, 0.0]);
    assert!(hub.guests[0].pending_move.is_none());
    assert!(hub.guests[0].inbox.is_empty());
}

#[test]
fn the_end_of_the_client_tick_and_the_next_report_keep_their_order_behind_a_held_report() {
    let mut hub = standing(true, vec![at(0.5), tick_end(), at(0.75)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    assert_eq!(hub.guests[0].inbox.len(), 2, "tick end and report wait");
    assert!(
        hub.guests[0].position_this_tick,
        "the tick end has not been read"
    );
    hub.resolve_move(conn, true);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let g = &hub.guests[0];
    assert!(
        !g.disconnect_pending,
        "the tick end was read before the report, so it is not a second position in one client tick"
    );
    assert_eq!(g.position, [0.5, 64.0, 0.0]);
    let second = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (second.from, second.to),
        ([0.5, 64.0, 0.0], [0.75, 64.0, 0.0])
    );
}

#[test]
fn a_veto_drops_the_report_and_corrects_the_client() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    let counter = hub.guests[0].teleport_counter;
    hub.resolve_move(conn, false);
    let g = &hub.guests[0];
    assert_eq!(g.position, [0.0, 64.0, 0.0], "the report is not applied");
    assert_eq!(
        g.teleport_counter,
        counter + 1,
        "the client got the teleport correction"
    );
    assert_eq!(
        g.awaiting_teleport.map(|(_, at)| at),
        Some([0.0, 64.0, 0.0])
    );
    assert!(g.pending_move.is_none());
    assert!(g.moves.is_empty(), "its bookkeeping was skipped");
}

#[test]
fn a_report_that_changes_nothing_is_not_held() {
    let mut hub = standing(true, vec![at(0.0)]);
    let events = drain(&mut hub);
    assert!(moves(&events).is_empty(), "the same position is not heard");
    assert!(hub.guests[0].pending_move.is_none());
}

#[test]
fn a_turn_alone_is_held() {
    let mut hub = standing(true, vec![turned(30.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(report.from, report.to, "only the look changed");
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([0.0, 0.0], [30.0, 0.0])
    );
}

#[test]
fn with_no_listener_the_report_is_committed_in_place_and_nothing_is_held() {
    let mut hub = standing(false, vec![at(0.5)]);
    let events = drain(&mut hub);
    assert!(moves(&events).is_empty());
    let g = &hub.guests[0];
    assert_eq!(g.position, [0.5, 64.0, 0.0]);
    assert!(g.pending_move.is_none());
    assert!(g.inbox.is_empty());
}
