//! The manifest shared by the game and launcher, and recovery of interrupted game installs.
use crate::durable;
use crate::install_lock::lock_home;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::Path;

pub const MANIFEST: &str = "mod-browser.json";
// There is at most one in-flight install in a home: all browser writers share the home lock.
const PENDING: &str = ".mod-browser.pending.json";
// Old and new can come from different indexes. Each row stores the version label twice,
// so allow two index budgets per row, plus space for fixed fields and the journal envelope.
const MAX_PENDING: u64 = 4 * super::MAX_INDEX + (4 << 10);

/// The managed path for a content mod whose id has been validated by the registry.
pub fn content_file(id: &str) -> String {
    format!("{}/{id}.smod", crate::mod_list::DIR)
}

#[derive(Serialize, Deserialize)]
struct Pending {
    id: String,
    file: String,
    old: Option<Value>,
    new: Value,
}

/// Settle an interrupted game install under the home lock before reading the browser manifest.
/// Mutating callers must still hold their own guard across the entire read/modify/write.
pub fn reconcile(home: &Path) -> Result<(), String> {
    let _lock = lock_home(home).map_err(|e| format!("{}: {e}", home.display()))?;
    reconcile_locked(home)
}

fn reconcile_locked(home: &Path) -> Result<(), String> {
    let path = home.join(PENDING);
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    match fs::symlink_metadata(&path) {
        Ok(info) if !info.is_file() => return Err(error(&"not a regular file")),
        Ok(info) if info.len() > MAX_PENDING => return Err(error(&"pending entry too large")),
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(&e)),
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| file.take(MAX_PENDING + 1).read_to_end(&mut bytes))
        .map_err(|e| error(&e))?;
    if bytes.len() as u64 > MAX_PENDING {
        return Err(error(&"pending entry too large"));
    }
    let pending: Pending = serde_json::from_slice(&bytes).map_err(|e| error(&e))?;
    let sha256 = pending.new["checksum"]
        .as_str()
        .and_then(|hash| hash.strip_prefix("SHA-256:"))
        .ok_or_else(|| error(&"missing SHA-256"))?;
    let valid_row = |row: &Value| {
        row["source"] == "sunrise" && row["project"] == pending.id && row["file"] == pending.file
    };
    if !super::valid_id(&pending.id)
        || pending.file != content_file(&pending.id)
        || !valid_row(&pending.new)
        || !pending.new["version"].is_string()
        || !pending.old.as_ref().is_none_or(valid_row)
    {
        return Err(error(&"invalid pending entry"));
    }
    super::hex64(sha256).map_err(|e| error(&e))?;
    let target = home.join(&pending.file);
    let published = match fs::symlink_metadata(&target) {
        Ok(info) if info.is_file() && info.len() <= super::MAX_FILE => {
            super::sha256_file(&target)? == sha256
        }
        Ok(_) => false,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(format!("{}: {e}", target.display())),
    };
    if published {
        let mut list = Installed::read(home)?;
        // Compare parsed rows so formatting changes don't matter but metadata edits do.
        let current = list.position(&pending.id).map(|i| &list.entries[i]);
        if current == Some(&pending.new) {
            // The row may be visible before its rename is durable. This also handles a
            // completed Windows install whose sidecar deletion did not survive power loss.
            let manifest = home.join(MANIFEST);
            durable::sync_parent(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        } else if current == pending.old.as_ref()
            && list.taken_by(&pending.file, &pending.id).is_none()
        {
            // The process could have died after renaming but before syncing this directory.
            durable::sync_parent(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            list.record(pending.new);
            list.save(home)?;
        }
    }
    // With the old bytes, or unrelated bytes, keep the old metadata. The usual checks
    // still decide whether the bytes actually match it. This also clears completed installs.
    durable::remove(&path).map_err(|e| error(&e))
}

/// `mod-browser.json`, kept as JSON values so the launcher's entries this
/// game doesn't model (Modrinth resource packs, ported Java mods) are written
/// back as they were.
#[derive(Debug, Default)]
pub struct Installed {
    pub entries: Vec<Value>,
}

impl Installed {
    /// Loads a consistent snapshot, settling an interrupted install first.
    pub fn load(home: &Path) -> Result<Self, String> {
        let _lock = lock_home(home).map_err(|e| format!("{}: {e}", home.display()))?;
        reconcile_locked(home)?;
        Self::read(home)
    }

    fn read(home: &Path) -> Result<Self, String> {
        let path = home.join(MANIFEST);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        let entries = value["entries"]
            .as_array()
            .ok_or_else(|| format!("{}: entries must be an array", path.display()))?
            .clone();
        Ok(Self { entries })
    }

    /// Saves the list. Hold the home lock from before loading through this write.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let path = home.join(MANIFEST);
        let text = serde_json::to_vec_pretty(&serde_json::json!({ "entries": self.entries }))
            .map_err(|e| e.to_string())?;
        durable::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Publishes a checked, already-synced package and its browser row. The caller holds
    /// the home lock from before loading through publication, and owns the target path.
    pub fn publish(
        &mut self,
        home: &Path,
        id: &str,
        download: &Path,
        entry: Value,
    ) -> Result<(), String> {
        let pending = Pending {
            id: id.to_owned(),
            file: content_file(id),
            old: self.position(id).map(|i| self.entries[i].clone()),
            new: entry,
        };
        let target = home.join(&pending.file);
        let path = home.join(PENDING);
        let bytes = serde_json::to_vec(&pending).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_PENDING {
            return Err(format!("{}: pending entry too large", path.display()));
        }
        durable::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        durable::replace(download, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        self.record(pending.new);
        self.save(home)?;
        durable::remove(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The registry's entry for project `id`.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e["source"] == "sunrise" && e["project"] == id)
    }

    /// The title of another entry that owns `file`.
    pub fn taken_by(&self, file: &str, id: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e["file"] == file && !(e["source"] == "sunrise" && e["project"] == id))
            .map(|e| e["title"].as_str().unwrap_or("another mod").to_owned())
    }

    fn record(&mut self, entry: Value) {
        let id = entry["project"].clone();
        self.entries
            .retain(|e| !(e["source"] == "sunrise" && e["project"] == id));
        self.entries.push(entry);
    }

    /// The version of `id` installed from the registry whose file is still
    /// there; for a `sunrise-mods/<id>.smod` put there another way, its
    /// manifest's version.
    pub fn version_of(&self, home: &Path, id: &str) -> Option<String> {
        if let Some(e) = self.position(id).map(|i| &self.entries[i])
            && let (Some(version), Some(file)) = (e["version"].as_str(), e["file"].as_str())
            && home.join(file).exists()
        {
            return Some(version.to_owned());
        }
        let path = home.join(content_file(id));
        crate::Package::read(&path)
            .ok()
            .filter(|p| p.manifest.id == id)
            .map(|p| p.manifest.version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_publication_recovers_the_row_only_for_the_published_bytes() {
        let entry = |version: &str, bytes: &[u8]| {
            serde_json::json!({
                "source": "sunrise", "project": "ruby", "file": content_file("ruby"),
                "version": version, "checksum": format!("SHA-256:{}", super::super::sha256(bytes)),
            })
        };
        let old = entry("1", b"old");
        let new = entry("2", b"new");
        let mut edited = old.clone();
        edited["dependency"] = Value::Bool(true);
        for (bytes, current, expected) in [
            (b"new", &old, &new),
            (b"old", &old, &old),
            (b"bad", &old, &old),
            (b"new", &edited, &edited),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let guard = lock_home(home).unwrap();
            let manifest = home.join(MANIFEST);
            let backup = home.join("backup");
            let target = home.join(content_file("ruby"));
            let download = home.join("download.part");
            fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
            durable::write(&target, b"old").unwrap();
            durable::write(&download, b"new").unwrap();
            let mut list = Installed {
                entries: vec![old.clone()],
            };
            list.save(home).unwrap();

            // Make saving the manifest fail after the package rename has succeeded.
            fs::rename(&manifest, &backup).unwrap();
            fs::create_dir(&manifest).unwrap();
            assert!(list.publish(home, "ruby", &download, new.clone()).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"new");
            fs::remove_dir(&manifest).unwrap();
            fs::rename(&backup, &manifest).unwrap();
            if current != &old {
                Installed {
                    entries: vec![current.clone()],
                }
                .save(home)
                .unwrap();
            }
            if bytes != b"new" {
                // Also model a rename that did not reach disk, and bytes neither row expects.
                durable::write(&target, bytes).unwrap();
            }
            drop(guard);

            let expected_rows = std::slice::from_ref(expected);
            assert_eq!(Installed::load(home).unwrap().entries, expected_rows);
            assert_eq!(Installed::read(home).unwrap().entries, expected_rows);
            assert_eq!(fs::read(&target).unwrap(), bytes);
            assert!(!home.join(PENDING).exists());
        }
    }

    #[test]
    fn interrupted_publication_recovers_a_large_registry_title() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let _guard = lock_home(home).unwrap();
        let target = home.join(content_file("ruby"));
        fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
        let download = home.join("download.part");
        durable::write(&download, b"new").unwrap();
        let row = serde_json::json!({
            "source": "sunrise", "project": "ruby", "file": content_file("ruby"),
            "version": "1", "checksum": format!("SHA-256:{}", super::super::sha256(b"new")),
            "title": "an allowed title ".repeat(20_000),
        });
        let manifest = home.join(MANIFEST);
        fs::create_dir(&manifest).unwrap();
        assert!(
            Installed::default()
                .publish(home, "ruby", &download, row.clone())
                .is_err()
        );
        assert_eq!(fs::read(target).unwrap(), b"new");
        fs::remove_dir(&manifest).unwrap();
        assert_eq!(Installed::load(home).unwrap().entries, [row]);
        assert!(!home.join(PENDING).exists());
    }

    #[test]
    fn malformed_or_oversized_sidecars_cannot_change_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let manifest = home.join(MANIFEST);
        let state = br#"{"entries": []}"#;
        fs::write(&manifest, state).unwrap();
        let path = home.join(PENDING);
        let file = content_file("ruby");
        fs::create_dir(home.join(crate::mod_list::DIR)).unwrap();
        fs::write(home.join(&file), b"new").unwrap();
        let pending = Pending {
            id: "ruby".into(),
            file: file.clone(),
            old: None,
            new: serde_json::json!({
                "source": "sunrise", "project": "ruby", "file": file,
                "version": "1", "checksum": format!("SHA-256:{}", super::super::sha256(b"new")),
            }),
        };
        let mut oversized = serde_json::to_vec(&pending).unwrap();
        // Trailing JSON whitespace changes the size without making the intent invalid.
        oversized.resize(MAX_PENDING as usize + 1, b' ');
        for bytes in [b"not json".to_vec(), oversized] {
            fs::write(&path, bytes).unwrap();
            assert!(reconcile(home).is_err());
            assert!(path.is_file());
            assert_eq!(fs::read(&manifest).unwrap(), state);
        }
    }
}
