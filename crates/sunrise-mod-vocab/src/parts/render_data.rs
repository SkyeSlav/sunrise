//! Startup registration of typed per-view render data keys (`RenderDataKey<T>`).
use std::any::TypeId;

use super::*;
use crate::RenderDataKey;

impl dyn Registrar + '_ {
    /// Register a typed [`RenderDataKey`] at startup through the shared `(namespaced id, Rust type)`
    /// identity table.
    ///
    /// Two mods registering the same namespaced ID with the same Rust type `T` share that key
    /// across [`Hooks::extract_level_render_state`](crate::Hooks::extract_level_render_state) and
    /// [`WorldRenderView::render_data`](crate::WorldRenderView::render_data). Registering the same
    /// ID with two different Rust types, or an ID that is not a valid namespaced identifier,
    /// refuses startup registration naming both mods and both types.
    pub fn register_render_data_key<T: Send + Sync + 'static>(&mut self, key: &RenderDataKey<T>) {
        self.register_render_data_key_raw(key.id(), TypeId::of::<T>(), type_name::<T>());
    }
}

/// Registrar operations for per-view render data keys.
pub trait RenderDataRegistrar {
    /// Register a namespaced render data key with its Rust type identity.
    // hidden: object-safe backing method for `dyn Registrar::register_render_data_key`.
    #[doc(hidden)]
    fn register_render_data_key_raw(
        &mut self,
        id: &'static str,
        type_id: TypeId,
        type_name: &'static str,
    );
}
