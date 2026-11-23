//! The `entity_size` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's dimensions are being resolved for `pose`: after a pose or vanilla-size change
    /// (NeoForge's `EntityEvent.Size`, fired from `Entity`'s constructor and from
    /// `Entity.refreshDimensions`: its pose changed to `pose`, it grew up, its `minecraft:scale`
    /// changed, or it is new). `size` (width, height and eye height, in
    /// blocks) starts as vanilla's new size and `old_size` is the size it had. Change `size`
    /// and the entity's box and eyes take it; later mods see earlier mods' edits. The answer
    /// holds while the entity keeps that pose and vanilla size; the next change asks again,
    /// starting from vanilla's. Changing the width or height leaves the eye height as it is:
    /// set it too. Zero-size and very large answers are valid (version 3 removes the 32-block
    /// upper limit: `EntityDimensions` has none). An answer with a component that is non-finite
    /// or negative is logged and ignored, keeping vanilla's size. `host`
    /// reads the side that asks; the entity may be new, so read only what you need of it.
    ///
    /// A mob is asked on the server before and after its tick. A player is asked when its pose or
    /// vanilla size changed: a client's own player after its tick, a LAN guest's player once a
    /// server tick. A size changed within a tick takes the answer at the next of those. The pose
    /// fit tests of `Player.updatePlayerPose` read `Entity.getDimensions(pose)`, which fires no
    /// event, so a candidate pose is tested at vanilla's size. Does not fire for:
    /// other kinds of entity, whose vanilla sizes are fixed, and a client's copies of other
    /// entities, which keep vanilla's size. Not cancellable.
    ///
    /// Version 3: no 32-block limit on an answer, and a player's candidate poses are no longer
    /// asked: only the pose it is in.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 3,
        contract = "collect",
        out = "size",
        host = "read",
        neoforge = "EntityEvent.Size",
        cancellable = false
    )]
    fn entity_size(entity: EntityRef, pose: mod_api::Pose, old_size: [f32; 3], size: &mut [f32; 3]);
}
