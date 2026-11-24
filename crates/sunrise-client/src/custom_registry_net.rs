//! Configuration-phase sync of mod-declared `sync(true)` static custom registries
//! (`sunrise:custom_registries`), matching NeoForge's `SyncRegistries`, `RegistryManager`
//! and `ClientPayloadHandler.handle(FrozenRegistrySyncCompletedPayload)`.
//!
//! Rules (exact NeoForge parity):
//! - Skipped on in-process / memory (single-player) connections
//!   (`ConfigurationInitialization.configureEarlyTasks:38-41`). LAN guests still sync.
//! - Skipped for vanilla / non-Sunrise clients (`NetworkInitialization.java:42-58`,
//!   `VanillaConnectionNetworkFilter.java:50,106`); vanilla clients are never disconnected
//!   for synced static registries.
//! - Local `sync(false)` registries never cross the wire (`RegistryManager.takeSnapshot:216-218`).
//! - On the client:
//!   - A server-sent registry absent on the client (or declared `sync(false)`) is ignored when
//!     its entry list is empty (`RegistryManager.applySnapshot:144-146`) and disconnects naming
//!     the registry when non-empty (`RegistryManager.applySnapshot:147-149`,
//!     `ClientPayloadHandler.java:114-118`).
//!   - Any server-sent entry ID missing from the client's registry disconnects naming the
//!     registry and the unknown entry IDs (`RegistryManager.applySnapshot:176-179`,
//!     `ClientPayloadHandler.java:104-110`).
//!   - Extra client-side entries or client-only registries do not disconnect
//!     (`ClientPayloadHandler.java:103`).
//! - When accepted, the client keeps the server's `(registry_id, raw_id -> Identifier)` table
//!   so subsequent `UpdateTags` packets decode custom-registry tags even when client and server
//!   registered entries in different orders.
use mod_api::Identifier;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Configuration custom-payload channel carrying the server's `sync(true)` static custom registries.
pub const CHANNEL: &str = "sunrise:custom_registries";

const MAGIC: &[u8; 4] = b"CRS1";
const HEADER: usize = 12;
const CHUNK_DATA: usize = 512 << 10;
const MAX_SNAPSHOT_BYTES: usize = mod_api::MAX_SNAPSHOT_BYTES;

/// One `sync(true)` static custom registry's ordered entry IDs on the wire (index = `raw_id`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRegistry {
    pub registry_id: String,
    pub entries: Vec<String>,
}

/// Complete configuration-phase snapshot of all `sync(true)` static custom registries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub registries: Vec<SnapshotRegistry>,
}

impl Snapshot {
    /// Collect all `sync(true)` registries from the current installed epoch, sorted by registry ID.
    pub fn from_installed() -> Self {
        let mut registries: Vec<SnapshotRegistry> =
            mod_api::__private::custom_registry_directories()
                .iter()
                .filter(|dir| dir.sync())
                .map(|dir| SnapshotRegistry {
                    registry_id: dir.id().to_owned(),
                    entries: dir.entry_ids().iter().map(ToString::to_string).collect(),
                })
                .collect();
        registries.sort_by(|a, b| a.registry_id.cmp(&b.registry_id));
        Self { registries }
    }

    /// Encode in length-prefixed binary (`u32` counts and UTF-8 string lengths, little-endian).
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let count = u32::try_from(self.registries.len())
            .map_err(|_| "Custom registry snapshot has too many registries.".to_owned())?;
        out.extend_from_slice(&count.to_le_bytes());
        for reg in &self.registries {
            write_str(&mut out, &reg.registry_id)?;
            let entry_count = u32::try_from(reg.entries.len()).map_err(|_| {
                format!("Custom registry {} has too many entries.", reg.registry_id)
            })?;
            out.extend_from_slice(&entry_count.to_le_bytes());
            for entry in &reg.entries {
                write_str(&mut out, entry)?;
            }
        }
        if out.len() > MAX_SNAPSHOT_BYTES {
            return Err("Custom registry snapshot exceeds the 64 MiB transport limit.".into());
        }
        Ok(out)
    }

    /// Decode from length-prefixed binary.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = bytes;
        let registry_count = read_u32(&mut cursor)? as usize;
        let mut registries = Vec::with_capacity(registry_count.min(256));
        for _ in 0..registry_count {
            let registry_id = read_str(&mut cursor)?;
            let entry_count = read_u32(&mut cursor)? as usize;
            let mut entries = Vec::with_capacity(entry_count.min(4096));
            for _ in 0..entry_count {
                entries.push(read_str(&mut cursor)?);
            }
            registries.push(SnapshotRegistry {
                registry_id,
                entries,
            });
        }
        if !cursor.is_empty() {
            return Err("Trailing bytes in custom registry snapshot.".into());
        }
        Ok(Self { registries })
    }
}

