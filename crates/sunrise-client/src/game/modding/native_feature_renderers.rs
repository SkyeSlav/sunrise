//! Registered custom feature renderers (`Hooks::register_feature_renderers`,
//! NeoForge 26.3's `RegisterFeatureRenderersEvent` and `RenderPhaseKeys`, #1162).
//!
//! Collected once at client startup, before resources load. Each registered renderer keeps its own
//! [`CallbackOwner`] so a panic isolates and disables only that renderer for the rest of the
//! session without affecting other feature renderers or the game.

use mod_api::{
    __private::game::{CallbackOwner, contain_unwind},
    ClientHost, FeatureRendererTable, RenderPhaseKey, WorldDrawContext, WorldRenderView,
};
use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

/// Process-wide table of registered feature renderers and their session callback owners.
pub(in crate::game) struct NativeFeatureRendererTable {
    table: FeatureRendererTable,
    owners: HashMap<&'static str, CallbackOwner>,
}

impl NativeFeatureRendererTable {
    /// Borrow the underlying [`FeatureRendererTable`] for submission validation.
    pub(in crate::game) fn inner(&self) -> &FeatureRendererTable {
        &self.table
    }
}

static TABLE: OnceLock<Arc<NativeFeatureRendererTable>> = OnceLock::new();

/// Publish the client startup feature renderer table once.
pub(in crate::game) fn register(table: FeatureRendererTable) {
    let owners = table
        .iter()
        .filter_map(|(id, renderer)| {
            renderer
                .owner()
                .map(|mod_id| (id, CallbackOwner::for_mod(mod_id)))
        })
        .collect();
    let _ = TABLE.set(Arc::new(NativeFeatureRendererTable { table, owners }));
}

/// Borrow the startup-registered feature renderer table, if any was registered.
pub(in crate::game) fn table() -> Option<&'static NativeFeatureRendererTable> {
    TABLE.get().map(Arc::as_ref)
}

/// Invoke the registered feature renderer for `id` with panic isolation.
pub(in crate::game) fn invoke(
    native_table: &NativeFeatureRendererTable,
    id: &'static str,
    host: &dyn ClientHost,
    view: &WorldRenderView<'_>,
    phase: RenderPhaseKey,
    nodes: &(dyn Any + Send + Sync),
    draw: &mut dyn WorldDrawContext,
) {
    let Some(renderer) = native_table.table.get(id) else {
        return;
    };
    let run = || renderer.render(host, view, phase, nodes, draw);
    match native_table.owners.get(id) {
        Some(owner) => {
            if owner.is_enabled() {
                let _ = owner.call("feature renderer", id, run);
            }
        }
        None => {
            let _ = contain_unwind(run);
        }
    }
}
