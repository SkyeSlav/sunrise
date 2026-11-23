//! At each Parts install (world open and `/reload`), the server publishes its resolved Part
//! table, block-state index and merged block tags for readers on workers and the server thread.
use super::api_values::block_state::Index;
use super::native_world_read::BlockTagSets;
use std::sync::{Arc, PoisonError, RwLock};
use sunrise_client_render::api_context::WorldgenReader;
use sunrise_world::registry::BlockRegistry;

#[derive(Clone)]
struct Published<'a> {
    index: Arc<Index>,
    registry: &'a BlockRegistry,
    block_tags: Option<Arc<BlockTagSets>>,
}

static INDEX: RwLock<Option<Published<'static>>> = RwLock::new(None);

/// Held by each test that publishes: the published tables are process-wide.
#[cfg(test)]
pub(in crate::game) static PUBLISHING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Publish the server's resolved Parts, its worker resource's block index, and the same bound
/// block tags that the server installs for live reads.
pub(in crate::game) fn publish(
    parts: Option<&mod_api::Parts>,
    index: &Arc<Index>,
    registry: &'static BlockRegistry,
    block_tags: Option<Arc<BlockTagSets>>,
) {
    *INDEX.write().unwrap_or_else(PoisonError::into_inner) = Some(Published {
        index: index.clone(),
        registry,
        block_tags,
    });
    mod_api::__private::publish_worldgen_parts(parts.cloned());
}

/// Retire only this world's tags. Different worlds can share a registry and index, but each
/// bind builds its own tag snapshot. Callbacks that already captured it keep their read view.
pub(super) fn retire_block_tags(owned: &Arc<BlockTagSets>) {
    let mut published = INDEX.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(published) = published.as_mut()
        && published
            .block_tags
            .as_ref()
            .is_some_and(|tags| Arc::ptr_eq(tags, owned))
    {
        published.block_tags = None;
    }
}

/// The index the server published last; None before a server published one.
pub(in crate::game) fn index() -> Option<Arc<Index>> {
    INDEX
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|published| published.index.clone())
}

/// Runs `f` with the server's worldgen context and its captured block queries. Returns None
/// before a server published its context.
pub(in crate::game) fn with_host<R>(
    f: impl FnOnce(&mod_api::OwningWorldgen<'_>) -> R,
) -> Option<R> {
    let context = sunrise_world::native_worldgen_hooks::context()?;
    let published = INDEX.read().unwrap_or_else(PoisonError::into_inner).clone();
    Some(with_snapshot(
        &WorldgenReader(&context),
        published.as_ref(),
        f,
    ))
}

fn with_snapshot<R>(
    reader: &WorldgenReader<'_>,
    published: Option<&Published<'_>>,
    f: impl FnOnce(&mod_api::OwningWorldgen<'_>) -> R,
) -> R {
    let state_with_property = |state: mod_api::BlockState, key: &str, value: &str| {
        let published = published?;
        let engine = published.index.to_engine(state).ok()?;
        let set = super::native_hooks::with_property(published.registry, engine, key, value)?;
        published.index.to_api(set).ok()
    };
    let block_in_tag = |block: mod_api::BlockKey, tag: &str| {
        let published = published?;
        let tags = published.block_tags.as_deref()?;
        published
            .index
            .has_block(block)
            .then(|| tags.contains(block, tag))
    };
    f(&mod_api::OwningWorldgen::new(
        reader,
        &state_with_property,
        Some(&block_in_tag),
    ))
}

/// The engine's feature callback: one feature's Part lookups read the tables published when it
/// started.
pub(crate) fn feature(place: &mut dyn FnMut()) {
    mod_api::__private::worldgen_feature(place);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::{block_state, resolve_block_definitions};
    use crate::game::native_world_read::prepare_block_tags;
    use mod_api::WorldgenHost;
    use std::collections::HashMap;
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};

    #[test]
    fn worldgen_block_tags_read_the_captured_server_generation() {
        mod_platform::testing::with_mods(Default::default(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
            let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:dirt":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"]]}"#;
            let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            resolve_block_definitions(&registry).unwrap();
            let key = |id| mod_api::__private::block_key(id).unwrap();
            let stone = key("minecraft:stone");
            let dirt = key("minecraft:dirt");
            let unresolved = mod_api::__private::parts_installation::block_key(u32::MAX);
            let mut published = Published {
                index: block_state::current().unwrap(),
                registry: &registry,
                block_tags: Some(prepare_block_tags(&HashMap::from([
                    ("minecraft:soil".into(), ["minecraft:stone".into()].into()),
                    ("fixture:empty".into(), Default::default()),
                    ("fixture:soil".into(), ["minecraft:dirt".into()].into()),
                ]))),
            };
            let load = LoadContext::new(
                DataLoadOrigin::ServerDataLoad,
                Arc::new(LoadInputs::new(Default::default(), Default::default())),
                [],
            );
            let context = load.worldgen_context([]);
            let reader = WorldgenReader(&context);
            let captured = published.clone();
            with_snapshot(&reader, Some(&captured), |host| {
                for name in ["soil", "minecraft:soil", ":soil"] {
                    assert_eq!(host.block_in_tag(stone, name), Some(true));
                }
                for name in [
                    "fixture:empty",
                    "missing",
                    "Fixture:soil",
                    "minecraft:bad path",
                    "",
                    "minecraft:",
                ] {
                    assert_eq!(host.block_in_tag(stone, name), Some(false), "tag {name:?}");
                }
                assert_eq!(host.block_in_tag(dirt, "soil"), Some(false));
                assert_eq!(host.block_in_tag(dirt, "fixture:soil"), Some(true));
                assert_eq!(host.block_in_tag(unresolved, "soil"), None);

                published.block_tags = Some(prepare_block_tags(&HashMap::from([(
                    "minecraft:soil".into(),
                    ["minecraft:dirt".into()].into(),
                )])));
                with_snapshot(&reader, Some(&published), |new_host| {
                    assert_eq!(new_host.block_in_tag(stone, "soil"), Some(false));
                    assert_eq!(new_host.block_in_tag(dirt, "soil"), Some(true));
                });
                assert_eq!(host.block_in_tag(stone, "soil"), Some(true));
            });

            std::thread::scope(|scope| {
                scope.spawn(|| {
                    with_snapshot(&reader, Some(&published), |host| {
                        assert_eq!(host.block_in_tag(dirt, "soil"), Some(true));
                        assert_eq!(host.block_in_tag(stone, "soil"), Some(false));
                    });
                });
            });

            for (tags, answer) in [(None, None), (Some(Arc::default()), Some(false))] {
                published.block_tags = tags;
                with_snapshot(&reader, Some(&published), |host| {
                    assert_eq!(host.block_in_tag(stone, "soil"), answer);
                });
            }
        });
    }
}