fn write_str(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let len =
        u32::try_from(value.len()).map_err(|_| "Custom registry string too long.".to_owned())?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn read_u32(cursor: &mut &[u8]) -> Result<u32, String> {
    if cursor.len() < 4 {
        return Err("Truncated custom registry snapshot.".into());
    }
    let (head, rest) = cursor.split_at(4);
    *cursor = rest;
    Ok(u32::from_le_bytes(head.try_into().unwrap()))
}

fn read_str(cursor: &mut &[u8]) -> Result<String, String> {
    let len = read_u32(cursor)? as usize;
    if cursor.len() < len {
        return Err("Truncated custom registry snapshot string.".into());
    }
    let (head, rest) = cursor.split_at(len);
    *cursor = rest;
    std::str::from_utf8(head)
        .map(str::to_owned)
        .map_err(|_| "Invalid UTF-8 in custom registry snapshot.".into())
}

/// Split an encoded snapshot into bounded configuration payload fragments.
pub fn chunks(bytes: &[u8]) -> Result<impl Iterator<Item = Vec<u8>> + '_, String> {
    if bytes.is_empty() || bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err("Custom registry snapshot exceeds the 64 MiB transport limit.".into());
    }
    let total = bytes.len() as u32;
    Ok(bytes.chunks(CHUNK_DATA).enumerate().map(move |(i, part)| {
        let mut packet = Vec::with_capacity(HEADER + part.len());
        packet.extend_from_slice(MAGIC);
        packet.extend_from_slice(&total.to_le_bytes());
        packet.extend_from_slice(&((i * CHUNK_DATA) as u32).to_le_bytes());
        packet.extend_from_slice(part);
        packet
    }))
}

type Held = Mutex<Option<Arc<Snapshot>>>;
static SERVED: Held = Mutex::new(None);
#[cfg(any(test, feature = "test-publication-scope"))]
static TEST_CLIENT_REGISTRIES: Mutex<Option<Vec<Arc<dyn mod_api::__private::RegistryDirectory>>>> =
    Mutex::new(None);

/// Override the local registries validated by [`Receiver`] on client network threads during
/// loopback integration tests (never used outside test builds).
#[cfg(any(test, feature = "test-publication-scope"))]
pub fn set_test_client_registries(
    registries: Option<Vec<Arc<dyn mod_api::__private::RegistryDirectory>>>,
) {
    *TEST_CLIENT_REGISTRIES
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = registries;
}

fn client_local_registries() -> Vec<Arc<dyn mod_api::__private::RegistryDirectory>> {
    #[cfg(any(test, feature = "test-publication-scope"))]
    if let Some(override_dirs) = TEST_CLIENT_REGISTRIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return override_dirs;
    }
    mod_api::__private::custom_registry_directories().to_vec()
}

#[cfg(any(test, feature = "test-publication-scope"))]
thread_local! {
    static SCOPED_SERVED: std::cell::RefCell<Option<Option<Arc<Snapshot>>>> =
        const { std::cell::RefCell::new(None) };
}

