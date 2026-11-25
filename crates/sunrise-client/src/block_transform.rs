//! The `minecraft:block_transformer` registry (axe, hoe, shovel) that
//! `BlockTransformer.transformBlock` reads: each transform's rule based state
//! provider is evaluated against the clicked block and the block above.
//! `tools/test_block_transformer_parity.py` compares every block state with
//! vanilla.
use crate::{Result, items::Items};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};
use sunrise_world::{chunk::BlockState, registry::BlockRegistry};

#[derive(Clone)]
enum Predicate {
    Blocks(Vec<String>),
    Tag { tag: String, offset: [i32; 3] },
    AllOf(Vec<Predicate>),
    AnyOf(Vec<Predicate>),
}

#[derive(Clone)]
enum Provider {
    Rules(Vec<(Predicate, Provider)>),
    State(BlockState),
    /// `CopyPropertiesProvider`: the source with the clicked block's properties.
    Copy(Box<Provider>),
}

/// One `BlockTransformData`.
#[derive(Clone)]
pub struct Transform {
    provider: Provider,
    pub disallowed_faces: Vec<String>,
    pub sound: String,
    pub particle: String,
    pub loot: Option<String>,
    pub drop_strategy: String,
    pub update_from_neighbors: bool,
    pub transform_type: String,
    pub consume_on_use: bool,
    pub damage: u32,
}

pub struct Transformers {
    transformers: HashMap<String, Vec<Transform>>,
}

/// What the transformer reads of the world: the block state at an offset from
/// the clicked position.
pub trait Around {
    fn state(&self, offset: [i32; 3]) -> BlockState;
}

fn state_of(value: &Value, registry: &BlockRegistry) -> Option<BlockState> {
    let name = value["id"].as_str().or_else(|| value["Name"].as_str())?;
    let default = registry.default_state(name)?;
    let mut state = registry.state(default)?.clone();
    for (k, v) in value["properties"].as_object().into_iter().flatten() {
        state.properties.insert(k.clone(), v.as_str()?.to_owned());
    }
    Some(state)
}

