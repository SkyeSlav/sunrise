//! Engine-owned tag reads passed only while evaluating custom loot kinds.
use crate::tag_layers::TagLayers;
use std::collections::{HashMap, HashSet};
use std::{io, sync::Arc};
use sunrise_core::identifier::{Identifier, IdentifierError};

pub(super) type BoundTags = HashMap<String, HashMap<String, Vec<String>>>;

/// Typed names for one generation of bound item tags. The original spelling
/// lets enumeration reuse that generation's membership lookup without formatting.
#[derive(Clone)]
pub struct ItemTagNames(Arc<[(String, Result<Identifier, IdentifierError>)]>);

impl ItemTagNames {
    pub fn bind(tags: &super::RegistryTags) -> Self {
        Self(
            tags.keys()
                .map(|name| (name.clone(), Identifier::parse(name)))
                .collect(),
        )
    }

    /// Clone the matching names, or refuse the query if a matching name is invalid.
    pub fn matching(&self, mut matches: impl FnMut(&str) -> bool) -> Option<Vec<Identifier>> {
        self.0
            .iter()
            .filter(|(name, _)| matches(name))
            .map(|(_, id)| id.as_ref().ok().cloned())
            .collect()
    }
}

/// Reverse membership for one generation of a registry's bound tags.
#[derive(Clone)]
pub struct MemberTagNames(Arc<HashMap<String, Option<Vec<Identifier>>>>);

impl MemberTagNames {
    /// Build from the final bindings, as `MappedRegistry.refreshTagsInHolders` does.
    pub fn bind(tags: &super::RegistryTags) -> Self {
        let mut by_member = HashMap::<String, Option<Vec<Identifier>>>::new();
        for (name, members) in tags {
            let name = Identifier::parse(name).ok();
            for member in members {
                let names = by_member
                    .entry(member.clone())
                    .or_insert_with(|| Some(Vec::new()));
                match (&name, names.as_mut()) {
                    (Some(name), Some(names)) => names.push(name.clone()),
                    (None, _) => *names = None,
                    (Some(_), None) => {}
                }
            }
        }
        for names in by_member.values_mut().flatten() {
            names.sort_unstable();
            names.dedup();
        }
        Self(Arc::new(by_member))
    }

    /// A member absent from the index has no tags. Refuse a member with any invalid tag name.
    pub fn names(&self, member: &str) -> Option<Vec<Identifier>> {
        self.0
            .get(member)
            .cloned()
            .unwrap_or_else(|| Some(Vec::new()))
    }
}

/// Authoritative tag reads without a game or registry borrow.
/// Registry names are vanilla directory names (`item`, `block`, `fluid`).
pub trait LootTags {
    /// Members of a loaded tag in its resolved order, or `None` if absent.
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>>;
    /// The bound item tags, including empty tags, or `None` before the registry binds.
    fn item_tag_names(&self) -> Option<&ItemTagNames>;
    /// Bound tag names for a resolved block or fluid, or `None` if unavailable or incomplete.
    fn member_tag_names(&self, _registry: &str, _member: &str) -> Option<Vec<Identifier>> {
        None
    }
    /// Whether one registry value belongs to a tag.
    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.members(registry, tag)
            .is_some_and(|members| members.iter().any(|member| member == id))
    }
}

impl LootTags for super::LootData {
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>> {
        self.tags.get(registry)?.get(tag).cloned()
    }

    fn item_tag_names(&self) -> Option<&ItemTagNames> {
        self.item_tag_names.as_ref()
    }

    fn member_tag_names(&self, registry: &str, member: &str) -> Option<Vec<Identifier>> {
        match registry {
            "block" => self.block_tag_names.as_ref(),
            "fluid" => self.fluid_tag_names.as_ref(),
            _ => None,
        }?
        .names(member)
    }

    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.tags
            .get(registry)
            .and_then(|tags| tags.get(tag))
            .is_some_and(|members| members.iter().any(|member| member == id))
    }
}

/// Unlike registry tags, cycles in a loot predicate or modifier tag are invalid.
pub(super) fn check_cycles(registry: &str, tags: &HashMap<String, TagLayers>) -> io::Result<()> {
    let mut active = HashSet::new();
    let mut done = HashSet::new();
    let mut pending = Vec::new();
    for name in tags.keys().map(String::as_str) {
        pending.push((name, false));
        while let Some((name, ready)) = pending.pop() {
            if ready {
                active.remove(name);
                done.insert(name);
            } else if !done.contains(name)
                && let Some(layers) = tags.get(name)
            {
                if !active.insert(name) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("tags/{registry} {name}: reference cycle"),
                    ));
                }
                pending.push((name, true));
                pending.extend(layers.tags().map(|tag| (tag, false)));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_binding_lists_distinct_names_and_refuses_only_affected_members() {
        let index = MemberTagNames::bind(&HashMap::from([
            (
                "test:a".into(),
                vec!["minecraft:stone".into(), "minecraft:stone".into()],
            ),
            (
                "test:b".into(),
                vec!["minecraft:stone".into(), "minecraft:dirt".into()],
            ),
            ("test:other".into(), vec!["minecraft:dirt".into()]),
            ("test:empty".into(), vec![]),
            ("..:bad".into(), vec!["minecraft:dirt".into()]),
        ]));
        let mut stone = index.names("minecraft:stone").unwrap();
        stone.sort_unstable();
        assert_eq!(
            stone,
            [
                Identifier::parse("test:a").unwrap(),
                Identifier::parse("test:b").unwrap()
            ]
        );
        assert_eq!(index.names("minecraft:air"), Some(vec![]));
        assert_eq!(index.names("minecraft:dirt"), None);
    }
}
