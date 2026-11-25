//! Explicit per-level attachment views (`LevelRef::attachments` and `LevelRef::attachments_mut`).

use crate::{
    AttachmentAccess, AttachmentError, AttachmentHost, AttachmentKey, AttachmentStoreKey,
    Attachments, Holder, Host, LevelRef, ServerHost,
};

/// Read-only view of one open server level's attachments, whether that level is in place or
/// parked.
///
/// Reaches that level's own attachment map directly without loading or switching levels; there
/// is no second store. Reads through [`LevelAttachments`] never insert defaults or mutate the
/// map. A stale handle, an unopened or closed level, or a client returns
/// [`AttachmentError::MissingHolder`].
// no wire: borrowed level attachment view; stays scoped to the host borrow.
// exhaustive: its fields are private; constructed through LevelRef::attachments
#[derive(Clone, Copy)]
pub struct LevelAttachments<'a> {
    level: LevelRef,
    host: &'a dyn Host,
}

impl<'a> LevelAttachments<'a> {
    fn read_map(&self) -> Result<&'a Attachments, AttachmentError> {
        crate::backend::backend_of(self.host)
            .and_then(|backend| backend.level_attachments_read(self.level))
            .ok_or(AttachmentError::MissingHolder)
    }

    /// Lend the level's value for `key`, or its default if absent, without inserting or mutating
    /// the map. A saved entry decodes for this read alone without caching into the map.
    pub fn get<T: Send + Sync + 'static, R>(
        &self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        let key = key.live()?;
        let mut store = *self;
        key.get_existing(&mut store, Holder::Level, |value| match value {
            Some(value) => read(value),
            None => read(&key.default_value()),
        })
    }

    /// Whether this level currently carries a value for `key`, saved or decoded; never inserts
    /// the default.
    pub fn has<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentKey<T>,
    ) -> Result<bool, AttachmentError> {
        let mut store = *self;
        key.live()?.has(&mut store, Holder::Level)
    }

    /// Lend the level's existing value for `key`, or `None` if absent. A saved entry decodes for
    /// this read alone without mutating the map; never inserts a default.
    pub fn get_existing<T: Send + Sync + 'static, R>(
        &self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        let mut store = *self;
        key.live()?.get_existing(&mut store, Holder::Level, read)
    }

    /// Borrow an already-decoded value for `key` on this level without decoding or inserting a
    /// default.
    pub fn with_decoded<T: Send + Sync + 'static, R>(
        &self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        let mut store = *self;
        key.live()?.with_decoded(&mut store, Holder::Level, read)
    }
}

impl AttachmentHost for LevelAttachments<'_> {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError> {
        crate::parts::validate_attachment_use(namespace, name, sync)
    }

    fn validate_attachment_key(&self, key: &AttachmentStoreKey<'_>) -> Result<(), AttachmentError> {
        crate::parts::validate_attachment_key(key)
    }

    fn attachments(
        &mut self,
        holder: Holder,
        _access: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
        if holder != Holder::Level {
            return Err(AttachmentError::MissingHolder);
        }
        let _ = self.read_map()?;
        Err(AttachmentError::ReadOnlyHolder)
    }

    fn read_only_attachments(
        &self,
        holder: Holder,
    ) -> Option<Result<Option<&Attachments>, AttachmentError>> {
        if holder != Holder::Level {
            return Some(Err(AttachmentError::MissingHolder));
        }
        Some(self.read_map().map(Some))
    }
}

/// Mutable view of one open server level's attachments, whether that level is in place or
/// parked.
///
/// Reaches that level's own attachment map directly without loading or switching levels; there
/// is no second store. Writes join any open `Host` transaction and roll back on abort; outside a
/// transaction they apply at once. A stale handle, an unopened or closed level, or a client
/// returns [`AttachmentError::MissingHolder`].
// no wire: borrowed mutable level attachment view; stays scoped to the server host borrow.
// exhaustive: its fields are private; constructed through LevelRef::attachments_mut
pub struct LevelAttachmentsMut<'a> {
    level: LevelRef,
    host: &'a mut dyn ServerHost,
}

