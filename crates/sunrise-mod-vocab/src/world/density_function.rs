//! Density function types a native mod registers: vanilla's
//! `BuiltInRegistries.DENSITY_FUNCTION_TYPE` entries (NeoForge registers them through
//! `RegisterEvent` for `Registries.DENSITY_FUNCTION_TYPE`), as
//! [`register_density_function_type`](crate::Registrar::register_density_function_type) hands
//! one over once at startup. A data pack's density function, noise router or noise settings
//! then names it as `{"type": "modid:ridges", ...}`, a named entry or an inline node alike.
//!
//! There is no `Host` here: density functions are decoded while a world's generator data
//! loads and sampled on the worldgen worker threads, two at once; a function keeps no
//! mutable state.

use std::sync::Arc;

/// A mod's density function type: decodes the JSON of each node that names it into a
/// [`DensityFunction`] (vanilla's `MapCodec` in `DensityFunctions.DIRECT_CODEC`).
pub trait DensityFunctionType: Send + Sync + 'static {
    /// Decode one node: `json` is its whole object, `type` included. `Err` refuses the
    /// world's generator data to load, with the message and the type's id; no world opens
    /// with it.
    fn decode(&self, json: &str) -> Result<Box<dyn DensityFunction>, String>;
}

/// A decoded density function (vanilla's `DensityFunction`): a leaf of the world's
/// density graph, sampled in `f32` as the graph is (`DensitySampler.sampleValue`).
/// Deterministic: the same position and seed give the same value, from
/// [`sample`](Self::sample) and [`fill`](Self::fill) alike.
pub trait DensityFunction: Send + Sync + 'static {
    /// `DensityFunction.minValue` and `maxValue`: every value the function returns lies in
    /// `[min, max]`. Read once, when the node is decoded; ordered and not NaN, or the
    /// generator data refuses to load. A value outside it while sampling stops the game
    /// with a crash report naming the mod and the type.
    fn range(&self) -> [f32; 2];
    /// Whether the value depends on Y. `false` lets the graph sample one Y (0) per X/Z
    /// column and reuse it (`DfRewriteRule.SliceUniformAxes`); then `sample` and `fill`
    /// are only asked at Y = 0. Read once, when the node is decoded.
    fn varies_with_y(&self) -> bool;
    /// The value at block position `pos` of a world with `seed`.
    fn sample(&self, pos: [i32; 3], seed: i64) -> f32;
    /// The values on a lattice: `count[axis]` points from `origin`, `step[axis]` blocks
    /// apart, written to `out` (exactly `count[0] * count[1] * count[2]` long) with Z
    /// outermost, then X, then Y innermost, as the graph's volumes are. Every slot must be
    /// written. The default asks [`sample`](Self::sample) at each point; override it when
    /// a column or slab is cheaper together.
    fn fill(
        &self,
        origin: [i32; 3],
        step: [i32; 3],
        count: [usize; 3],
        seed: i64,
        out: &mut [f32],
    ) {
        let mut index = 0;
        for z in 0..count[2] {
            for x in 0..count[0] {
                for y in 0..count[1] {
                    let at = |axis: usize, i: usize| {
                        origin[axis].wrapping_add(step[axis].wrapping_mul(i as i32))
                    };
                    out[index] = self.sample([at(0, x), at(1, y), at(2, z)], seed);
                    index += 1;
                }
            }
        }
    }
}

/// A registered density function type, with the mod that registered it.
pub(crate) struct DensityFunctionTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) mod_id: &'static str,
    pub(crate) ty: Arc<dyn DensityFunctionType>,
}

/// Registered density function types for the hidden game integration boundary: each id,
/// its mod and the type.
// hidden: the game's startup adapter reads the registered types; mods declare them through the registrar.
#[doc(hidden)]
pub fn density_function_type_declarations()
-> Vec<(String, &'static str, Arc<dyn DensityFunctionType>)> {
    crate::parts::installed_density_function_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.mod_id, info.ty.clone()))
        .collect()
}
