//! The player's motion for mods (docs/modding/src/how-to/native-mods.md, "A player's motion"): a push for the local player, on a
//! client.
use super::Game;

impl Game<'_> {
    /// `Entity.push(double, double, double)` on the local player, for a client mod: `kick` is added to its
    /// velocity, in blocks a tick, as an explosion's knockback is (the window's player moves itself, as
    /// vanilla's client does). Whatever the player's mode; a value that is not finite is left alone.
    pub(super) fn push_player(&mut self, kick: [f64; 3]) {
        if let Some(p) = self.player.as_mut()
            && kick.iter().all(|v| v.is_finite())
        {
            p.velocity = sunrise_core::geometry::Vec3::new(
                p.velocity.x + kick[0],
                p.velocity.y + kick[1],
                p.velocity.z + kick[2],
            );
            if kick[1] > 0.0 {
                p.ground.on_ground = false;
            }
        }
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;

    #[test]
    fn a_push_adds_to_the_velocity_as_given_in_any_mode() {
        with_game(|game| {
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.player.as_mut().unwrap().spectator = true;
            game.player.as_mut().unwrap().velocity =
                sunrise_core::geometry::Vec3::new(0.1, -0.08, 0.0);
            game.push_player([9.0, -9.0, 0.5]);
            let v = game.player.as_ref().unwrap().velocity;
            assert_eq!([v.x, v.y, v.z], [9.1, -9.08, 0.5]);
            game.push_player([f64::NAN, 0.0, 0.0]);
            assert_eq!(game.player.as_ref().unwrap().velocity.x, 9.1);
        });
    }
}