/// Isolates this thread's custom-registry server publication for tests (ruling point 19).
#[cfg(any(test, feature = "test-publication-scope"))]
pub fn isolate_server_publication() -> impl Drop {
    struct Scope(
        Option<Option<Arc<Snapshot>>>,
        std::marker::PhantomData<std::rc::Rc<()>>,
    );
    impl Drop for Scope {
        fn drop(&mut self) {
            SCOPED_SERVED.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    Scope(
        SCOPED_SERVED.with(|slot| slot.replace(Some(None))),
        std::marker::PhantomData,
    )
}

/// Override the snapshot this server sends during configuration (primarily for tests).
pub fn serve(snapshot: Option<Arc<Snapshot>>) {
    #[cfg(any(test, feature = "test-publication-scope"))]
    if SCOPED_SERVED.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(local) = slot.as_mut() {
            *local = snapshot.clone();
            true
        } else {
            false
        }
    }) {
        return;
    }
    *SERVED.lock().unwrap_or_else(|e| e.into_inner()) = snapshot;
}

/// The explicitly served snapshot, if set.
pub fn served() -> Option<Arc<Snapshot>> {
    #[cfg(any(test, feature = "test-publication-scope"))]
    if let Some(local) = SCOPED_SERVED.with(|slot| slot.borrow().clone()) {
        return local;
    }
    SERVED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Build the `sunrise:custom_registries` payload for a connecting client during configuration.
///
/// Returns `Ok(None)` when:
/// - the connection is in-process (`is_memory == true`, matching NeoForge's
///   `ConfigurationInitialization.configureEarlyTasks:38-41`),
/// - the client is not a Sunrise client (`!sunrise_client`), or
/// - there are no `sync(true)` custom registries to sync.
pub fn server_payload(sunrise_client: bool, is_memory: bool) -> Result<Option<Vec<u8>>, String> {
    if !sunrise_client || is_memory {
        return Ok(None);
    }
    let snapshot = served().unwrap_or_else(|| Arc::new(Snapshot::from_installed()));
    if snapshot.registries.is_empty() {
        return Ok(None);
    }
    snapshot.encode().map(Some)
}

/// Accepted server `(registry_id -> [entry_id in server raw_id order])` table held on the client
/// connection (`Remote`), used to decode `UpdateTags` for synced custom static registries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncedCustomRegistries {
    registries: HashMap<String, Vec<Identifier>>,
}

impl SyncedCustomRegistries {
    /// Resolve a wire `raw_id` for `registry` to its canonical [`Identifier`].
    ///
    /// Uses the server-synced ID table when received over a network connection, and falls back
    /// to this process's frozen static registry order for in-process single-player connections
    /// (where configuration sync was skipped per `ConfigurationInitialization:38-41`).
    pub fn entry_id(&self, registry: &str, raw_id: i32) -> Option<Identifier> {
        let index = usize::try_from(raw_id).ok()?;
        if let Some(entries) = self.registries.get(registry) {
            return entries.get(index).cloned();
        }
        mod_api::__private::custom_registry_entry_id(registry, index as u32).cloned()
    }

    /// Resolve an entry's canonical ID to the server-synced wire `raw_id` (or local slot in SP).
    pub fn raw_id(&self, registry: &str, id: &str) -> Option<i32> {
        if let Some(entries) = self.registries.get(registry) {
            let index = entries.iter().position(|entry| entry.to_string() == id)?;
            return i32::try_from(index).ok();
        }
        let dir = mod_api::__private::custom_registry_directories()
            .iter()
            .find(|dir| dir.id() == registry)?;
        let index = dir
            .entry_ids()
            .iter()
            .position(|entry| entry.to_string() == id)?;
        i32::try_from(index).ok()
    }

    /// Whether `registry` was synced by the server.
    pub fn has_registry(&self, registry: &str) -> bool {
        self.registries.contains_key(registry)
    }

    /// All entry IDs synced for `registry`, if present.
    pub fn entries(&self, registry: &str) -> Option<&[Identifier]> {
        self.registries.get(registry).map(Vec::as_slice)
    }

    pub(crate) fn owned_heap_bytes(&self) -> usize {
        self.registries
            .iter()
            .map(|(key, values)| {
                key.capacity()
                    .saturating_add(values.capacity().saturating_mul(size_of::<Identifier>()))
            })
            .sum()
    }
}

