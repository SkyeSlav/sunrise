//! Shared startup `(domain, namespaced_id) -> Rust type` identity table (mod-to-mod §0.2).
use std::any::TypeId;
use std::collections::{HashMap, hash_map::Entry};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TypeIdentityDomain {
    BlockCapability,
    ItemCapability,
    EntityCapability,
    RenderData,
    FeatureRenderer,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeIdentityEntry {
    pub(crate) type_id: TypeId,
    pub(crate) type_name: &'static str,
    pub(crate) owner: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeIdentityError {
    InvalidName,
    Conflict {
        existing_type_name: &'static str,
        existing_owner: &'static str,
    },
}

/// Startup identity guard shared across capabilities, render data keys, and other in-process
/// typed keys so a `(domain, namespaced_id)` collision with two Rust types is refused at startup
/// naming both mods and both types.
#[derive(Clone, Default)]
pub(crate) struct TypeIdentityTable {
    entries: HashMap<(TypeIdentityDomain, &'static str), TypeIdentityEntry>,
}

impl TypeIdentityTable {
    /// Seed a built-in engine entry before mod registration begins.
    pub(crate) fn seed_builtin(
        &mut self,
        domain: TypeIdentityDomain,
        name: &'static str,
        type_id: TypeId,
        type_name: &'static str,
    ) {
        self.entries.insert(
            (domain, name),
            TypeIdentityEntry {
                type_id,
                type_name,
                owner: "the game",
            },
        );
    }

    /// Register `(domain, name)` with `(type_id, type_name)` for `owner`.
    ///
    /// Returns `Ok(true)` if newly inserted, `Ok(false)` if already registered with the same
    /// `TypeId`, `Err(TypeIdentityError::InvalidName)` if `name` is not a valid namespaced
    /// identifier, or `Err(TypeIdentityError::Conflict { .. })` if already registered with a
    /// different `TypeId`.
    pub(crate) fn register(
        &mut self,
        domain: TypeIdentityDomain,
        name: &'static str,
        type_id: TypeId,
        type_name: &'static str,
        owner: &'static str,
    ) -> Result<bool, TypeIdentityError> {
        if crate::Identifier::parse_namespaced(name).is_none() {
            return Err(TypeIdentityError::InvalidName);
        }
        match self.entries.entry((domain, name)) {
            Entry::Vacant(entry) => {
                entry.insert(TypeIdentityEntry {
                    type_id,
                    type_name,
                    owner,
                });
                Ok(true)
            }
            Entry::Occupied(entry) => {
                let existing = entry.get();
                if existing.type_id == type_id {
                    Ok(false)
                } else {
                    Err(TypeIdentityError::Conflict {
                        existing_type_name: existing.type_name,
                        existing_owner: existing.owner,
                    })
                }
            }
        }
    }
}
