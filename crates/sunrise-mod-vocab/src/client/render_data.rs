//! Per-view typed render data keys and in-process storage (`ExtractLevelRenderStateEvent`, #962).

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::LevelRenderStateWrite;

/// Typed key for per-view data stored during
/// [`Hooks::extract_level_render_state`](crate::Hooks::extract_level_render_state) and read from
/// [`WorldRenderView::render_data`](crate::WorldRenderView::render_data) (NeoForge 26.3's
/// `ContextKey<T>`).
///
/// Declare keys as `const` with a namespaced identifier (`"namespace:path"`) and register them
/// during [`Hooks::register`](crate::Hooks::register) via
/// [`Registrar::register_render_data_key`](crate::Registrar::register_render_data_key) so startup
/// refuses any `(id, Rust type)` collision naming both mods and both types.
// no wire: in-process typed key for per-view client render state; never crosses the wire.
// exhaustive: fields are private; constructed through RenderDataKey::new.
pub struct RenderDataKey<T: Send + Sync + 'static> {
    id: &'static str,
    marker: PhantomData<fn() -> T>,
}

impl<T: Send + Sync + 'static> RenderDataKey<T> {
    /// Define a typed render data key for the namespaced identifier `id` (`"namespace:path"`).
    ///
    /// Register this key in [`Hooks::register`](crate::Hooks::register) via
    /// [`Registrar::register_render_data_key`](crate::Registrar::register_render_data_key).
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// The namespaced identifier of this key.
    pub const fn id(&self) -> &'static str {
        self.id
    }
}

impl<T: Send + Sync + 'static> Clone for RenderDataKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Send + Sync + 'static> Copy for RenderDataKey<T> {}

impl<T: Send + Sync + 'static> PartialEq for RenderDataKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: Send + Sync + 'static> Eq for RenderDataKey<T> {}

impl<T: Send + Sync + 'static> Hash for RenderDataKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: Send + Sync + 'static> fmt::Debug for RenderDataKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderDataKey")
            .field("id", &self.id)
            .field("type", &std::any::type_name::<T>())
            .finish()
    }
}

/// Engine-owned per-view typed render data store.
///
/// Cleared before each view's extraction and dropped when the view finishes, so values from
/// frame N never leak into frame N+1 and mod values are dropped before any library unloads.
/// Lookups check `TypeId` through [`Any::downcast_ref`]; a type mismatch returns `None`.
// hidden: engine-owned per-view store; mods use `LevelRenderStateWrite` and `WorldRenderView::render_data`.
// no wire: process-local typed render state for one frame view.
// exhaustive: fields are private; only the engine and tests construct it.
#[doc(hidden)]
#[derive(Default)]
pub struct LevelRenderState {
    entries: HashMap<&'static str, Box<dyn Any + Send + Sync>>,
}

impl fmt::Debug for LevelRenderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LevelRenderState")
            .field("len", &self.entries.len())
            .finish()
    }
}

impl LevelRenderState {
    /// Empty per-view store; performs no heap allocation until a value is written.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all stored values, dropping every mod-owned value immediately.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of entries currently stored for this view.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no entries are currently stored for this view.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Store a boxed `Any` value under `id`.
    pub fn set_raw(&mut self, id: &'static str, value: Box<dyn Any + Send + Sync>) {
        self.entries.insert(id, value);
    }

    /// Borrow a stored `Any` value under `id`.
    pub fn get_raw(&self, id: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.entries.get(id).map(|value| &**value)
    }

    /// Store a typed value under `key`.
    pub fn set<T: Send + Sync + 'static>(&mut self, key: &RenderDataKey<T>, value: T) {
        self.set_raw(key.id(), Box::new(value));
    }

    /// Borrow a typed value stored under `key`, returning `None` on absence or `TypeId` mismatch.
    pub fn get<T: Send + Sync + 'static>(&self, key: &RenderDataKey<T>) -> Option<&T> {
        self.get_raw(key.id())?.downcast_ref::<T>()
    }
}

impl LevelRenderStateWrite for LevelRenderState {
    fn set_render_data_raw(&mut self, id: &'static str, value: Box<dyn Any + Send + Sync>) {
        self.set_raw(id, value);
    }

    fn get_render_data_raw(&self, id: &'static str) -> Option<&(dyn Any + Send + Sync)> {
        self.get_raw(id)
    }
}

