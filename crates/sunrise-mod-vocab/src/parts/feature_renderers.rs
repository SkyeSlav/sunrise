//! Startup registration of typed feature renderer keys (`FeatureRendererType<S>`).
use std::any::TypeId;

use super::*;
use crate::FeatureRendererType;

impl dyn Registrar + '_ {
    /// Register a typed [`FeatureRendererType`] at startup through the shared
    /// `(namespaced id, Rust type)` identity table.
    ///
    /// Two mods registering the same namespaced ID with the same Rust type `S` share that feature
    /// renderer type across [`Hooks::register_feature_renderers`](crate::Hooks::register_feature_renderers)
    /// and `WorldDrawContext::submit_feature`.
    /// Registering the same ID with two different Rust types, or an ID that is not a valid
    /// namespaced identifier, refuses startup registration naming both mods and both types.
    pub fn register_feature_renderer_type<S: Send + Sync + 'static>(
        &mut self,
        ty: &FeatureRendererType<S>,
    ) {
        self.register_feature_renderer_type_raw(ty.id(), TypeId::of::<S>(), type_name::<S>());
    }
}

/// Registrar operations for custom feature renderer types.
pub trait FeatureRendererRegistrar {
    /// Register a namespaced feature renderer type with its Rust type identity.
    // hidden: object-safe backing method for `dyn Registrar::register_feature_renderer_type`.
    #[doc(hidden)]
    fn register_feature_renderer_type_raw(
        &mut self,
        id: &'static str,
        type_id: TypeId,
        type_name: &'static str,
    );
}
