//! Native density function adapter: the mods' `DensityFunctionType`s as the engine's.
//! Each call into a mod's code runs attributed to it, so a crash report names the mod; a
//! value outside the range the function declared, NaN or a slot `fill` left unwritten
//! breaks the contract and stops the game there, naming the type.
use mod_api::content::crash::{ModCall, enter_native_mod};
use mod_api::{DensityFunction, DensityFunctionType};
use std::sync::Arc;
use sunrise_core::density::{DensitySampler, Interval, SampleError, SamplerContext};
use sunrise_core::noise::{DensityError, DensityVolume};
use sunrise_world::worldgen::mod_density_types::{
    ModDensityLeaf, ModDensityType, ModDensityTypes, install_mod_density_types,
};

/// Who a call into a type's code belongs to: the mod and the type's id.
#[derive(Clone, Debug)]
struct Owner {
    mod_id: &'static str,
    id: Arc<str>,
}
impl Owner {
    fn call(&self, hook: &'static str) -> ModCall {
        ModCall {
            id: self.mod_id,
            hook,
            part: None,
        }
    }
}

/// A mod's density function type.
struct TypeAdapter {
    owner: Owner,
    ty: Arc<dyn DensityFunctionType>,
}
impl std::fmt::Debug for TypeAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TypeAdapter").field(&self.owner).finish()
    }
}
impl ModDensityType for TypeAdapter {
    fn decode(&self, json: &str, seed: i64) -> Result<ModDensityLeaf, String> {
        let _call = enter_native_mod(self.owner.call("density_function_decode"));
        let function = self.ty.decode(json)?;
        let [min, max] = function.range();
        let range = Interval::new(min, max)
            .map_err(|_| format!("declared range [{min}, {max}] is not ordered"))?;
        let varies_with_y = function.varies_with_y();
        Ok(ModDensityLeaf {
            sampler: Arc::new(Leaf {
                owner: self.owner.clone(),
                function: Arc::from(function),
                range: [min, max],
                seed,
            }),
            range,
            varies_with_y,
        })
    }
}

/// One decoded node, sampled with its world's seed.
struct Leaf {
    owner: Owner,
    function: Arc<dyn DensityFunction>,
    range: [f32; 2],
    seed: i64,
}
impl std::fmt::Debug for Leaf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Leaf")
            .field("owner", &self.owner)
            .field("range", &self.range)
            .finish()
    }
}
impl Leaf {
    /// The contract every value keeps: inside the declared range, which NaN never is.
    fn check(&self, value: f32, at: impl std::fmt::Display) {
        let [min, max] = self.range;
        if !(min..=max).contains(&value) {
            panic!(
                "density function type {} of mod {} gave {value} {at}, outside its declared \
                 range [{min}, {max}] (NaN is also an unwritten fill slot)",
                self.owner.id, self.owner.mod_id
            );
        }
    }
}
impl DensitySampler for Leaf {
    fn pointwise_inputs(&self) -> Option<Vec<&Arc<dyn DensitySampler>>> {
        Some(Vec::new())
    }
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        let _call = enter_native_mod(self.owner.call("density_function_sample"));
        let value = self.function.sample(position, self.seed);
        self.check(value, format_args!("at {position:?}"));
        Ok(value)
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        let out = buffer
            .get_mut(..volume.size())
            .ok_or(DensityError::BufferTooShort)?;
        // A slot the function leaves unwritten stays NaN, which the check refuses.
        out.fill(f32::NAN);
        let _call = enter_native_mod(self.owner.call("density_function_fill"));
        self.function.fill(
            volume.minimum(),
            volume.steps(),
            volume.sizes(),
            self.seed,
            out,
        );
        for (index, &value) in out.iter().enumerate() {
            self.check(value, format_args!("in fill slot {index}"));
        }
        Ok(())
    }
}

fn installed_density_types() -> ModDensityTypes {
    mod_api::__private::density_function_type_declarations()
        .into_iter()
        .map(|(id, mod_id, ty)| {
            let owner = Owner {
                mod_id,
                id: id.as_str().into(),
            };
            (
                id,
                Arc::new(TypeAdapter { owner, ty }) as Arc<dyn ModDensityType>,
            )
        })
        .collect()
}

