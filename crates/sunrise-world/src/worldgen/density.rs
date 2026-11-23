//! Bounded decoding of density registries into the immutable core graph.
//! Named registry holders and raw numeric tokens preserve compilation behavior.
use super::mod_density_types::{ModDensityType, ModDensityTypes};
use super::*;
use std::{collections::BTreeSet, sync::Arc};
use sunrise_core::{
    density::{
        BlendedNoise, CubicSpline, DistanceMetric, RoundMode, ShiftNoiseKind, SplinePoint,
        TilingMode, WorldField,
        graph::{DensityGraph, FunctionId, GraphBinary, GraphNode, GraphUnary, NoiseId},
    },
    geometry::Axis,
};

pub(super) fn identifier(name: &str) -> Result<String, DecodeError> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid)
        || !path.bytes().all(|c| valid(c) || c == b'/')
        || path.is_empty()
    {
        return Err(DecodeError::Registry(format!(
            "invalid registry identifier: {name}"
        )));
    }
    Ok(format!("{namespace}:{path}"))
}
fn string(raw: &RawValue) -> Result<String, DecodeError> {
    Ok(serde_json::from_str(raw.get())?)
}
fn float(raw: &RawValue) -> Result<f32, DecodeError> {
    number_text(raw)?
        .parse()
        .map_err(|_| DecodeError::Field("float"))
}
fn noise_value(raw: &RawValue) -> Result<f32, DecodeError> {
    let value = float(raw)?;
    if (-1e6..=1e6).contains(&value) {
        Ok(value)
    } else {
        Err(DecodeError::Field("density value"))
    }
}
fn axis(raw: &RawValue) -> Result<Axis, DecodeError> {
    match string(raw)?.as_str() {
        "x" => Ok(Axis::X),
        "y" => Ok(Axis::Y),
        "z" => Ok(Axis::Z),
        _ => Err(DecodeError::Field("axis")),
    }
}