impl LevelAttachmentsMut<'_> {
    /// Lend the level's value for `key`, inserting its default if absent.
    pub fn get<T: Send + Sync + 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        key.live()?.get(self, Holder::Level, read)
    }

    /// Whether this level currently carries a value for `key`, saved or decoded; never inserts
    /// the default.
    pub fn has<T: Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<bool, AttachmentError> {
        key.live()?.has(self, Holder::Level)
    }

    /// Lend the level's existing value for `key`, or `None` if absent. Decodes a saved entry
    /// once and never inserts a default.
    pub fn get_existing<T: Send + Sync + 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        key.live()?.get_existing(self, Holder::Level, read)
    }

    /// Borrow an already-decoded value for `key` on this level without decoding or inserting a
    /// default.
    pub fn with_decoded<T: Send + Sync + 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        key.live()?.with_decoded(self, Holder::Level, read)
    }

    /// Replace the value for `key` on this level.
    pub fn set<T: Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        key.live()?.set(self, Holder::Level, value)
    }

    /// Remove the value for `key` from this level, returning the previous value if one was
    /// present.
    pub fn remove<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<Option<T>, AttachmentError> {
        key.live()?.remove(self, Holder::Level)
    }

    /// Queue the current value for `key` on this level for synchronization to clients that see
    /// it, or a removal marker if absent. A live key with no sync codec does nothing.
    pub fn sync_data<T: Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<(), AttachmentError> {
        key.live()?.sync_data(self, Holder::Level)
    }
}

impl AttachmentHost for LevelAttachmentsMut<'_> {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError> {
        crate::parts::validate_attachment_use(namespace, name, sync)
    }

    fn validate_attachment_key(&self, key: &AttachmentStoreKey<'_>) -> Result<(), AttachmentError> {
        crate::parts::validate_attachment_key(key)
    }

    fn read_attachments(
        &mut self,
        holder: Holder,
    ) -> Result<Option<&Attachments>, AttachmentError> {
        if holder != Holder::Level {
            return Err(AttachmentError::MissingHolder);
        }
        crate::backend::backend_of(&*self.host)
            .and_then(|backend| backend.level_attachments_read(self.level))
            .map(Some)
            .ok_or(AttachmentError::MissingHolder)
    }

    fn attachment_changed(&mut self, holder: Holder, _persist: bool, sync: bool) {
        if holder == Holder::Level
            && sync
            && let Some(backend) = crate::backend::backend_mut_of(self.host)
        {
            backend.level_attachment_changed(self.level, true);
        }
    }

    fn attachments(
        &mut self,
        holder: Holder,
        _access: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
        if holder != Holder::Level {
            return Err(AttachmentError::MissingHolder);
        }
        crate::backend::backend_mut_of(self.host)
            .and_then(|backend| backend.level_attachments_mut(self.level))
            .ok_or(AttachmentError::MissingHolder)
    }
}