/// Hand the density function types the mods registered to the engine, before any world opens.
pub(in crate::game) fn install() {
    if install_mod_density_types(installed_density_types()).is_err() {
        log::warn!("Mod density function types already installed; keeping the first table");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `x + y + z`, ranged `[-100, 100]`; a `fill` that skips its last slot when the node
    /// asks for it.
    struct Sum {
        skip_last: bool,
    }
    impl DensityFunction for Sum {
        fn range(&self) -> [f32; 2] {
            [-100.0, 100.0]
        }
        fn varies_with_y(&self) -> bool {
            true
        }
        fn sample(&self, [x, y, z]: [i32; 3], _: i64) -> f32 {
            (x + y + z) as f32
        }
        fn fill(&self, origin: [i32; 3], _: [i32; 3], _: [usize; 3], _: i64, out: &mut [f32]) {
            let len = out.len() - usize::from(self.skip_last);
            out[..len].fill(self.sample(origin, 0));
        }
    }
    struct SumType;
    impl DensityFunctionType for SumType {
        fn decode(&self, json: &str) -> Result<Box<dyn DensityFunction>, String> {
            Ok(Box::new(Sum {
                skip_last: json.contains("skip_last"),
            }))
        }
    }
    struct Inverted;
    impl DensityFunction for Inverted {
        fn range(&self) -> [f32; 2] {
            [1.0, -1.0]
        }
        fn varies_with_y(&self) -> bool {
            false
        }
        fn sample(&self, _: [i32; 3], _: i64) -> f32 {
            0.0
        }
    }
    struct InvertedType;
    impl DensityFunctionType for InvertedType {
        fn decode(&self, _: &str) -> Result<Box<dyn DensityFunction>, String> {
            Ok(Box::new(Inverted))
        }
    }
    fn adapter(ty: impl DensityFunctionType) -> TypeAdapter {
        TypeAdapter {
            owner: Owner {
                mod_id: "test",
                id: "test:sum".into(),
            },
            ty: Arc::new(ty),
        }
    }
    fn volume() -> DensityVolume {
        DensityVolume::new([1, 2, 1], [0, 0, 0], [1, 1, 1]).unwrap()
    }

    /// The declared range reaches the graph and values inside it pass through.
    #[test]
    fn a_decoded_node_keeps_its_declared_range_and_values() {
        let leaf = adapter(SumType).decode("{}", 0).unwrap();
        assert_eq!((leaf.range.min(), leaf.range.max()), (-100.0, 100.0));
        assert!(leaf.varies_with_y);
        let mut context = SamplerContext::default();
        assert_eq!(
            leaf.sampler.sample_value(&mut context, [1, 2, 3]).unwrap(),
            6.0
        );
        let mut values = [0.0; 2];
        leaf.sampler
            .sample_volume(&mut context, &mut values, volume())
            .unwrap();
        assert_eq!(values, [0.0, 0.0]);
    }

    /// A range that isn't ordered refuses the node, so the world never opens with it.
    #[test]
    fn an_inverted_declared_range_refuses_the_node() {
        let Err(error) = adapter(InvertedType).decode("{}", 0) else {
            panic!("an inverted range decoded");
        };
        assert!(error.contains("[1, -1]"), "{error}");
    }

    /// A value outside the declared range breaks the contract and stops the game.
    #[test]
    #[should_panic(expected = "outside its declared range")]
    fn a_value_outside_the_declared_range_stops_the_game() {
        let leaf = adapter(SumType).decode("{}", 0).unwrap();
        let _ = leaf
            .sampler
            .sample_value(&mut SamplerContext::default(), [100, 1, 0]);
    }

    /// A slot `fill` leaves unwritten breaks the contract too.
    #[test]
    #[should_panic(expected = "in fill slot 1")]
    fn an_unwritten_fill_slot_stops_the_game() {
        let leaf = adapter(SumType).decode("skip_last", 0).unwrap();
        let mut values = [0.0; 2];
        let _ = leaf
            .sampler
            .sample_volume(&mut SamplerContext::default(), &mut values, volume());
    }
}
