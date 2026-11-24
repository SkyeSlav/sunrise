//! The server-side mob target change hook.

pub trait Hooks {
    /// A mob is about to change its target. For `Mob.setTarget` (`is_brain ==
    /// false`), invalid live targets normalize through `Mob.asValidTarget` to
    /// `None` before this hook fires, so `original_target` is the normalized
    /// candidate (`None` when clearing or when `asValidTarget` rejected it).
    /// `new_target` starts as that value; a listener can redirect or clear it.
    /// Set `cancel` to retain the mob's existing target. `is_brain` identifies
    /// a brain-driven target change; false identifies a goal or other mob target
    /// setter. Handles may be read through the host. During a lifted mob tick,
    /// the scoped view reads the ticking mob and peer mobs from the tick's live
    /// disjoint loan, and players, the dragon and non-mob entities directly from
    /// the owning game host. Removed mobs do not resolve; dead-but-present mobs
    /// are distinct from removed mobs.
    /// A stale, fake-player, cross-dimension or unbindable replacement is refused.
    /// Authoritative server only. When piglins turn on a player who took guarded
    /// gold, the game reads the end of its mob list again at each step and finds each
    /// piglin again by its entity id after the hook; a piglin that is removed is
    /// skipped.
    #[hook(
        version = 2,
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        target = "sunrise_client::game::native_hooks::living_change_target::NativeHookHost<'_>::native_living_change_target",
        neoforge = "LivingChangeTargetEvent",
        cancellable = true
    )]
    fn living_change_target(
        mob: MobRef,
        original_target: Option<LivingRef>,
        new_target: &mut Option<LivingRef>,
        is_brain: bool,
        cancel: &mut bool,
    );
}