impl LevelRef {
    /// This level's attachments, for any open server level, in place or not.
    /// Does not load or switch levels.
    pub fn attachments<'a>(&self, host: &'a dyn Host) -> LevelAttachments<'a> {
        LevelAttachments { level: *self, host }
    }

    /// This level's attachments for writes, for any open server level, in place or not.
    /// Does not load or switch levels.
    pub fn attachments_mut<'a>(&self, host: &'a mut dyn ServerHost) -> LevelAttachmentsMut<'a> {
        LevelAttachmentsMut { level: *self, host }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AttachmentCodec, AttachmentDescriptor, AttachmentStore, Backend, BackendMut, BackendMutRef,
        BackendRef, Compound, DimensionKey, Tag, WorldEpoch,
    };

    struct TwoLevelHost {
        overworld: LevelRef,
        nether: LevelRef,
        overworld_attachments: Attachments,
        nether_attachments: Attachments,
        synced: Vec<LevelRef>,
    }

    impl TwoLevelHost {
        fn new() -> Self {
            let epoch = WorldEpoch::new(1);
            Self {
                overworld: LevelRef::new(epoch, DimensionKey::OVERWORLD),
                nether: LevelRef::new(epoch, DimensionKey::NETHER),
                overworld_attachments: Attachments::default(),
                nether_attachments: Attachments::default(),
                synced: Vec::new(),
            }
        }
    }

    impl Host for TwoLevelHost {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    impl crate::prelude::ModStateHost for TwoLevelHost {
        fn attachments(&mut self) -> &mut dyn AttachmentStore {
            &mut self.overworld_attachments
        }
    }
    crate::impl_host_areas!(TwoLevelHost, except(mod_state));

    impl ServerHost for TwoLevelHost {
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(BackendMutRef::of(self))
        }
    }
    crate::impl_server_host_areas!(TwoLevelHost);

    impl Backend for TwoLevelHost {}
    impl crate::prelude::ModStateBackend for TwoLevelHost {
        fn level_attachments_read(&self, level: LevelRef) -> Option<&Attachments> {
            if level == self.overworld {
                Some(&self.overworld_attachments)
            } else if level == self.nether {
                Some(&self.nether_attachments)
            } else {
                None
            }
        }
    }
    crate::impl_backend_areas!(TwoLevelHost, except(mod_state));

    impl BackendMut for TwoLevelHost {}
    impl crate::prelude::ModStateBackendMut for TwoLevelHost {
        fn level_attachments_mut(&mut self, level: LevelRef) -> Option<&mut Attachments> {
            if level == self.overworld {
                Some(&mut self.overworld_attachments)
            } else if level == self.nether {
                Some(&mut self.nether_attachments)
            } else {
                None
            }
        }

        fn level_attachment_changed(&mut self, level: LevelRef, sync: bool) {
            if sync {
                self.synced.push(level);
            }
        }
    }
    crate::impl_backend_mut_areas!(TwoLevelHost, except(mod_state));

    #[test]
    fn holder_level_through_host_still_names_level_in_place() {
        let codec = AttachmentCodec {
            encode: |value: &u32| {
                let mut tag = Compound::new();
                tag.insert("n".into(), Tag::Int(*value as i32));
                Ok(tag)
            },
            decode: |tag| match tag.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n as u32),
                _ => Err(AttachmentError::Decode),
            },
        };
        let (record, key) = crate::parts::registered_attachment_fixture(
            "demo",
            AttachmentDescriptor::new("demo:count", || 7u32).sync(codec),
        );
        crate::parts::install_input(crate::parts::installation::InstalledInput {
            attachment_types: std::collections::HashMap::from([(
                "demo",
                std::collections::HashMap::from([("count", record)]),
            )]),
            ..Default::default()
        });

        let mut host = TwoLevelHost::new();
        let overworld = host.overworld;
        let nether = host.nether;
        let stale = LevelRef::new(WorldEpoch::new(2), DimensionKey::OVERWORLD);

        // Holder::Level through the host writes the level in place (overworld).
        key.set(&mut host, Holder::Level, 11).unwrap();
        assert_eq!(overworld.attachments(&host).get(&key, |v| *v), Ok(11));
        assert_eq!(nether.attachments(&host).has(&key), Ok(false));
        assert_eq!(nether.attachments(&host).get(&key, |v| *v), Ok(7));
        // Read-only view does not insert the default.
        assert_eq!(nether.attachments(&host).has(&key), Ok(false));

        // Writing through nether.attachments_mut leaves Holder::Level (overworld) unchanged.
        nether.attachments_mut(&mut host).set(&key, 42).unwrap();
        assert_eq!(key.get(&mut host, Holder::Level, |v| *v), Ok(11));
        assert_eq!(
            nether.attachments(&host).get_existing(&key, |v| v.copied()),
            Ok(Some(42))
        );
        assert_eq!(host.synced, vec![nether]);

        // AttachmentStore trait methods work with Holder::Level and refuse other holders / writes on read-only view.
        let checked = key.live().unwrap();
        assert_eq!(
            checked.set(&mut nether.attachments(&host), Holder::Level, 99),
            Err(AttachmentError::ReadOnlyHolder)
        );
        assert_eq!(
            checked.get_existing(&mut nether.attachments(&host), Holder::Entity(1), |v| v
                .copied()),
            Err(AttachmentError::MissingHolder)
        );
        assert_eq!(
            stale.attachments(&host).has(&key),
            Err(AttachmentError::MissingHolder)
        );
        assert_eq!(
            stale.attachments_mut(&mut host).set(&key, 1),
            Err(AttachmentError::MissingHolder)
        );
    }
}
