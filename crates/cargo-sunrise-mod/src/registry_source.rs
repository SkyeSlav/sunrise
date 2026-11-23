//! The reviewed source artifacts used by native builds and source releases.
use crate::bounded;
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};
use sunrise_mod_package::index::{self, Index, Project, Trust, Version, native::Bundle};

#[derive(Default)]
pub(crate) struct Registry {
    /// Tests read the files from a folder, named by SHA-256, instead of downloading them.
    #[cfg(test)]
    pub(crate) files: Option<std::path::PathBuf>,
    /// Source release jobs can require an author signature; dev builds also use reviewed unsigned rows.
    pub(crate) signed_only: bool,
}

impl Registry {
    pub(crate) fn read_index(&self) -> Result<Index, String> {
        let url =
            test_url("index.json").unwrap_or_else(|| sunrise_addresses::MOD_REGISTRY_INDEX.into());
        let mut raw = String::new();
        download(&url)?
            .take(index::MAX_INDEX + 1)
            .read_to_string(&mut raw)
            .map_err(|e| format!("{url}: {e}"))?;
        if raw.len() as u64 > index::MAX_INDEX {
            return Err(format!("{url}: index exceeds its size limit"));
        }
        Index::parse(&raw).map_err(|e| format!("{url}: {e}"))
    }

    /// Reuse the verified download and its unpacked source. A cache key is a file identity, not
    /// an eligibility decision: the current index's revocation and signature checks still run.
    pub(crate) fn native_source(
        &self,
        cache: &Path,
        index: &Index,
        project: &Project,
        row: &Version,
    ) -> Result<Bundle, String> {
        let failed = |why: String| format!("{} {}: {why}", project.id, row.version);
        // A selected mod version passed a semver constraint, and the index checked id and hash.
        let parent = cache.join("registry").join(&project.id).join(&row.version);
        let kept = parent.join(&row.file.sha256);
        if let Some(revoked) = index.revoked(&project.id, &row.file.sha256) {
            return Err(failed(format!("taken down: {}", revoked.reason)));
        }
        let checked = |dir: &Path| -> Result<Bundle, String> {
            let archive = dir.join("package.smod");
            let mut bundle = self.verify_native(index, project, row, &archive)?;
            // Only listed files enter staging. Refuse edits to the unpacked cache rather than
            // compiling bytes that weren't covered by the verified archive.
            for (name, expected) in &mut bundle.files {
                let path = dir.join("source").join(name);
                let bytes =
                    bounded::read_bytes(&path, expected.len() as u64, bounded::Symlinks::Refuse)?;
                if *expected != bytes {
                    return Err(format!(
                        "{} differs from the verified source; clear this registry cache entry",
                        path.display()
                    ));
                }
                *expected = bytes;
            }
            Ok(bundle)
        };
        if kept.is_dir() {
            return checked(&kept).map_err(failed);
        }
        fs::create_dir_all(&parent).map_err(|e| failed(e.to_string()))?;
        let pending = tempfile::tempdir_in(&parent).map_err(|e| failed(e.to_string()))?;
        let archive = pending.path().join("package.smod");
        #[cfg(test)]
        let folder = self.files.as_deref();
        #[cfg(not(test))]
        let folder: Option<&Path> = None;
        let input: Box<dyn Read> = if let Some(files) = folder {
            Box::new(
                fs::File::open(files.join(&row.file.sha256)).map_err(|e| failed(e.to_string()))?,
            )
        } else {
            let local = test_url(&format!("files/{}/{}", project.id, row.file.name));
            download(local.as_deref().unwrap_or(&row.file.url)).map_err(failed)?
        };
        io::copy(
            &mut input.take(row.file.size + 1),
            &mut fs::File::create(&archive).map_err(|e| failed(e.to_string()))?,
        )
        .map_err(|e| failed(e.to_string()))?;
        let bundle = self
            .verify_native(index, project, row, &archive)
            .map_err(failed)?;
        write_files(&pending.path().join("source"), &bundle.files).map_err(failed)?;
        // A concurrent invocation may have installed the same verified artifact first.
        if let Err(e) = fs::rename(pending.path(), &kept) {
            if !kept.is_dir() {
                return Err(failed(e.to_string()));
            }
            return checked(&kept).map_err(failed);
        }
        Ok(bundle)
    }

    fn verify_native(
        &self,
        index: &Index,
        project: &Project,
        row: &Version,
        archive: &Path,
    ) -> Result<Bundle, String> {
        if index::check_file(index, project, row, archive)? != Trust::Verified && self.signed_only {
            return Err("library source has no author signature".into());
        }
        let bundle = Bundle::read(archive)?;
        if row.dependencies.is_none() && !bundle.manifest.dependencies.is_empty() {
            return Err(
                "the index omits the library's dependencies; rebuild the registry index".into(),
            );
        }
        Ok(bundle)
    }
}

fn download(url: &str) -> Result<Box<dyn Read + Send + Sync>, String> {
    sunrise_proxy::get(url, &[], None)
        .map(|response| response.into_reader())
        .map_err(|e| format!("{url}: {e}"))
}

#[cfg(test)]
mod tests;

fn test_url(file: &str) -> Option<String> {
    // Test the HTTP path with a local fixture without changing the published index's URL rules.
    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("SUNRISE_MOD_REGISTRY_TEST_BASE") {
        return Some(format!("{}/{file}", base.trim_end_matches('/')));
    }
    let _ = file;
    None
}

/// Write checked, package-relative paths to a private, fresh directory. `create_new` detects
/// collisions on case-insensitive filesystems as well as exact duplicates.
pub(crate) fn write_files(
    directory: &Path,
    files: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<[u8]>)>,
) -> Result<(), String> {
    for (name, bytes) in files {
        let name = name.as_ref();
        #[cfg(windows)]
        if name.split('/').any(|part| {
            // Win32 strips trailing spaces/dots and can treat DOS device names as files.
            let stem = part
                .split('.')
                .next()
                .unwrap_or(part)
                .trim_end()
                .to_ascii_uppercase();
            part.ends_with([' ', '.'])
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || ["COM", "LPT"].into_iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|n| {
                        matches!(
                            n,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                })
        }) {
            return Err(format!("{name}: cannot stage a Windows-aliased path"));
        }
        let file = directory.join(name);
        let write = || -> io::Result<()> {
            fs::create_dir_all(file.parent().expect("file in the staging directory"))?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)?
                .write_all(bytes.as_ref())
        };
        write().map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}