/// All entries are loaded atomically. Named child references resolve through
/// distinct registry holders; returned entry values are their direct graphs.
#[derive(Debug)]
pub struct DensityRegistry {
    graph: DensityGraph,
    values: BTreeMap<String, FunctionId>,
    noises: BTreeMap<String, NoiseId>,
    /// The mods' density function types later roots may name, and the world seed their
    /// functions sample with.
    types: Arc<ModDensityTypes>,
    seed: i64,
}
impl DensityRegistry {
    /// Input keys are registry identifiers, and values contain original JSON.
    /// This API accepts acyclic registries, at most 256 nested graph levels and
    /// at most 256 interval branches. Inline noise holders are not yet supported.
    /// Vanilla's density function types only.
    pub fn from_json(
        noises: &BTreeMap<String, String>,
        densities: &BTreeMap<String, String>,
    ) -> Result<Self, DecodeError> {
        Self::from_json_with_types(noises, densities, Arc::default(), 0)
    }
    /// [`Self::from_json`] where a node may also name one of the mods' `types`
    /// (`DensityFunctions.DIRECT_CODEC` over the `DENSITY_FUNCTION_TYPE` registry), decoded
    /// for a world with `seed`.
    pub fn from_json_with_types(
        noises: &BTreeMap<String, String>,
        densities: &BTreeMap<String, String>,
        types: Arc<ModDensityTypes>,
        seed: i64,
    ) -> Result<Self, DecodeError> {
        let mut loader = Loader {
            graph: DensityGraph::new(),
            noises: BTreeMap::new(),
            definitions: BTreeMap::new(),
            values: BTreeMap::new(),
            active: BTreeSet::new(),
            types,
            seed,
        };
        for (name, json) in noises {
            let name = identifier(name)?;
            let noise = loader
                .graph
                .register_noise(name.clone(), decode_noise(json)?)?;
            loader.noises.insert(name, noise);
        }
        for (name, json) in densities {
            let name = identifier(name)?;
            let raw: Box<RawValue> = serde_json::from_str(json)?;
            if loader.definitions.insert(name.clone(), raw).is_some() {
                return Err(DecodeError::Registry(format!(
                    "duplicate density identifier: {name}"
                )));
            }
        }
        let names: Vec<_> = loader.definitions.keys().cloned().collect();
        for name in names {
            loader.resolve(&name, 0)?;
        }
        Ok(Self {
            graph: loader.graph,
            values: loader.values,
            noises: loader.noises,
            types: loader.types,
            seed,
        })
    }
    pub fn graph(&self) -> &DensityGraph {
        &self.graph
    }
    pub fn into_graph(self) -> DensityGraph {
        self.graph
    }
    pub fn entries(&self) -> impl Iterator<Item = (&str, FunctionId)> {
        self.values.iter().map(|(name, id)| (name.as_str(), *id))
    }
    pub fn value(&self, name: &str) -> Option<FunctionId> {
        self.values.get(name).copied()
    }
    pub fn reference(&self, name: &str) -> Option<FunctionId> {
        self.graph.function(name)
    }
    /// Decode direct roots against this registry, retaining shared named holders.
    /// All roots are added atomically; a failure leaves existing IDs and graphs
    /// intact. Roots are not registered as additional named density entries.
    pub fn decode_functions(&mut self, json: &[&str]) -> Result<Vec<FunctionId>, DecodeError> {
        let mut loader = Loader {
            graph: self.graph.clone(),
            noises: self.noises.clone(),
            definitions: BTreeMap::new(),
            values: BTreeMap::new(),
            active: BTreeSet::new(),
            types: self.types.clone(),
            seed: self.seed,
        };
        let roots = json
            .iter()
            .map(|json| {
                let raw: Box<RawValue> = serde_json::from_str(json)?;
                loader.decode(&raw, 0)
            })
            .collect::<Result<_, _>>()?;
        self.graph = loader.graph;
        Ok(roots)
    }
}
struct Loader {
    graph: DensityGraph,
    noises: BTreeMap<String, NoiseId>,
    definitions: BTreeMap<String, Box<RawValue>>,
    values: BTreeMap<String, FunctionId>,
    active: BTreeSet<String>,
    types: Arc<ModDensityTypes>,
    seed: i64,
}
impl Loader {
    fn resolve(&mut self, name: &str, depth: usize) -> Result<FunctionId, DecodeError> {
        if let Some(id) = self.graph.function(name) {
            return Ok(id);
        }
        if depth > 256 {
            return Err(DecodeError::Field("density nesting"));
        }
        if !self.active.insert(name.to_owned()) {
            return Err(DecodeError::Registry(format!(
                "cyclic density reference: {name}"
            )));
        }
        let raw = self
            .definitions
            .get(name)
            .ok_or_else(|| DecodeError::Registry(format!("unknown density reference: {name}")))?
            .clone();
        let value = self.decode(&raw, depth + 1)?;
        let reference = self.graph.register_function(name.to_owned(), value)?;
        self.values.insert(name.to_owned(), value);
        self.active.remove(name);
        Ok(reference)
    }
    fn input(
        &mut self,
        object: &Object,
        name: &'static str,
        depth: usize,
    ) -> Result<FunctionId, DecodeError> {
        self.decode(required(object, name)?, depth + 1)
    }
    fn optional_input(
        &mut self,
        object: &Object,
        name: &'static str,
        fallback: f32,
        depth: usize,
    ) -> Result<FunctionId, DecodeError> {
        match optional(object, name) {
            Some(raw) => self.decode(raw, depth + 1),
            None => Ok(self.graph.insert(GraphNode::Constant(fallback))?),
        }
    }
    fn noise(&self, object: &Object) -> Result<NoiseId, DecodeError> {
        let raw = required(object, "noise")?;
        if !raw.get().starts_with('"') {
            return Err(DecodeError::Field("inline noise holder unsupported"));
        }
        let name = identifier(&string(raw)?)?;
        self.noises
            .get(&name)
            .copied()
            .ok_or_else(|| DecodeError::Registry(format!("unknown noise reference: {name}")))
    }
    fn decode(&mut self, raw: &RawValue, depth: usize) -> Result<FunctionId, DecodeError> {
        if depth > 256 {
            return Err(DecodeError::Field("density nesting"));
        }
        if raw.get().starts_with('"') {
            return self.resolve(&identifier(&string(raw)?)?, depth);
        }
        if !raw.get().starts_with('{') {
            return Ok(self.graph.insert(GraphNode::Constant(noise_value(raw)?))?);
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let kind = identifier(&string(required(&object, "type")?)?)?;
        let unary = match kind.as_str() {
            "minecraft:abs" => Some(GraphUnary::Abs),
            "minecraft:square" => Some(GraphUnary::Square),
            "minecraft:cube" => Some(GraphUnary::Cube),
            "minecraft:sqrt" => Some(GraphUnary::Sqrt),
            "minecraft:half_negative" => Some(GraphUnary::HalfNegative),
            "minecraft:quarter_negative" => Some(GraphUnary::QuarterNegative),
            "minecraft:reciprocal" => Some(GraphUnary::Reciprocal),
            "minecraft:negate" => Some(GraphUnary::Negate),
            "minecraft:squeeze" => Some(GraphUnary::Squeeze),
            "minecraft:log" => Some(GraphUnary::Log),
            "minecraft:sign" => Some(GraphUnary::Sign),
            _ => None,
        };
        let binary = match kind.as_str() {
            "minecraft:add" => Some(GraphBinary::Add),
            "minecraft:sub" => Some(GraphBinary::Subtract),
            "minecraft:mul" => Some(GraphBinary::Multiply),
            "minecraft:div" => Some(GraphBinary::Divide),
            "minecraft:min" => Some(GraphBinary::Minimum),
            "minecraft:max" => Some(GraphBinary::Maximum),
            _ => None,
        };
        let node = if let Some(kind) = unary {
            GraphNode::Unary {
                kind,
                input: self.input(&object, "input", depth)?,
            }
        } else if let Some(kind) = binary {
            GraphNode::Binary {
                kind,
                left: self.input(&object, "left", depth)?,
                right: self.input(&object, "right", depth)?,
            }
        } else {
            match kind.as_str() {
                "minecraft:constant" => {
                    GraphNode::Constant(noise_value(required(&object, "value")?)?)
                }
                "minecraft:blend_alpha" => GraphNode::WorldField(WorldField::BlendAlpha),
                "minecraft:blend_offset" => GraphNode::WorldField(WorldField::BlendOffset),
                "minecraft:beardifier" => GraphNode::WorldField(WorldField::Beardifier),
                "minecraft:end_outer_islands" => GraphNode::EndIslands,
                "minecraft:cache" => GraphNode::Cache(self.input(&object, "input", depth)?),
                "minecraft:blend_density" => {
                    GraphNode::BlendDensity(self.input(&object, "input", depth)?)
                }
                "minecraft:pow" => GraphNode::Power {
                    base: self.input(&object, "base", depth)?,
                    exponent: self.input(&object, "exponent", depth)?,
                },
                "minecraft:floor" | "minecraft:round" | "minecraft:ceil" | "minecraft:truncate" => {
                    GraphNode::Round {
                        mode: match kind.as_str() {
                            "minecraft:floor" => RoundMode::Floor,
                            "minecraft:round" => RoundMode::Round,
                            "minecraft:ceil" => RoundMode::Ceil,
                            _ => RoundMode::Truncate,
                        },
                        input: self.input(&object, "input", depth)?,
                        multiple: self.optional_input(&object, "multiple", 1.0, depth)?,
                    }
                }
                "minecraft:gradient" => {
                    let from = integer(required(&object, "from_coordinate")?)?;
                    let to = integer(required(&object, "to_coordinate")?)?;
                    if from == to {
                        return Err(DecodeError::Field("gradient coordinates"));
                    }
                    GraphNode::Gradient {
                        axis: axis(required(&object, "axis")?)?,
                        tiling: match optional(&object, "tiling")
                            .map(string)
                            .transpose()?
                            .as_deref()
                            .unwrap_or("clamp_to_edge")
                        {
                            "clamp_to_edge" => TilingMode::ClampToEdge,
                            "repeat" => TilingMode::Repeat,
                            "mirrored_repeat" => TilingMode::MirroredRepeat,
                            _ => return Err(DecodeError::Field("tiling")),
                        },
                        from,
                        to,
                        low: noise_value(required(&object, "from_value")?)?,
                        high: noise_value(required(&object, "to_value")?)?,
                    }
                }
                "minecraft:noise" => GraphNode::Noise {
                    noise: self.noise(&object)?,
                    xz_scale: double(required(&object, "xz_scale")?)?,
                    y_scale: double(required(&object, "y_scale")?)?,
                    shift_x: self.optional_input(&object, "shift_x", 0.0, depth)?,
                    shift_y: self.optional_input(&object, "shift_y", 0.0, depth)?,
                    shift_z: self.optional_input(&object, "shift_z", 0.0, depth)?,
                },
                "minecraft:shift" | "minecraft:shift_a" | "minecraft:shift_b" => {
                    GraphNode::OffsetNoise {
                        noise: self.noise(&object)?,
                        kind: match kind.as_str() {
                            "minecraft:shift" => ShiftNoiseKind::Shift,
                            "minecraft:shift_a" => ShiftNoiseKind::ShiftA,
                            _ => ShiftNoiseKind::ShiftB,
                        },
                    }
                }
                "minecraft:old_blended_noise" => GraphNode::BlendedNoise(BlendedNoise {
                    xz_scale: double_range(
                        required(&object, "xz_scale")?,
                        "xz_scale",
                        0.001,
                        1000.0,
                    )?,
                    y_scale: double_range(required(&object, "y_scale")?, "y_scale", 0.001, 1000.0)?,
                    xz_factor: double_range(
                        required(&object, "xz_factor")?,
                        "xz_factor",
                        0.001,
                        1000.0,
                    )?,
                    y_factor: double_range(
                        required(&object, "y_factor")?,
                        "y_factor",
                        0.001,
                        1000.0,
                    )?,
                    smear_scale_multiplier: double_range(
                        required(&object, "smear_scale_multiplier")?,
                        "smear_scale_multiplier",
                        1.0,
                        8.0,
                    )?,
                }),
                "minecraft:slice" => GraphNode::Slice {
                    axis: axis(required(&object, "axis")?)?,
                    coordinate: integer(required(&object, "coordinate")?)?,
                    input: self.input(&object, "input", depth)?,
                },
                "minecraft:interpolated" => GraphNode::Interpolated {
                    input: self.input(&object, "input", depth)?,
                    xz: int_range(
                        required(&object, "cell_size_xz")?,
                        "cell_size_xz",
                        1,
                        i32::MAX,
                    )?,
                    y: int_range(
                        required(&object, "cell_size_y")?,
                        "cell_size_y",
                        1,
                        i32::MAX,
                    )?,
                },
                "minecraft:find_top_surface" => GraphNode::Surface {
                    density: self.input(&object, "density", depth)?,
                    upper: self.input(&object, "upper_bound", depth)?,
                    lower: int_range(
                        required(&object, "lower_bound")?,
                        "lower_bound",
                        -4064,
                        4062,
                    )?,
                    height: int_range(
                        required(&object, "cell_height")?,
                        "cell_height",
                        1,
                        i32::MAX,
                    )?,
                },
                "minecraft:clamp" => {
                    let min = noise_value(required(&object, "min")?)?;
                    let max = noise_value(required(&object, "max")?)?;
                    if max < min {
                        return Err(DecodeError::Field("clamp bounds"));
                    }
                    GraphNode::Clamp {
                        input: self.input(&object, "input", depth)?,
                        min,
                        max,
                    }
                }
                "minecraft:range_choice" => GraphNode::Choice {
                    input: self.input(&object, "input", depth)?,
                    min: noise_value(required(&object, "min_inclusive")?)?,
                    max: noise_value(required(&object, "max_exclusive")?)?,
                    inside: self.input(&object, "when_in_range", depth)?,
                    outside: self.input(&object, "when_out_of_range", depth)?,
                },
                "minecraft:lerp" => GraphNode::Lerp {
                    alpha: self.input(&object, "alpha", depth)?,
                    first: self.input(&object, "first", depth)?,
                    second: self.input(&object, "second", depth)?,
                },
                "minecraft:interval_select" => {
                    let thresholds: Vec<Box<RawValue>> =
                        serde_json::from_str(required(&object, "thresholds")?.get())?;
                    let thresholds: Vec<f32> = thresholds
                        .iter()
                        .map(|raw| noise_value(raw))
                        .collect::<Result<_, _>>()?;
                    let functions: Vec<Box<RawValue>> =
                        serde_json::from_str(required(&object, "functions")?.get())?;
                    if functions.len() < 2
                        || functions.len() > 256
                        || functions.len() != thresholds.len() + 1
                        || thresholds
                            .windows(2)
                            .any(|pair| pair[0].total_cmp(&pair[1]).is_gt())
                    {
                        return Err(DecodeError::Field("interval selection"));
                    }
                    let input = self.input(&object, "input", depth)?;
                    let branches = functions
                        .iter()
                        .map(|raw| self.decode(raw, depth + 1))
                        .collect::<Result<_, _>>()?;
                    GraphNode::IntervalSelect {
                        input,
                        thresholds: Arc::from(thresholds),
                        branches,
                    }
                }
                "minecraft:distance_to_point" => {
                    let raw: Vec<Box<RawValue>> =
                        serde_json::from_str(required(&object, "point")?.get())?;
                    if raw.len() != 3 {
                        return Err(DecodeError::Field("point"));
                    }
                    let point = [integer(&raw[0])?, integer(&raw[1])?, integer(&raw[2])?];
                    let metric = match string(required(&object, "metric")?)?.as_str() {
                        "euclidean" => DistanceMetric::Euclidean,
                        "euclidean_squared" => DistanceMetric::EuclideanSquared,
                        "manhattan" => DistanceMetric::Manhattan,
                        "chebyshev" => DistanceMetric::Chebyshev,
                        _ => return Err(DecodeError::Field("metric")),
                    };
                    GraphNode::DistanceToPoint { point, metric }
                }
                "minecraft:spline" => {
                    GraphNode::Spline(self.spline(required(&object, "spline")?, depth + 1)?)
                }
                _ => {
                    let Some(ty) = self.types.get(&kind).cloned() else {
                        return Err(DecodeError::Registry(format!(
                            "unknown density type: {kind}"
                        )));
                    };
                    return self.mod_leaf(&kind, &*ty, raw);
                }
            }
        };
        Ok(self.graph.insert(node)?)
    }
    /// A mod's node: decoded by its type, then bound with the range it declared, over
    /// X, Y and Z, or X and Z alone when it doesn't vary with Y.
    fn mod_leaf(
        &mut self,
        kind: &str,
        ty: &dyn ModDensityType,
        raw: &RawValue,
    ) -> Result<FunctionId, DecodeError> {
        let leaf = ty
            .decode(raw.get(), self.seed)
            .map_err(|error| DecodeError::Registry(format!("density type {kind}: {error}")))?;
        let axes = if leaf.varies_with_y { 0b111 } else { 0b101 };
        Ok(self.graph.bind(leaf.sampler, leaf.range, axes)?)
    }
    fn spline(
        &mut self,
        raw: &RawValue,
        depth: usize,
    ) -> Result<CubicSpline<FunctionId>, DecodeError> {
        if depth > 256 {
            return Err(DecodeError::Field("spline nesting"));
        }
        if !raw.get().starts_with('{') {
            return Ok(CubicSpline::Constant(float(raw)?));
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let coordinate = self.input(&object, "coordinate", depth)?;
        let points: Vec<Box<RawValue>> = serde_json::from_str(required(&object, "points")?.get())?;
        let points = points
            .iter()
            .map(|raw| {
                let point: Object = serde_json::from_str(raw.get())?;
                Ok(SplinePoint {
                    location: float(required(&point, "location")?)?,
                    derivative: float(required(&point, "derivative")?)?,
                    value: self.spline(required(&point, "value")?, depth + 1)?,
                })
            })
            .collect::<Result<_, DecodeError>>()?;
        CubicSpline::multipoint(coordinate, points)
            .map_err(|error| DecodeError::Graph(error.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::density::{DensitySampler, SamplerContext, graph::DensityCompiler};
    #[test]
    fn numeric_tokens_keep_negative_zero_and_avoid_double_rounding() {
        let densities = BTreeMap::from([
            ("minecraft:negative_zero".into(), "-0".into()),
            (
                "minecraft:rounded".into(),
                "1.0000000596046447753906250000000001".into(),
            ),
        ]);
        let registry = DensityRegistry::from_json(&BTreeMap::new(), &densities).unwrap();
        let zero = registry.value("minecraft:negative_zero").unwrap();
        let rounded = registry.value("minecraft:rounded").unwrap();
        let compiler = DensityCompiler::new(registry.into_graph());
        let mut context = SamplerContext::default();
        assert_eq!(
            compiler
                .compile_raw(zero)
                .unwrap()
                .sample_value(&mut context, [0; 3])
                .unwrap()
                .to_bits(),
            0x8000_0000
        );
        assert_eq!(
            compiler
                .compile_raw(rounded)
                .unwrap()
                .sample_value(&mut context, [0; 3])
                .unwrap()
                .to_bits(),
            0x3f80_0001
        );
    }
    #[test]
    fn forward_references_share_named_holders_and_reject_cycles_or_missing_values() {
        let mut densities = BTreeMap::from([
            (
                "minecraft:a".into(),
                r#"{"type":"add","left":"z","right":"minecraft:z"}"#.into(),
            ),
            ("minecraft:z".into(), "0.5".into()),
        ]);
        let registry = DensityRegistry::from_json(&BTreeMap::new(), &densities).unwrap();
        let root = registry.value("minecraft:a").unwrap();
        let compiler = DensityCompiler::new(registry.into_graph());
        assert_eq!(
            compiler
                .compile_raw(root)
                .unwrap()
                .sample_value(&mut SamplerContext::default(), [0; 3])
                .unwrap(),
            1.0
        );
        densities.insert("minecraft:z".into(), r#""a""#.into());
        assert!(DensityRegistry::from_json(&BTreeMap::new(), &densities).is_err());
        densities.remove("minecraft:z");
        assert!(DensityRegistry::from_json(&BTreeMap::new(), &densities).is_err());
    }
    #[test]
    fn codec_validation_rejects_invalid_gradients_intervals_and_splines() {
        for json in [
            r#"{"type":"gradient","axis":"y","from_coordinate":0,"to_coordinate":0,"from_value":0,"to_value":1}"#,
            r#"{"type":"interval_select","input":0,"thresholds":[0,-0],"functions":[0,1,2]}"#,
            r#"{"type":"interpolated","input":0,"cell_size_xz":0,"cell_size_y":8}"#,
            r#"{"type":"spline","spline":{"coordinate":0,"points":[]}}"#,
            r#"{"type":"clamp","input":0,"min":1,"max":-1}"#,
        ] {
            assert!(
                DensityRegistry::from_json(
                    &BTreeMap::new(),
                    &BTreeMap::from([("minecraft:a".into(), json.into())])
                )
                .is_err(),
                "{json}"
            );
        }
    }

    /// A mod-registered leaf: `x + 2y + 4z + seed + 0.25`, or a record of the positions a
    /// Y-invariant one was asked for.
    #[derive(Debug)]
    struct Linear {
        seed: i64,
        asked: Option<Arc<std::sync::Mutex<Vec<[i32; 3]>>>>,
    }
    impl DensitySampler for Linear {
        fn sample_value(
            &self,
            _: &mut SamplerContext,
            [x, y, z]: [i32; 3],
        ) -> Result<f32, sunrise_core::density::SampleError> {
            if let Some(asked) = &self.asked {
                asked.lock().unwrap().push([x, y, z]);
                return Ok(x as f32);
            }
            Ok((x + 2 * y + 4 * z) as f32 + self.seed as f32 + 0.25)
        }
    }
    #[derive(Debug, Default)]
    struct LinearType {
        asked: Option<Arc<std::sync::Mutex<Vec<[i32; 3]>>>>,
    }
    impl ModDensityType for LinearType {
        fn decode(
            &self,
            json: &str,
            seed: i64,
        ) -> Result<super::super::mod_density_types::ModDensityLeaf, String> {
            if json.contains("refuse") {
                return Err("the fixture refuses this node".into());
            }
            Ok(super::super::mod_density_types::ModDensityLeaf {
                sampler: Arc::new(Linear {
                    seed,
                    asked: self.asked.clone(),
                }),
                range: sunrise_core::density::Interval::new(-1e9, 1e9).unwrap(),
                varies_with_y: self.asked.is_none(),
            })
        }
    }
    fn linear_types(ty: LinearType) -> Arc<ModDensityTypes> {
        Arc::new(BTreeMap::from([(
            "test:linear".to_owned(),
            Arc::new(ty) as Arc<dyn ModDensityType>,
        )]))
    }

    /// `DensityFunctions.DIRECT_CODEC` dispatches a registered type for a named entry, a
    /// node inside a vanilla one and an inline root alike, point and volume agreeing.
    #[test]
    fn a_mod_density_type_decodes_named_nested_and_inline_nodes() {
        let densities = BTreeMap::from([(
            "test:leaf".to_owned(),
            r#"{"type":"test:linear"}"#.to_owned(),
        )]);
        let mut registry = DensityRegistry::from_json_with_types(
            &BTreeMap::new(),
            &densities,
            linear_types(LinearType::default()),
            7,
        )
        .unwrap();
        let roots = registry
            .decode_functions(&[
                r#"{"type":"minecraft:add","left":"test:leaf","right":2}"#,
                r#"{"type":"test:linear"}"#,
            ])
            .unwrap();
        let compiler = DensityCompiler::new(registry.into_graph());
        let mut context = SamplerContext::default();
        let sum = compiler.compile_raw(roots[0]).unwrap();
        assert_eq!(sum.sample_value(&mut context, [1, -2, 3]).unwrap(), 18.25);
        let volume =
            sunrise_core::noise::DensityVolume::new([2, 2, 2], [1, -2, 3], [2, 3, 5]).unwrap();
        let mut values = [0.0; 8];
        sum.sample_volume(&mut context, &mut values, volume)
            .unwrap();
        assert_eq!(
            values,
            [18.25, 24.25, 20.25, 26.25, 38.25, 44.25, 40.25, 46.25]
        );
        let inline = compiler.compile_raw(roots[1]).unwrap();
        assert_eq!(
            inline.sample_value(&mut context, [1, -2, 3]).unwrap(),
            16.25
        );
        // Another world's seed reaches the function.
        let registry = DensityRegistry::from_json_with_types(
            &BTreeMap::new(),
            &densities,
            linear_types(LinearType::default()),
            8,
        )
        .unwrap();
        let leaf = registry.value("test:leaf").unwrap();
        let compiler = DensityCompiler::new(registry.into_graph());
        assert_eq!(
            compiler
                .compile_raw(leaf)
                .unwrap()
                .sample_value(&mut context, [1, -2, 3])
                .unwrap(),
            17.25
        );
    }

    /// An unregistered type and a node its type refuses fail the load, naming the type.
    #[test]
    fn a_refused_or_unknown_mod_density_node_fails_the_load() {
        let load = |json: &str| {
            DensityRegistry::from_json_with_types(
                &BTreeMap::new(),
                &BTreeMap::from([("test:leaf".to_owned(), json.to_owned())]),
                linear_types(LinearType::default()),
                0,
            )
            .unwrap_err()
            .to_string()
        };
        let refused = load(r#"{"type":"test:linear","refuse":true}"#);
        assert!(
            refused.contains("test:linear") && refused.contains("the fixture refuses"),
            "{refused}"
        );
        assert!(load(r#"{"type":"test:other"}"#).contains("unknown density type: test:other"));
        // Without the mods' types, vanilla's loader knows none of them.
        assert!(
            DensityRegistry::from_json(
                &BTreeMap::new(),
                &BTreeMap::from([(
                    "test:leaf".to_owned(),
                    r#"{"type":"test:linear"}"#.to_owned()
                )]),
            )
            .is_err()
        );
    }

    /// A function that doesn't vary with Y is asked at Y = 0 only (`SliceUniformAxes`).
    #[test]
    fn a_y_invariant_mod_density_node_is_asked_at_y_zero() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut registry = DensityRegistry::from_json_with_types(
            &BTreeMap::new(),
            &BTreeMap::new(),
            linear_types(LinearType {
                asked: Some(asked.clone()),
            }),
            0,
        )
        .unwrap();
        let root = registry
            .decode_functions(&[r#"{"type":"test:linear"}"#])
            .unwrap()[0];
        let sampler = DensityCompiler::new(registry.into_graph())
            .get_sampler(root)
            .unwrap();
        let volume =
            sunrise_core::noise::DensityVolume::new([2, 3, 1], [0, 10, 0], [1, 1, 1]).unwrap();
        let mut values = [0.0; 6];
        sampler
            .sample_volume(&mut SamplerContext::default(), &mut values, volume)
            .unwrap();
        assert_eq!(values, [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        let asked = asked.lock().unwrap();
        assert!(
            !asked.is_empty() && asked.iter().all(|&[_, y, _]| y == 0),
            "{asked:?}"
        );
    }
}
