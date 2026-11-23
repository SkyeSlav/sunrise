use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Player `player` (their UUID) would move from `from` to `to` (or turn from
    /// `from_rotation` to `to_rotation`); on the server, set `cancel` to veto the report before it
    /// is committed. On a client, the segment its own player is about to report: `cancel` predicts
    /// the server's veto: the player goes back to `from` and the report is not sent.
    pub(in crate::game) fn native_player_move(
        &mut self,
        player: u128,
        from: [f64; 3],
        from_rotation: [f32; 2],
        to: [f64; 3],
        to_rotation: [f32; 2],
        cancel: &mut bool,
    ) {
        self.fire_player_move(player, from, from_rotation, to, to_rotation, cancel);
    }
}
