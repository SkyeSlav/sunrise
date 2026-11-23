//! The `crop_grow` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A crop-like block at `pos` is about to grow on a random tick
    /// (NeoForge's `CropGrowEvent.Pre`): crops, stems, nether wart, cocoa,
    /// sweet berries and bamboo at their own position; cactus and chorus
    /// flowers at the block above; sugar cane at its own position; kelp,
    /// weeping and twisting vines and cave vines at the block they grow
    /// into. `state` is the growing block. Read-only `vanilla` is vanilla's own
    /// decision for this tick: its chance roll, or true for cactus, sugar cane
    /// and chorus flowers, which have none. `Default` yields `vanilla`; set
    /// `True` to grow now or `False` to stop it (`DO_NOT_GROW`). NeoForge does
    /// not expose this default decision on its event.
    ///
    /// The roll is drawn before the hook either way, in vanilla's order,
    /// including sweet berries and bamboo, not NeoForge's moved draw order.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "CropGrowEvent.Pre",
        version = 2,
        target = "sunrise_client::game::native_hooks::crop_grow::GameLevel<'_>::native_crop_grow_pre"
    )]
    fn crop_grow_pre();
    /// A crop-like block grew (NeoForge's `CropGrowEvent.Post`).
    /// `original_state` is the block before it grew and `state` what is at
    /// `pos` now. Sugar cane reports the new cane above it, and kelp and
    /// vines the new head, as both states. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "CropGrowEvent.Post",
        target = "sunrise_client::game::native_hooks::crop_grow::GameLevel<'_>::native_crop_grow_post"
    )]
    fn crop_grow_post();
}
