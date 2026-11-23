//! Density function types a mod registers (vanilla's `DENSITY_FUNCTION_TYPE` registry),
//! handed over once at startup and looked up when a density node's `type` names one.
//! Like a mod's biome source, the decoded functions run on the worldgen worker threads,
//! so they keep no mutable state and have no `Host`.
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use sunrise_core::density::{DensitySampler, Interval};

/// One decoded node: the sampler bound into the graph, the range it declared and whether
/// it depends on Y.
pub struct ModDensityLeaf {
    pub sampler: Arc<dyn DensitySampler>,
    pub range: Interval,
    pub varies_with_y: bool,
}

/// A density function type's engine side, as the executable's adapter sees it.
pub trait ModDensityType: std::fmt::Debug + Send + Sync {
    /// Decode a node's whole JSON object for a world with `seed`; `Err` names why.
    fn decode(&self, json: &str, seed: i64) -> Result<ModDensityLeaf, String>;
}

/// The installed types, `modid:ridges` → type. Set once, before any world opens.
pub type ModDensityTypes = BTreeMap<String, Arc<dyn ModDensityType>>;

static MOD_DENSITY_TYPES: OnceLock<Arc<ModDensityTypes>> = OnceLock::new();

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_density_types(types: ModDensityTypes) -> Result<(), Arc<ModDensityTypes>> {
    MOD_DENSITY_TYPES.set(Arc::new(types))
}

/// The installed types: empty until the executable installs the mods'.
pub fn mod_density_types() -> Arc<ModDensityTypes> {
    MOD_DENSITY_TYPES.get().cloned().unwrap_or_default()
}
