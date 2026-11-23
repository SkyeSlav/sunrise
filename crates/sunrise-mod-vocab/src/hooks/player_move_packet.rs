//! The `player_move_packet` hook declaration (see `hooks/mod.rs`).

pub trait Hooks {
    /// Observes one validated player movement packet after the server handles it and
    /// before it handles the player's next packet. The server host acts as `player`.
    /// Position-only, rotation-only, combined and status-only packets are heard,
    /// even unchanged or on the first tick in a level. A passenger's accepted look
    /// is heard; vehicle packets, teleport acknowledgements, reports handled while
    /// awaiting a teleport and synthetic moves are not.
    ///
    /// `position` is the target feet position in world coordinates; `yaw` and `pitch`
    /// are target angles in degrees. Supplied fields are the server's clamped
    /// position and wrapped angles, before application; missing fields retain the
    /// player's actual pre-packet pose. These are not the player's final pose.
    /// `on_ground` is the client's claim, not the server's ground state.
    /// `has_position` and `has_rotation` indicate fields carried, not values changed.
    ///
    /// A valid report vetoed by `player_move` is heard after the veto and correction,
    /// with the same pre-application snapshot. This listener cannot change or veto
    /// the report; each listener gets its own event.
    #[hook(
        target = "sunrise_client::game::native_hooks::player_move_packet::Game<'_>::native_player_move_packet",
        receiver = "write",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        event_per_listener = true,
        neoforge = "none (PlayerTickEvent observes a player tick, not a movement packet)"
    )]
    fn player_move_packet(
        player: mod_api::PlayerRef,
        position: mod_api::Vec3,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
        has_position: bool,
        has_rotation: bool,
    );
}
