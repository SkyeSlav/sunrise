use super::*;
use sunrise_mod_package::Package;

struct Fixture {
    root: tempfile::TempDir,
    index: Index,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new(extra: &[(&str, &[u8])]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let manifest = format!(
            "id='library'\nname='Library'\nversion='1.2.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'",
            "a".repeat(40)
        );
        let mut files: Vec<(&str, &[u8])> = vec![
            ("Cargo.toml", b"[package]\nname='library'\nversion='1.2.0'"),
            ("src/lib.rs", b"pub fn value() -> u32 { 20 }"),
        ];
        files.extend_from_slice(extra);
        let bytes = Package::write_files(&manifest, &files).unwrap();
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
            files: Some(self.root.path()),
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
    let fixture = Fixture::new(&[]);
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
    let mut fixture = Fixture::new(&[]);
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

#[test]
fn staging_refuses_nonportable_resource_paths_on_both_download_and_cache_hit() {
    let name = "assets/library/notes./data.txt";
    let fixture = Fixture::new(&[(name, b"resource bytes")]);
    fs::write(fixture.archive(), &fixture.bytes).unwrap();
    let row = &fixture.index.projects[0].versions[0];
    let kept = fixture
        .root
        .path()
        .join("cache/registry/library/1.2.0")
        .join(&row.file.sha256);
    let error = fixture.fetch().err().unwrap();
    assert!(
        error.contains(name) && error.contains("trailing dot"),
        "{error}"
    );
    assert!(!kept.exists());

    let staged = kept.join("source");
    fs::create_dir_all(&staged).unwrap();
    fs::write(kept.join("package.smod"), &fixture.bytes).unwrap();
    let bundle = Bundle::read(&fixture.archive()).unwrap();
    fs::write(staged.join("Cargo.toml"), &bundle.files["Cargo.toml"]).unwrap();
    let error = fixture.fetch().err().unwrap();
    assert!(
        error.contains(name) && error.contains("trailing dot"),
        "{error}"
    );
}
