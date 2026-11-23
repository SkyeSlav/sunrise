use super::Game;

impl Game<'_> {
    pub(in crate::game) fn native_player_move_packet(
        &mut self,
        call: mod_api::__private::PlayerMovePacketCall,
    ) {
        self.fire_player_move_packet(call);
    }
}