/// Validate a decoded server snapshot against a client's local frozen custom registries,
/// matching NeoForge's `RegistryManager.applySnapshot` and `ClientPayloadHandler`.
pub fn validate_snapshot(
    snapshot: &Snapshot,
    local_registries: &[Arc<dyn mod_api::__private::RegistryDirectory>],
) -> Result<SyncedCustomRegistries, String> {
    let mut synced = HashMap::with_capacity(snapshot.registries.len());
    for server_reg in &snapshot.registries {
        let local = local_registries
            .iter()
            .find(|dir| dir.id() == server_reg.registry_id && dir.sync());
        let Some(local) = local else {
            // RegistryManager.applySnapshot (lines 144-149): an empty server registry absent
            // on the client is ignored; a non-empty one disconnects naming the registry.
            if server_reg.entries.is_empty() {
                continue;
            }
            return Err(format!(
                "Server requires custom registry {} which is not present on this client.",
                server_reg.registry_id
            ));
        };
        let mut unknown = Vec::new();
        let mut parsed_entries = Vec::with_capacity(server_reg.entries.len());
        for entry_id in &server_reg.entries {
            if !local.contains_id(entry_id) {
                unknown.push(entry_id.as_str());
                continue;
            }
            let parsed = Identifier::parse_namespaced(entry_id).ok_or_else(|| {
                format!(
                    "Server sent invalid entry id {entry_id:?} for custom registry {}.",
                    server_reg.registry_id
                )
            })?;
            parsed_entries.push(parsed);
        }
        // RegistryManager.applySnapshot (lines 176-179) & ClientPayloadHandler (lines 104-110):
        // collect unknown entries and disconnect naming the registry and unknown entry IDs.
        if !unknown.is_empty() {
            return Err(format!(
                "Server sent unknown entries for custom registry {}: {}",
                server_reg.registry_id,
                unknown.join(", ")
            ));
        }
        synced.insert(server_reg.registry_id.clone(), parsed_entries);
    }
    Ok(SyncedCustomRegistries { registries: synced })
}

/// Per-connection client receiver for `sunrise:custom_registries` during configuration.
#[derive(Debug, Default)]
pub struct Receiver {
    pending: Option<(usize, Vec<u8>)>,
    synced: SyncedCustomRegistries,
}

impl Receiver {
    pub fn accept_chunk(&mut self, packet: &[u8]) -> Result<(), String> {
        if let Some(bytes) = self.push_fragment(packet)? {
            let snapshot = Snapshot::decode(&bytes)?;
            let local = client_local_registries();
            self.synced = validate_snapshot(&snapshot, &local)?;
        }
        Ok(())
    }

    pub(crate) fn finish_transfer(&self) -> Result<(), String> {
        if self.pending.is_some() {
            Err("Incomplete custom registry snapshot at the end of configuration.".into())
        } else {
            Ok(())
        }
    }

    pub fn into_synced(self) -> SyncedCustomRegistries {
        self.synced
    }

    pub(crate) fn owned_heap_bytes(&self) -> usize {
        self.pending
            .as_ref()
            .map_or(0, |(_, buf)| buf.capacity())
            .saturating_add(self.synced.owned_heap_bytes())
    }