fn predicate(value: &Value) -> Option<Predicate> {
    Some(match value["type"].as_str()? {
        "minecraft:matching_blocks" => Predicate::Blocks(match &value["blocks"] {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .filter_map(|b| b.as_str().map(str::to_owned))
                .collect(),
            _ => return None,
        }),
        "minecraft:matching_block_tag" => {
            let o = value["offset"].as_array();
            let at = |i: usize| {
                o.and_then(|o| o.get(i))
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32
            };
            Predicate::Tag {
                tag: value["tag"].as_str()?.to_owned(),
                offset: [at(0), at(1), at(2)],
            }
        }
        "minecraft:all_of" => Predicate::AllOf(
            value["predicates"]
                .as_array()?
                .iter()
                .map(predicate)
                .collect::<Option<_>>()?,
        ),
        "minecraft:any_of" => Predicate::AnyOf(
            value["predicates"]
                .as_array()?
                .iter()
                .map(predicate)
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

fn provider(value: &Value, registry: &BlockRegistry) -> Option<Provider> {
    match value["type"].as_str() {
        Some("minecraft:rule_based") => {
            let mut rules = Vec::new();
            for rule in value["rules"].as_array()? {
                rules.push((
                    predicate(&rule["if_true"])?,
                    provider(&rule["then"], registry)?,
                ));
            }
            Some(Provider::Rules(rules))
        }
        Some("minecraft:copy_properties") => Some(Provider::Copy(Box::new(provider(
            &value["source"],
            registry,
        )?))),
        Some("minecraft:simple_state_provider") => {
            state_of(&value["state"], registry).map(Provider::State)
        }
        _ => state_of(value, registry).map(Provider::State),
    }
}

/// Decode one `BlockTransformer.BlockTransformData` JSON object, validating field types when
/// present and filling vanilla's defaults when omitted (used by `neoforge:transformables`
/// and `neoforge:block_transform_appenders`).
pub fn parse_transform(
    value: &Value,
    registry: &BlockRegistry,
) -> std::result::Result<Transform, String> {
    let Some(object) = value.as_object() else {
        return Err("block transform data must be an object".into());
    };
    let Some(raw_provider) = object.get("block_state_provider") else {
        return Err("block transform data missing block_state_provider".into());
    };
    let Some(provider) = provider(raw_provider, registry) else {
        return Err("invalid block_state_provider in block transform data".into());
    };
    let string_field = |key: &str, default: &str| -> std::result::Result<String, String> {
        match object.get(key) {
            None => Ok(default.to_owned()),
            Some(Value::String(text)) => Ok(text.clone()),
            Some(_) => Err(format!("expected string in {key}")),
        }
    };
    let bool_field = |key: &str, default: bool| -> std::result::Result<bool, String> {
        match object.get(key) {
            None => Ok(default),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(_) => Err(format!("expected boolean in {key}")),
        }
    };
    let disallowed_faces = match object.get("disallowed_faces") {
        None => Vec::new(),
        Some(Value::Array(faces)) => faces
            .iter()
            .map(|face| {
                face.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "expected face string in disallowed_faces".to_owned())
            })
            .collect::<std::result::Result<Vec<_>, _>>()?,
        Some(_) => return Err("expected array in disallowed_faces".into()),
    };
    let loot = match object.get("loot") {
        None => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => return Err("expected string in loot".into()),
    };
    let damage = match object.get("item_damage_per_use") {
        None => 0,
        Some(raw) => raw
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "expected non-negative integer in item_damage_per_use".to_owned())?,
    };
    Ok(Transform {
        provider,
        disallowed_faces,
        sound: string_field("sound", "minecraft:empty")?,
        particle: string_field("particle", "none")?,
        loot,
        drop_strategy: string_field("drop_strategy", "from_middle")?,
        update_from_neighbors: bool_field("update_from_neighbors", true)?,
        transform_type: string_field("transform_type", "single_block")?,
        consume_on_use: bool_field("consume_on_use", true)?,
        damage,
    })
}

/// `BlockState.withPropertiesOf`: the properties both blocks have.
fn with_properties_of(
    registry: &BlockRegistry,
    mut state: BlockState,
    old: &BlockState,
) -> Option<BlockState> {
    let defaults = registry
        .default_state(&state.name)
        .and_then(|id| registry.state(id))?;
    for (k, v) in &old.properties {
        if defaults.properties.contains_key(k) {
            let mut candidate = state.clone();
            candidate.properties.insert(k.clone(), v.clone());
            if registry.resolve(&candidate).is_ok() {
                state = candidate;
            }
        }
    }
    Some(state)
}

impl Predicate {
    fn test(&self, items: &Items, around: &dyn Around) -> bool {
        match self {
            Self::Blocks(names) => {
                let state = around.state([0, 0, 0]);
                names.contains(&state.name)
            }
            Self::Tag { tag, offset } => {
                let state = around.state(*offset);
                items
                    .block_tags
                    .get(tag)
                    .is_some_and(|members| members.contains(&state.name))
            }
            Self::AllOf(all) => all.iter().all(|p| p.test(items, around)),
            Self::AnyOf(any) => any.iter().any(|p| p.test(items, around)),
        }
    }
}

impl Provider {
    fn state(
        &self,
        items: &Items,
        registry: &BlockRegistry,
        around: &dyn Around,
    ) -> Option<BlockState> {
        match self {
            Self::Rules(rules) => rules
                .iter()
                .find(|(p, _)| p.test(items, around))
                .and_then(|(_, then)| then.state(items, registry, around)),
            Self::State(state) => Some(state.clone()),
            Self::Copy(source) => {
                let state = source.state(items, registry, around)?;
                with_properties_of(registry, state, &around.state([0, 0, 0]))
            }
        }
    }
}

impl Transformers {
    /// An empty transformer table (used in unit tests).
    pub fn empty() -> Self {
        Self {
            transformers: HashMap::new(),
        }
    }

    /// Reads `data/minecraft/block_transformer/*.json`.
    pub fn load(root: &Path, registry: &BlockRegistry) -> Result<Self> {
        let mut transformers = HashMap::new();
        let dir = root.join("data/minecraft/block_transformer");
        if dir.exists() {
            for entry in fs::read_dir(&dir)?.flatten() {
                let path = entry.path();
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let list: Value = serde_json::from_slice(&fs::read(&path)?)?;
                let mut transforms = Vec::new();
                for t in list.as_array().into_iter().flatten() {
                    let text = |k: &str, d: &str| t[k].as_str().unwrap_or(d).to_owned();
                    let Some(provider) = provider(&t["block_state_provider"], registry) else {
                        continue;
                    };
                    transforms.push(Transform {
                        provider,
                        disallowed_faces: t["disallowed_faces"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|f| f.as_str().map(str::to_owned))
                            .collect(),
                        sound: text("sound", "minecraft:empty"),
                        particle: text("particle", "none"),
                        loot: t["loot"].as_str().map(str::to_owned),
                        drop_strategy: text("drop_strategy", "from_middle"),
                        update_from_neighbors: t["update_from_neighbors"].as_bool().unwrap_or(true),
                        transform_type: text("transform_type", "single_block"),
                        consume_on_use: t["consume_on_use"].as_bool().unwrap_or(true),
                        damage: t["item_damage_per_use"].as_u64().unwrap_or(0) as u32,
                    });
                }
                transformers.insert(format!("minecraft:{stem}"), transforms);
            }
        }
        Ok(Self { transformers })
    }

    /// `BlockTransformer.transformBlock` without the world writes: the first
    /// transform that allows the clicked face and provides a state, with its
    /// index.
    pub fn apply<'a>(
        &'a self,
        items: &Items,
        registry: &BlockRegistry,
        id: &str,
        face: &str,
        around: &dyn Around,
    ) -> Option<(usize, &'a Transform, BlockState)> {
        self.apply_with_extra(None, items, registry, id, face, around)
    }

    /// `DataMapHooks.appendDatamapTransformers`: evaluate `id`'s base transforms
    /// followed by any data-map appended transforms (`neoforge:block_transform_appenders`
    /// then `neoforge:transformables`).
    pub fn apply_with_extra<'a>(
        &'a self,
        extra: Option<&'a [Transform]>,
        items: &Items,
        registry: &BlockRegistry,
        id: &str,
        face: &str,
        around: &dyn Around,
    ) -> Option<(usize, &'a Transform, BlockState)> {
        let base = self.transformers.get(id).map(Vec::as_slice).unwrap_or(&[]);
        let extra = extra.unwrap_or(&[]);
        if base.is_empty() && extra.is_empty() {
            return None;
        }
        base.iter()
            .chain(extra.iter())
            .enumerate()
            .filter(|(_, t)| !t.disallowed_faces.iter().any(|f| f == face))
            .find_map(|(i, t)| Some((i, t, t.provider.state(items, registry, around)?)))
    }

    /// `HoneycombItem.getWaxed`: the waxed twin of a copper block, found from
    /// the axe's wax-off transforms (`WAX_OFF_BY_BLOCK` is the inverse of
    /// `WAXABLES`).
    pub fn waxed(&self, registry: &BlockRegistry, old: &BlockState) -> Option<BlockState> {
        let (names, _) = self
            .wax_pairs()
            .find(|(_, unwaxed)| unwaxed.name == old.name)?;
        let target = registry.state(registry.default_state(names.first()?)?)?;
        with_properties_of(registry, target.clone(), old)
    }

    /// `HoneycombItem.WAX_OFF_BY_BLOCK`: the unwaxed twin of `old`, read
    /// from the same axe rules as `waxed`, with shared properties copied.
    pub fn unwaxed(&self, registry: &BlockRegistry, old: &BlockState) -> Option<BlockState> {
        let (_, target) = self
            .wax_pairs()
            .find(|(names, _)| names.contains(&old.name))?;
        with_properties_of(registry, target.clone(), old)
    }

    fn wax_pairs(&self) -> impl Iterator<Item = (&[String], &BlockState)> {
        self.transformers
            .get("minecraft:axe")
            .into_iter()
            .flatten()
            .filter_map(|transform| match &transform.provider {
                Provider::Rules(rules) if transform.particle == "wax_off" => Some(rules),
                _ => None,
            })
            .flatten()
            .filter_map(|(predicate, then)| {
                let (Predicate::Blocks(names), Provider::Copy(source)) = (predicate, then) else {
                    return None;
                };
                let Provider::State(unwaxed) = &**source else {
                    return None;
                };
                (!names.is_empty()).then_some((names.as_slice(), unwaxed))
            })
    }
}
