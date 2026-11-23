//! The game's dispatch for the hub's movement veto and packet observation hooks.
use super::Game;
use mod_api::{__private::PlayerMovePacketCall, HookId, Vec3};
use sunrise_client::lan::hub::{Hub, MovePacketReport};

impl Game<'_> {
    /// What the hub needs to know of the loaded mods each tick.
    pub(in crate::game) fn sync_hub_mod_flags(hub: &mut Hub) {
        hub.mods_loaded = !mod_api::__private::mods().is_empty();
        hub.moves_listened = mod_api::__private::listened(HookId::PlayerMove);
        hub.move_packets_listened = mod_api::__private::listened(HookId::PlayerMovePacket);
    }

    /// Observe the snapshot after the report's handling, with the reporting player acting.
    pub(in crate::game) fn lan_player_move_packet(&mut self, conn: u32, report: MovePacketReport) {
        self.as_guest(conn, |game| {
            let Some(player) = game
                .native_acting_player()
                .filter(|p| p.uuid() == report.player)
            else {
                return;
            };
            let [x, y, z] = report.position;
            game.native_player_move_packet(PlayerMovePacketCall {
                player,
                position: Vec3::new(x, y, z),
                yaw: report.yaw,
                pitch: report.pitch,
                on_ground: report.on_ground,
                has_position: report.has_position,
                has_rotation: report.has_rotation,
            });
        });
    }

    /// The hub holds guest `conn`'s movement report: the mods hear it with the player's state
    /// swapped in and nothing of the hub borrowed, then the hub commits it or drops it and
    /// corrects the client. A report the hub no longer holds (the player left or was teleported
    /// first) is not heard.
    pub(in crate::game) fn lan_player_move(&mut self, conn: u32) {
        let Some(report) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.pending_move_report(conn))
        else {
            return;
        };
        let mut cancel = false;
        self.as_guest(conn, |game| {
            game.native_player_move(
                report.player,
                report.from,
                report.from_rotation,
                report.to,
                report.to_rotation,
                &mut cancel,
            );
        });
        if let Some(movement) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.resolve_move(conn, !cancel))
        {
            self.guest_accepted_move(conn, movement);
        }
    }
}
