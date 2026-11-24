//! Registration and lookup plumbing for mod-declared static custom registries.
use super::*;
use crate::world::custom_registries::{
    CustomRegistry, CustomRegistryFreezer, EntryKey, RegistryDirectory, RegistryKey,
    RegistryOptions, freeze_typed,
};
use std::any::TypeId;

/// Object-safe low-level registrar trait for static custom registries.
/// Mods call [`Registrar::declare_registry`] and [`Registrar::register_entry`] instead.
// hidden: typed helpers live on `dyn Registrar`; this object-safe layer is internal plumbing.
#[doc(hidden)]
pub trait CustomRegistriesRegistrar {
    /// Declare a static custom registry with its type identity and boxed typed freezer.
    fn declare_custom_registry_erased(
        &mut self,
        id: &'static str,
        type_id: TypeId,
        type_name: &'static str,
        options: RegistryOptions,
        freeze: Box<dyn Any + Send + Sync>,
    );

    /// Register one boxed entry into a declared static custom registry.
    fn register_custom_entry_erased(
        &mut self,
        registry_id: &'static str,
        type_id: TypeId,
        type_name: &'static str,
        entry_id: &str,
        value: Box<dyn Any + Send + Sync>,
    ) -> (u32, u32);
}

impl dyn Registrar + '_ {
    /// Declare a new typed static custom registry owned by the calling mod
    /// (NeoForge's `NewRegistryEvent` / `RegistryBuilder`).
    ///
    /// The registry ID must be a valid namespaced identifier in a namespace owned by the
    /// calling mod, never `minecraft:`, and must not collide with any built-in or
    /// mod-declared registry. A `side = "server"` package may not declare `sync(true)`.
    pub fn declare_registry<T: Send + Sync + 'static>(
        &mut self,
        key: RegistryKey<T>,
        options: RegistryOptions,
    ) {
        self.declare_custom_registry_erased(
            key.id(),
            TypeId::of::<T>(),
            type_name::<T>(),
            options,
            Box::new(freeze_typed::<T> as CustomRegistryFreezer),
        );
    }

    /// Register a typed entry into a previously declared static custom registry
    /// (called in dependency order during [`Hooks::register`](crate::Hooks::register)).
    ///
    /// Returns a dense [`EntryKey<T>`] for O(1) runtime lookups on [`CustomRegistry<T>`].
    /// If registration is refused (undeclared registry, type mismatch, foreign or duplicate
    /// entry ID, or a `side = "server"` package registering into a synced registry), startup
    /// fails naming the mod and refusal reason.
    pub fn register_entry<T: Send + Sync + 'static>(
        &mut self,
        key: RegistryKey<T>,
        id: &str,
        value: T,
    ) -> EntryKey<T> {
        let (registry_slot, entry_slot) = self.register_custom_entry_erased(
            key.id(),
            TypeId::of::<T>(),
            type_name::<T>(),
            id,
            Box::new(value),
        );
        EntryKey::new(registry_slot, entry_slot)
    }
}

/// Look up one frozen typed static custom registry in the current installed epoch
/// (`TEST_PARTS` first, then the process-wide `INSTALLED`).
pub(crate) fn custom_registry<T: Send + Sync + 'static>(
    id: &str,
) -> Option<&'static CustomRegistry<T>> {
    installed()?.custom_registries.get::<T>(id)
}

/// All frozen static custom registry directories in the current installed epoch
/// (`TEST_PARTS` first, then `INSTALLED`).
pub fn custom_registry_directories() -> &'static [Arc<dyn RegistryDirectory>] {
    installed().map_or(&[], |installed| installed.custom_registries.directories())
}

/// Look up an entry's canonical identifier by registry ID and entry slot in the current
/// installed epoch (`TEST_PARTS` first, then `INSTALLED`).
pub fn custom_registry_entry_id(
    registry_id: &str,
    entry_slot: u32,
) -> Option<&'static crate::Identifier> {
    installed()?
        .custom_registries
        .entry_id(registry_id, entry_slot)
}