    fn push_fragment(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let previous = self.pending.take();
        let bad = || "Invalid or out-of-order custom registry snapshot fragment.".to_string();
        if packet.len() < HEADER || &packet[..4] != MAGIC {
            return Err(bad());
        }
        let total = u32::from_le_bytes(packet[4..8].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(packet[8..12].try_into().unwrap()) as usize;
        if total == 0 || total > MAX_SNAPSHOT_BYTES {
            return Err("Custom registry snapshot exceeds the 64 MiB transport limit.".into());
        }
        if offset >= total || packet.len() - HEADER != CHUNK_DATA.min(total - offset) {
            return Err(bad());
        }
        let mut bytes = if offset == 0 {
            if previous.is_some() {
                return Err(bad());
            }
            Vec::with_capacity(total)
        } else {
            let (expected, bytes) = previous.ok_or_else(bad)?;
            if total != expected || bytes.len() != offset {
                return Err(bad());
            }
            bytes
        };
        bytes.extend_from_slice(&packet[HEADER..]);
        if bytes.len() == total {
            Ok(Some(bytes))
        } else {
            self.pending = Some((total, bytes));
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    struct FakeDir {
        id: String,
        tag_dir: String,
        sync: bool,
        entries: Vec<Identifier>,
    }

    impl FakeDir {
        fn arc(
            id: &str,
            sync: bool,
            entries: &[&str],
        ) -> Arc<dyn mod_api::__private::RegistryDirectory> {
            let parsed = Identifier::parse_namespaced(id).unwrap();
            Arc::new(Self {
                id: id.to_owned(),
                tag_dir: format!("{}/{}", parsed.namespace(), parsed.path()),
                sync,
                entries: entries
                    .iter()
                    .map(|e| Identifier::parse_namespaced(e).unwrap())
                    .collect(),
            })
        }
    }

    impl mod_api::__private::RegistryDirectory for FakeDir {
        fn id(&self) -> &str {
            &self.id
        }
        fn sync(&self) -> bool {
            self.sync
        }
        fn entry_ids(&self) -> &[Identifier] {
            &self.entries
        }
        fn contains_id(&self, id: &str) -> bool {
            self.entries.iter().any(|e| e.to_string() == id)
        }
        fn tag_directory(&self) -> &str {
            &self.tag_dir
        }
        fn as_any(&self) -> &(dyn Any + Send + Sync) {
            self
        }
    }

    #[test]
    fn snapshot_encode_decode_roundtrip_and_neoforge_validation_rules() {
        let snapshot = Snapshot {
            registries: vec![
                SnapshotRegistry {
                    registry_id: "chemlib:gas".into(),
                    entries: vec!["boiler:steam".into(), "chemlib:oxygen".into()],
                },
                SnapshotRegistry {
                    registry_id: "chemlib:empty_reg".into(),
                    entries: vec![],
                },
            ],
        };
        let encoded = snapshot.encode().unwrap();
        assert_eq!(Snapshot::decode(&encoded).unwrap(), snapshot);

        // Client has chemlib:gas in DIFFERENT order + extra client-only entry + client-only registry,
        // and lacks the empty server registry -> accepted!
        let local = vec![
            FakeDir::arc(
                "chemlib:gas",
                true,
                &["chemlib:oxygen", "boiler:steam", "clientonly:neon"],
            ),
            FakeDir::arc("clientonly:extra", true, &["clientonly:a"]),
        ];
        let synced = validate_snapshot(&snapshot, &local).unwrap();
        assert_eq!(
            synced.entry_id("chemlib:gas", 0).unwrap().to_string(),
            "boiler:steam"
        );
        assert_eq!(
            synced.entry_id("chemlib:gas", 1).unwrap().to_string(),
            "chemlib:oxygen"
        );

        // Client missing non-empty registry -> disconnects naming the registry.
        let err = validate_snapshot(&snapshot, &[]).unwrap_err();
        assert!(err.contains("chemlib:gas"), "got {err}");

        // Client missing one server entry -> disconnects naming registry and unknown entry.
        let missing_entry = vec![FakeDir::arc("chemlib:gas", true, &["chemlib:oxygen"])];
        let err = validate_snapshot(&snapshot, &missing_entry).unwrap_err();
        assert!(
            err.contains("chemlib:gas") && err.contains("boiler:steam"),
            "got {err}"
        );

        // Client declared sync(false) for server's non-empty synced registry -> disconnects.
        let local_unsynced = vec![FakeDir::arc(
            "chemlib:gas",
            false,
            &["chemlib:oxygen", "boiler:steam"],
        )];
        let err = validate_snapshot(&snapshot, &local_unsynced).unwrap_err();
        assert!(err.contains("chemlib:gas"), "got {err}");
    }

    #[test]
    fn memory_and_vanilla_connections_skip_sync_payload() {
        let _guard = isolate_server_publication();
        serve(Some(Arc::new(Snapshot {
            registries: vec![SnapshotRegistry {
                registry_id: "chemlib:gas".into(),
                entries: vec!["chemlib:oxygen".into()],
            }],
        })));
        assert!(
            server_payload(true, true).unwrap().is_none(),
            "in-process memory connection skips sync"
        );
        assert!(
            server_payload(false, false).unwrap().is_none(),
            "vanilla client skips sync"
        );
        assert!(
            server_payload(true, false).unwrap().is_some(),
            "Sunrise TCP/LAN client gets sync"
        );
    }
}