impl dyn LevelRenderStateWrite + '_ {
    /// Store a typed value for the current view under `key`.
    ///
    /// Replaces any value stored earlier under the same `key` during this view's extraction.
    /// Listeners run in dependency order, so later listeners see committed writes via
    /// [`Self::get_render_data`], and subsequent world render callbacks for the same view read
    /// them via [`WorldRenderView::render_data`](crate::WorldRenderView::render_data). Values are
    /// cleared before the next view's extraction and dropped when the view finishes.
    pub fn set_render_data<T: Send + Sync + 'static>(&mut self, key: &RenderDataKey<T>, value: T) {
        self.set_render_data_raw(key.id(), Box::new(value));
    }

    /// Borrow a typed value stored earlier in the current view's extraction under `key`.
    ///
    /// Returns `None` if no value was stored under `key` or if the stored value's runtime
    /// [`TypeId`](std::any::TypeId) does not match `T`.
    pub fn get_render_data<T: Send + Sync + 'static>(&self, key: &RenderDataKey<T>) -> Option<&T> {
        self.get_render_data_raw(key.id())?.downcast_ref::<T>()
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const COUNT_KEY: RenderDataKey<u32> = RenderDataKey::new("test:count");
    const WRONG_TYPE_KEY: RenderDataKey<String> = RenderDataKey::new("test:count");
    const LABEL_KEY: RenderDataKey<String> = RenderDataKey::new("test:label");

    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    const DROP_KEY: RenderDataKey<DropFlag> = RenderDataKey::new("test:drop_flag");

    fn test_camera() -> Camera {
        Camera {
            position: [1.0, 2.0, 3.0],
            eye_height: 1.62,
            partial_tick: 0.25,
            yaw: 15.0,
            pitch: -10.0,
            roll: 0.0,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: [0.0; 16],
            projection: [0.0; 16],
            dimension: "minecraft:overworld".into(),
        }
    }

    #[test]
    fn render_data_key_is_not_wire() {
        trait Local<Marker> {
            fn assert_local() {}
        }
        impl<T> Local<()> for T {}
        impl<T: crate::wire::Wire> Local<u8> for T {}

        let _ = <RenderDataKey<u32> as Local<_>>::assert_local;
    }

    #[test]
    fn store_enforces_type_isolation_cross_frame_clearing_and_drop() {
        let mut state = LevelRenderState::new();
        assert!(state.is_empty());

        let writer: &mut dyn LevelRenderStateWrite = &mut state;
        writer.set_render_data(&COUNT_KEY, 7u32);
        assert_eq!(writer.get_render_data(&COUNT_KEY), Some(&7));
        // Same id with a different Rust type must return None (Any::downcast_ref).
        assert_eq!(writer.get_render_data(&WRONG_TYPE_KEY), None);

        let dropped = Arc::new(AtomicBool::new(false));
        writer.set_render_data(&DROP_KEY, DropFlag(Arc::clone(&dropped)));
        assert!(!dropped.load(Ordering::SeqCst));

        let camera = test_camera();
        let view = WorldRenderView::new(&camera, 0.5, 10, false, &[]).__with_render_data(&state);
        assert_eq!(view.render_data(&COUNT_KEY), Some(&7));
        assert_eq!(view.render_data(&WRONG_TYPE_KEY), None);
        assert_eq!(view.render_data(&LABEL_KEY), None);

        // Clearing for the next frame drops stored values and leaves no stale entries.
        state.clear();
        assert!(state.is_empty());
        assert!(dropped.load(Ordering::SeqCst));
        let next_view =
            WorldRenderView::new(&camera, 0.5, 11, false, &[]).__with_render_data(&state);
        assert_eq!(next_view.render_data(&COUNT_KEY), None);
    }

    #[test]
    fn two_views_stay_isolated() {
        let mut state_a = LevelRenderState::new();
        let mut state_b = LevelRenderState::new();
        state_a.set(&COUNT_KEY, 11);
        state_b.set(&COUNT_KEY, 22);

        let camera = test_camera();
        let view_a = WorldRenderView::new(&camera, 0.0, 1, false, &[]).__with_render_data(&state_a);
        let view_b = WorldRenderView::new(&camera, 0.0, 1, false, &[]).__with_render_data(&state_b);
        assert_eq!(view_a.render_data(&COUNT_KEY), Some(&11));
        assert_eq!(view_b.render_data(&COUNT_KEY), Some(&22));
    }

    struct Producer {
        value: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Producer {
        fn extract_level_render_state(
            &self,
            _: &dyn ClientHost,
            event: &mut ExtractLevelRenderStateEvent<'_>,
        ) {
            let v = self.value.load(Ordering::SeqCst) as u32;
            event.state.set_render_data(&COUNT_KEY, v);
        }
    }

    struct Consumer {
        seen_in_extract: Arc<AtomicUsize>,
        seen_in_stage: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Consumer {
        fn extract_level_render_state(
            &self,
            _: &dyn ClientHost,
            event: &mut ExtractLevelRenderStateEvent<'_>,
        ) {
            if let Some(&v) = event.state.get_render_data(&COUNT_KEY) {
                self.seen_in_extract.store(v as usize, Ordering::SeqCst);
                event
                    .state
                    .set_render_data(&LABEL_KEY, format!("frame-{v}"));
            }
        }

        fn render_level_stage(&self, _: &dyn ClientHost, event: &mut RenderLevelStageEvent<'_>) {
            let v = event.view.render_data(&COUNT_KEY).copied().unwrap_or(0);
            self.seen_in_stage.store(v as usize, Ordering::SeqCst);
        }
    }

    struct DummyHost;
    impl Host for DummyHost {}
    mod_api::impl_host_areas!(DummyHost);
    impl ClientHost for DummyHost {}
    mod_api::impl_client_host_areas!(DummyHost);
    impl RenderLevelStageContext for DummyHost {}

    #[test]
    fn extract_dispatch_runs_in_order_and_isolates_frames() {
        let live = Arc::new(AtomicUsize::new(7));
        let seen_extract = Arc::new(AtomicUsize::new(0));
        let seen_stage = Arc::new(AtomicUsize::new(0));
        install(
            vec![
                ModEntry::new(
                    "producer",
                    Box::new(Producer {
                        value: Arc::clone(&live),
                    }),
                ),
                ModEntry::new(
                    "consumer",
                    Box::new(Consumer {
                        seen_in_extract: Arc::clone(&seen_extract),
                        seen_in_stage: Arc::clone(&seen_stage),
                    }),
                ),
            ]
            .into(),
        );

        let camera = test_camera();
        let mut state = LevelRenderState::new();
        let extract_view = WorldRenderView::new(&camera, 0.5, 1, false, &[]);
        dispatch_extract_level_render_state(&DummyHost, &extract_view, &mut state);
        assert_eq!(seen_extract.load(Ordering::SeqCst), 7);
        assert_eq!(state.get(&LABEL_KEY).map(String::as_str), Some("frame-7"));

        // Mutating the live value after extraction must not change what frame 1 draws.
        live.store(9, Ordering::SeqCst);
        let stage_view =
            WorldRenderView::new(&camera, 0.5, 1, false, &[]).__with_render_data(&state);
        dispatch_render_level_stage(
            &DummyHost,
            RenderLevelStage::AfterOpaqueBlocks,
            &stage_view,
            &mut DummyHost,
        );
        assert_eq!(seen_stage.load(Ordering::SeqCst), 7);

        // Frame 2 clears state before extraction and observes the new live value 9.
        state.clear();
        let extract_view_2 = WorldRenderView::new(&camera, 0.5, 2, false, &[]);
        dispatch_extract_level_render_state(&DummyHost, &extract_view_2, &mut state);
        let stage_view_2 =
            WorldRenderView::new(&camera, 0.5, 2, false, &[]).__with_render_data(&state);
        dispatch_render_level_stage(
            &DummyHost,
            RenderLevelStage::AfterOpaqueBlocks,
            &stage_view_2,
            &mut DummyHost,
        );
        assert_eq!(seen_stage.load(Ordering::SeqCst), 9);

        // Disabling producer leaves COUNT_KEY absent after frame 3 clears.
        assert!(set_mod_enabled("producer", false));
        state.clear();
        dispatch_extract_level_render_state(&DummyHost, &extract_view_2, &mut state);
        assert_eq!(state.get(&COUNT_KEY), None);
        let stage_view_3 =
            WorldRenderView::new(&camera, 0.5, 3, false, &[]).__with_render_data(&state);
        dispatch_render_level_stage(
            &DummyHost,
            RenderLevelStage::AfterOpaqueBlocks,
            &stage_view_3,
            &mut DummyHost,
        );
        assert_eq!(seen_stage.load(Ordering::SeqCst), 0);
    }
}
