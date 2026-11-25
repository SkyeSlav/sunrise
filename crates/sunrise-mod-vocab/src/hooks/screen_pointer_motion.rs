//! The `screen_pointer_motion` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Raw pointer motion while a mod screen holds the pointer grab
    /// (`ClientHost::set_pointer_grab`): `delta` is the device's relative movement
    /// (winit's `DeviceEvent::MouseMotion`), in the device's own units, not GUI pixels. It
    /// is not scaled by the mouse sensitivity option, since a screen is not the camera;
    /// the in-world camera and `calculate_player_turn` are untouched. `screen` is the
    /// grabbing screen (`screen.mod_screen` is its mod); its `mouse` stays where the grab
    /// put the cursor. A different moment from `screen_mouse_moved_pre`, which reports the
    /// cursor's movement in GUI pixels and does not fire while the grab holds.
    /// Fires only while that screen holds the grab, for the real pointer: a controller
    /// sends no motion, and a zero delta is not reported. Not cancellable.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 1,
        contract = "listen",
        cancellable = false
    )]
    fn screen_pointer_motion(screen: &ScreenView<'_>, delta: [f64; 2]);
}
