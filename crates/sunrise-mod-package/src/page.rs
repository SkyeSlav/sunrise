//! Registry release metadata, shipped separately from the `.smod`.
use crate::{
    Manifest,
    checkout::read_regular,
    manifest::{MAX_MANIFEST, plain_path},
};
use serde::Deserialize;
use std::{io::ErrorKind, path::Path};

/// The page manifest beside `sunrise-mod.toml`.
pub const PAGE_FILE: &str = "page.toml";
/// Modrinth's gallery upload limit (`apps/labrinth/src/routes/internal/globals.rs`).
pub const MAX_SCREENSHOT_BYTES: u64 = 5 * 1024 * 1024;
// Modrinth's minimums and category cap (`apps/labrinth/src/validate/project/{summary,description,tags}.rs`).
const MIN_SUMMARY: usize = 25;
const MIN_DESCRIPTION: usize = 125;
const MAX_CATEGORIES: usize = 8;

/// The metadata shown on a project's registry page, with the manifest defaults applied.
#[derive(Debug)]
pub struct Page {
    pub summary: String,
    pub description: String,
    pub categories: Vec<String>,
    pub screenshots: Vec<Screenshot>,
    pub name: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    pub issues: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageToml {
    summary: String,
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    screenshots: Vec<Screenshot>,
    name: Option<String>,
    authors: Option<Vec<String>>,
    homepage: Option<String>,
    issues: Option<String>,
}

impl Page {
    /// Parses `page.toml`, rejecting unknown fields and invalid screenshot paths. Releasing
    /// also requires [`Self::release_problems`] to be empty and every named image to be valid.
    pub fn parse(text: &str, manifest: &Manifest) -> Result<Self, String> {
        let page: PageToml = toml::from_str(text).map_err(|e| format!("{PAGE_FILE}: {e}"))?;
        for (field, link) in [("homepage", &page.homepage), ("issues", &page.issues)] {
            if let Some(link) = link
                && !link.starts_with("https://")
                && !link.starts_with("http://")
            {
                return Err(format!("{PAGE_FILE}: {field} must be an http(s) link"));
            }
        }
        Ok(Self {
            summary: page.summary,
            description: page.description,
            categories: page.categories,
            screenshots: page.screenshots,
            name: page.name.unwrap_or_else(|| manifest.name.clone()),
            authors: page.authors.unwrap_or_else(|| manifest.authors.clone()),
            homepage: page.homepage.or_else(|| manifest.homepage.clone()),
            issues: page.issues.or_else(|| manifest.issues.clone()),
        })
    }

    /// Reads the page and its original TOML bytes for bundling. `None` means it does not exist.
    pub fn read(root: &Path, manifest: &Manifest) -> Result<Option<(Self, Vec<u8>)>, String> {
        let bytes = match read_regular(root, Path::new(PAGE_FILE), MAX_MANIFEST) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{PAGE_FILE}: {e}")),
        };
        let text = std::str::from_utf8(&bytes).map_err(|e| format!("{PAGE_FILE}: {e}"))?;
        Ok(Some((Self::parse(text, manifest)?, bytes)))
    }

    /// Lists release prerequisites not met by the metadata. `None` skips category membership
    /// when the caller has no registry list. Check the named files separately with [`Screenshot::read`]
    /// for a checkout or [`Screenshot::check_image`] for an archive.
    pub fn release_problems(&self, categories: Option<&Categories>) -> Vec<String> {
        let mut problems = Vec::new();
        for (field, text, min) in [
            ("summary", &self.summary, MIN_SUMMARY),
            ("description", &self.description, MIN_DESCRIPTION),
        ] {
            if text.trim().chars().count() < min {
                problems.push(format!(
                    "{PAGE_FILE}: {field} needs at least {min} characters"
                ));
            }
        }
        if self.screenshots.is_empty() {
            problems.push(format!(
                "{PAGE_FILE}: add at least one [[screenshots]] entry"
            ));
        }
        if self.categories.len() > MAX_CATEGORIES {
            problems.push(format!(
                "{PAGE_FILE}: choose at most {MAX_CATEGORIES} categories"
            ));
        }
        if let Some(categories) = categories {
            for id in &self.categories {
                if !categories.category.iter().any(|row| row.id == *id) {
                    problems.push(format!("{PAGE_FILE}: unknown registry category {id:?}"));
                }
            }
        }
        problems
    }
}

/// One image for the registry page. The path uses forward slashes and stays below `page/`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    #[serde(deserialize_with = "screenshot_path")]
    path: String,
    pub caption: Option<String>,
}

fn screenshot_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error;
    let path = String::deserialize(d)?;
    if !path.starts_with("page/") || !plain_path(&path) || path.contains('\0') {
        return Err(D::Error::custom(format!(
            "{path:?}: expected a relative screenshot file under page/ without . or .. components"
        )));
    }
    // Win32 may strip trailing dots or spaces when opening a path component.
    if path.split('/').any(|part| part.ends_with(['.', ' '])) {
        return Err(D::Error::custom(format!(
            "{path:?}: screenshot path components cannot end in a dot or space"
        )));
    }
    // Release check must also refuse paths that cannot be stored in the required bundle format.
    tar::Header::new_ustar()
        .set_path(&path)
        .map_err(|e| D::Error::custom(format!("{path:?}: not a ustar path: {e}")))?;
    Ok(path)
}

impl Screenshot {
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Checks PNG and JPEG signatures, regardless of the file's extension.
    pub fn check_image(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(b"\xff\xd8\xff") {
            Ok(())
        } else {
            Err(format!("{}: not a PNG or JPEG screenshot", self.path))
        }
    }

    /// Reads and checks the image within [`MAX_SCREENSHOT_BYTES`], without following links.
    pub fn read(&self, root: &Path) -> Result<Vec<u8>, String> {
        let bytes = read_regular(root, Path::new(&self.path), MAX_SCREENSHOT_BYTES)
            .map_err(|e| format!("{}: {e}", self.path))?;
        self.check_image(&bytes)?;
        Ok(bytes)
    }
}

/// The category catalog published by the registry as `registry/categories.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Categories {
    pub category: Vec<Category>,
}

/// One registry category, its Lucide icon and the project kinds it applies to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub icon: String,
    pub kinds: Vec<String>,
}

impl Categories {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("categories.toml: {e}"))
    }
}

#[cfg(test)]
mod tests;
