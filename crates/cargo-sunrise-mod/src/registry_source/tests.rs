use super::*;
use sunrise_mod_package::Package;

struct Fixture {
    root: tempfile::TempDir,
    index: Index,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let manifest = format!(
            "id='library'\nname='Library'\nversion='1.2.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'",
            "a".repeat(40)
        );
        let bytes = Package::write_files(
            &manifest,
            &[
                ("Cargo.toml", b"[package]\nname='library'\nversion='1.2.0'"),
                ("src/lib.rs", b"pub fn value() -> u32 { 20 }"),
            ],
        )
        .unwrap();
        let index = Index::parse(&serde_json::json!({
            "schema": index::SCHEMA,
            "projects": [{"id": "library", "name": "Library", "summary": "Fixture", "kind": "native-mod", "license": "MIT", "source": "https://example.org/source",
                "versions": [{"version": "1.2.0", "api": format!("native-1/{}", "a".repeat(40)), "game": "26.3",
                "file": {"name": "library.smod", "url": "https://example.org/library.smod", "size": bytes.len(), "sha256": index::sha256(&bytes)}}]}]
        }).to_string()).unwrap();
        assert!(index.left_out.is_empty(), "{:?}", index.left_out);
        Self { root, index, bytes }
    }

    fn archive(&self) -> PathBuf {
        self.root
            .path()
            .join(&self.index.projects[0].versions[0].file.sha256)
    }

    fn fetch(&self) -> Result<Bundle, String> {
        let project = &self.index.projects[0];
        Registry {
            files: Some(self.root.path().to_path_buf()),
            ..Registry::default()
        }
        .native_source(
            &self.root.path().join("cache"),
            &self.index,
            project,
            &project.versions[0],
        )
    }
}

use std::path::PathBuf;

#[test]
fn registry_refuses_same_size_bytes_with_a_different_sha256() {
    let fixture = Fixture::new();
    let mut corrupt = fixture.bytes.clone();
    corrupt[0] ^= 1;
    fs::write(fixture.archive(), corrupt).unwrap();
    let error = fixture.fetch().err().unwrap();
    assert!(
        error.contains("library 1.2.0") && error.contains("SHA-256"),
        "{error}"
    );
}

#[test]
fn a_cached_source_is_reused_but_the_latest_index_can_revoke_its_hash() {
    let mut fixture = Fixture::new();
    fs::write(fixture.archive(), &fixture.bytes).unwrap();
    let downloaded = fixture.fetch().unwrap();
    fs::remove_file(fixture.archive()).unwrap();
    assert_eq!(fixture.fetch().unwrap().files, downloaded.files);
    fixture.index.revoked.push(index::Revoked {
        id: "library".into(),
        sha256: Some(index::sha256(&fixture.bytes)),
        reason: "Takedown fixture".into(),
        date: String::new(),
    });
    let error = fixture.fetch().err().unwrap();
    assert!(
        error.contains("taken down") && error.contains("Takedown fixture"),
        "{error}"
    );
}
