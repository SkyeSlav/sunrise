pub use world::custom_registries::{
    CustomRegistry, EntryKey, HostRegistries, RegistryKey, RegistryOptions,
};

mod __private {
    pub use crate::parts::{custom_registry_directories, custom_registry_entry_id};
    pub use crate::world::custom_registries::{
        CustomRegistryFreezer, FrozenCustomRegistries, RegistryDirectory,
    };
}
