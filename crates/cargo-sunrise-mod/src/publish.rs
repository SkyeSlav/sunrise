//! `cargo sunrise-mod publish` prepares a local submission. It uploads nothing, opens no pull request,
//! and needs no registry access. An unsigned submission runs `build`, which may download the SDK,
//! toolchain and dependencies; a signed submission uses the package already built and signed.
//!
//! The output holds the entry, a new project's icon if supplied, the new package under its release name
//! and SHA-256, and `PULL_REQUEST.md`. It is not a complete registry checkout.
use crate::{
    bounded::{self, Symlinks},
    internals::atomic_write,
};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    Manifest,
    icon::{MAX_DIMENSION, MAX_ICON},
    index::{
        self, Kind,
        page::{self, ICON_SIDE, Image},
        registry::{self, ENTRIES, Entry, EntryVersion, TODO},
    },
    manifest::MANIFEST,
};

const USAGE: &str = "cargo sunrise-mod publish [--out <dir>] [--registry <checkout>] [--summary <text>] [--owner <login>]… [--categories a,b] [--tags a,b] [--changelog <text>] [--key <public key> --signature <file.minisig>] [--wasm]";

#[derive(Default)]
struct Options {
    out: Option<PathBuf>,
    registry: Option<PathBuf>,
    summary: Option<String>,
    owners: Vec<String>,
    categories: Vec<String>,
    tags: Vec<String>,
    changelog: String,
    key: Option<String>,
    signature: Option<PathBuf>,
    wasm: bool,
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--wasm" {
            o.wasm = true;
            continue;
        }
        let known = [
            "--out",
            "--registry",
            "--summary",
            "--owner",
            "--categories",
            "--tags",
            "--changelog",
            "--key",
            "--signature",
        ];
        if !known.contains(&arg.as_str()) {
            return Err(format!(
                "publish does not take {arg:?}: see the options in `{USAGE}`"
            ));
        }
        let value = it
            .next()
            .ok_or_else(|| format!("{arg} needs a value: `{USAGE}`"))?;
        match arg.as_str() {
            "--out" => o.out = Some(value.into()),
            "--registry" => o.registry = Some(value.into()),
            "--summary" => o.summary = Some(value.clone()),
            "--owner" => o.owners.push(value.clone()),
            "--categories" => o.categories = list(value),
            "--tags" => o.tags = list(value),
            "--changelog" => o.changelog = value.clone(),
            "--key" => o.key = Some(value.clone()),
            _ => o.signature = Some(value.into()),
        }
    }
    Ok(o)
}

/// A submission icon must fit both the package and the registry page.
const PUBLISH_ICON_SIDE: std::ops::RangeInclusive<u32> = *ICON_SIDE.start()
    ..=if MAX_DIMENSION < *ICON_SIDE.end() {
        MAX_DIMENSION
    } else {
        *ICON_SIDE.end()
    };

fn read_icon(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let bytes = match bounded::read_bytes(path, MAX_ICON as u64, Symlinks::Refuse) {
        Ok(bytes) => bytes,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => {
            return Err(format!(
                "{e}: replace icon.png with a regular square PNG of at most {} KiB and run publish again",
                MAX_ICON >> 10
            ));
        }
    };
    if !matches!(page::image_size(&bytes), Some((Image::Png, w, h)) if w == h && PUBLISH_ICON_SIDE.contains(&w))
    {
        return Err(format!(
            "icon.png does not fit the package and registry limits: save it as a square PNG, {} to {} pixels a side, and run publish again",
            PUBLISH_ICON_SIDE.start(),
            PUBLISH_ICON_SIDE.end()
        ));
    }
    Ok(Some(bytes))
}

/// The version row's file name in the release, and the name a maintainer stages it under
/// (`<sha256>.<extension>`, which `scripts/fetch_files.py` of the registry looks for).
fn release_name(id: &str, version: &str, extension: &str) -> String {
    format!("{id}-{version}.{extension}")
}

fn staged_name(sha256: &str, file: &str) -> String {
    let extension = file.rsplit_once('.').map_or("", |(_, e)| e);
    format!("{sha256}.{extension}")
}

/// `YYYY-MM-DD` of a day count since 1970-01-01.
fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    date(i64::try_from(secs / 86_400).unwrap_or(0))
}

fn todo(what: &str) -> String {
    format!("{TODO}: {what}")
}

/// The new entry for a project's first version, filled from its manifest; what the manifest cannot say
/// is the options' or a [`TODO`] value that the registry's checks refuse.
fn draft(
    m: &sunrise_mod_package::Manifest,
    kind: Kind,
    o: &Options,
    icon: bool,
    version: EntryVersion,
) -> Entry {
    let or_todo = |value: &str, what: &str| {
        if value.trim().is_empty() {
            todo(what)
        } else {
            value.trim().to_owned()
        }
    };
    Entry {
        id: m.id.clone(),
        kind,
        repo: None,
        name: m.name.clone(),
        summary: o
            .summary
            .clone()
            .unwrap_or_else(|| todo("one line, at most 160 characters (--summary)")),
        description: or_todo(
            &m.description,
            "50 to 10,000 characters: set `description` in sunrise-mod.toml",
        ),
        authors: m.authors.clone(),
        owners: if o.owners.is_empty() {
            vec![todo("your GitHub login (--owner)")]
        } else {
            o.owners.clone()
        },
        owner_ids: Vec::new(),
        license: or_todo(
            &m.license,
            "an SPDX licence: set `license` in sunrise-mod.toml",
        ),
        source: or_todo(
            m.homepage.as_deref().unwrap_or(""),
            "the project's repository: set `homepage` in sunrise-mod.toml",
        ),
        icon: icon.then(|| format!("icons/{}.png", m.id)),
        categories: if o.categories.is_empty() {
            vec![todo("1 to 3 of the kind's categories (--categories)")]
        } else {
            o.categories.clone()
        },
        tags: o.tags.clone(),
        screenshots: Vec::new(),
        key: o.key.clone(),
        versions: vec![version],
    }
}

fn pull_request(entry: &Entry, v: &EntryVersion, size: u64, first: bool) -> (String, String) {
    let id = &entry.id;
    let title = if first {
        format!("Add {} {}", id, v.version)
    } else {
        format!("{id} {}", v.version)
    };
    let signed = if v.signature.is_some() {
        "signed (the signature is in the entry)"
    } else {
        "unsigned"
    };
    let submitted = if first && entry.icon.is_some() {
        "entry and icon are"
    } else {
        "entry is"
    };
    let body = format!(
        "{title}\n\n\
         - Project: `{id}` ({}), version {}, for Minecraft {}\n\
         - File: `{}`, {size} bytes, {signed}\n\
         - Artifact URL: {TODO}: paste the address of `{}` (a release asset of your repository works)\n\
         - SHA-256: `{}`\n\n\
         The {submitted} in this pull request; the file's facts (API, side, capabilities) are read from the file.\n\n\
         For the maintainer: copy the file from the artifact URL to this repository's `staging` release as `{}`, check that its SHA-256 is the one above, and let the `check` workflow run. After the merge, `publish` releases it as `{id}-{}`.\n",
        entry.kind.name(),
        v.version,
        v.game,
        v.file,
        v.file,
        v.sha256,
        staged_name(&v.sha256, &v.file),
        v.version,
    );
    (title, body)
}

pub fn run(entry_point: mod_surface::child::ChildEntry, args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let root = Path::new(".");
    let manifest = Manifest::parse(&bounded::read(&root.join(MANIFEST), bounded::MAX_MANIFEST)?)?;
    let id = &manifest.id;
    let release_version = manifest.version.to_string();
    if !registry::plain_version(&release_version) {
        return Err(format!(
            "{MANIFEST}: version {:?} cannot name a registry release: set `version` to at most 64 ASCII letters, digits, '.', '-' or '_', starting with a letter or digit",
            manifest.version
        ));
    }
    let kind = if manifest.native.is_some() {
        Kind::NativeMod
    } else {
        Kind::Mod
    };
    let existing = if let Some(checkout) = &o.registry {
        let loaded = registry::load(checkout).map_err(|errors| format!(
            "--registry {} is not a valid registry checkout: {}. Restore the checked-in entries or use a clean checkout",
            checkout.display(),
            errors.join("; ")
        ))?;
        // The registry loader has bound every entry's id to its file name; select this project's id.
        loaded.entries.into_iter().find(|e| &e.id == id)
    } else {
        None
    };
    if let Some(entry) = &existing {
        if let Some(repo) = &entry.repo {
            return Err(format!(
                "{id} follows the releases of {repo}: publish a release there; the registry takes no file for it"
            ));
        }
        if entry.kind != kind {
            return Err(format!(
                "{id} is listed as a {}, and a project's kind never changes: build a {} instead",
                entry.kind.name(),
                entry.kind.name()
            ));
        }
        if let Some(listed) = entry.versions.iter().find(|v| {
            sunrise_mod_package::manifest::parse_version(&v.version)
                .is_ok_and(|version| version == manifest.version)
        }) {
            return Err(format!(
                "{id} {} is already listed in the registry, and a published version never changes: raise `version` in {MANIFEST} and run publish again",
                listed.version
            ));
        }
        if entry.key.is_some() && (o.key.is_none() || o.signature.is_none()) {
            return Err(format!(
                "{id} has a key, so every version must be signed: use `cargo sunrise-mod build`, sign the package, then pass --key and --signature to publish"
            ));
        }
        if o.key.as_deref().map(str::trim) != entry.key.as_deref().map(str::trim) {
            return Err(format!(
                "{id} cannot change its signing key: pass the --key its entry names, or omit --key and --signature if it has none"
            ));
        }
    }
    if o.key.is_some() != o.signature.is_some() {
        return Err(
            "--key and --signature go together: supply both to publish a signed package".into(),
        );
    }
    let signature = o
        .signature
        .as_ref()
        .map(|path| {
            bounded::read(path, 16 << 10).map_err(|e| {
                format!("{e}: pass the .minisig produced by minisign -S to --signature")
            })
        })
        .transpose()?;
    let icon = read_icon(&root.join("icon.png"))?;
    let first = existing.is_none();
    // Remove trailing slashes and `.` before symlink_metadata; both could hide a symlink.
    let out: PathBuf = o
        .out
        .as_deref()
        .unwrap_or_else(|| Path::new("target/sunrise-mod/publish"))
        .components()
        .collect();
    if out.as_os_str().is_empty() {
        return Err("--out is empty: name a folder for the submission".into());
    }
    let mods = out.join(ENTRIES);
    let files = out.join("files");
    let icons = (first && icon.is_some()).then(|| out.join("icons"));
    // An unsigned submission is built first, into the project's own output folder.
    let built = signature
        .is_none()
        .then(|| [Path::new("target"), Path::new("target/sunrise-mod")]);
    // Refuse planted directory links before creating any output; the file replacements below
    // can safely replace links at the leaf itself.
    for dir in out.ancestors().filter(|p| !p.as_os_str().is_empty()).chain(
        [
            Some(mods.as_path()),
            Some(files.as_path()),
            icons.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(built.into_iter().flatten()),
    ) {
        match fs::symlink_metadata(dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(format!(
                    "{} is not a real directory: publish writes only into real folders; replace it with one, or choose a --out folder without symlinks",
                    dir.display()
                ));
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "{}: {e}; choose a writable --out folder",
                    dir.display()
                ));
            }
        }
    }
    if let Some(checkout) = &o.registry
        && let Ok(out_path) = dunce::canonicalize(&out)
        && dunce::canonicalize(checkout).is_ok_and(|base_path| base_path == out_path)
    {
        return Err("--out is the registry checkout: choose a separate output folder so the original entry stays unchanged".into());
    }

    let package = if signature.is_some() {
        root.join("target/sunrise-mod").join(format!("{id}.smod"))
    } else {
        let args: Vec<String> = o.wasm.then(|| "--wasm".to_owned()).into_iter().collect();
        crate::build_command(entry_point, &args)?
    };
    let package_bytes = bounded::read_bytes(&package, index::MAX_FILE, Symlinks::Refuse).map_err(|e| {
        format!("{e}: build a regular .smod within the registry's size limit with `cargo sunrise-mod build` and sign it again if needed")
    })?;
    let report = crate::check::check(
        &package,
        kind.clone(),
        o.key.as_deref(),
        signature.as_deref(),
    )
    .map_err(|e| {
        format!(
            "the package {} fails its check: {e}; rebuild it and sign the rebuilt file if needed",
            package.display()
        )
    })?;
    if report.facts.id.as_deref() != Some(id)
        || report.facts.version.as_deref() != Some(release_version.as_str())
    {
        return Err(format!(
            "{} does not contain {id} {} from {MANIFEST}: build that version and sign the resulting package before publishing",
            package.display(),
            manifest.version
        ));
    }
    if index::sha256(&package_bytes) != report.sha256 {
        return Err(format!(
            "{} changed during the check: finish building and signing before running publish again",
            package.display()
        ));
    }
    let file = release_name(id, &release_version, "smod");
    let version = EntryVersion {
        native_binaries: Vec::new(),
        version: release_version,
        published: today(),
        game: report
            .facts
            .game
            .unwrap_or_else(|| sunrise_mod_package::sdk::GAME.to_owned()),
        file: file.clone(),
        sha256: report.sha256.clone(),
        signature,
        changelog: o.changelog.clone(),
    };
    let entry = match existing {
        Some(mut entry) => {
            entry.versions.insert(0, version.clone());
            entry
        }
        None => draft(&manifest, kind, &o, icon.is_some(), version.clone()),
    };
    let entry_text = entry.to_toml()?;
    let (title, body) = pull_request(&entry, &version, report.size, first);
    for dir in [Some(&mods), Some(&files), icons.as_ref()]
        .into_iter()
        .flatten()
    {
        fs::create_dir_all(dir)
            .map_err(|e| format!("{}: {e}; choose a writable --out folder", dir.display()))?;
    }
    let write = |path: PathBuf, bytes: &[u8]| {
        atomic_write(&path, bytes).map_err(|e| format!("{e}; choose a writable --out folder"))
    };
    write(mods.join(format!("{id}.toml")), entry_text.as_bytes())?;
    write(files.join(&version.sha256), &package_bytes)?;
    write(out.join(&file), &package_bytes)?;
    if let (Some(dir), Some(bytes)) = (&icons, &icon) {
        write(dir.join(format!("{id}.png")), bytes)?;
    }
    write(out.join("PULL_REQUEST.md"), body.as_bytes())?;

    let mut remaining = entry.check_fields();
    remaining.extend(
        page::check_required(
            &entry.id,
            &entry.description,
            &entry.categories,
            entry.icon.as_deref(),
            &entry.screenshots,
        )
        .into_iter()
        .map(|p| format!("{}: {p}", entry.id)),
    );
    // A category that is still the TODO is already listed as such.
    remaining.retain(|r| !r.contains("is not a category") || !r.contains(TODO));
    println!(
        "{} {} ({}, {} bytes, SHA-256 {})",
        entry.id,
        version.version,
        entry.kind.name(),
        report.size,
        report.sha256
    );
    println!("Wrote in {}:", out.display());
    println!("  {ENTRIES}/{}.toml   the registry entry", entry.id);
    println!("  {file}   the package, to attach to a release of your repository");
    println!(
        "  files/{}   the same file, named as `registry check --files` reads it",
        report.sha256
    );
    if first && icon.is_some() {
        println!("  icons/{}.png   your icon.png", entry.id);
    }
    println!("  PULL_REQUEST.md   the pull request text, below");
    if remaining.is_empty() {
        println!("\nNothing is left to fill in.");
    } else {
        println!("\nStill to fill in before the pull request:");
        for r in &remaining {
            println!("  - {r}");
        }
    }
    // Prefix the package path so even --out values starting with '-' are positional.
    // Use single-quoted arguments for the POSIX shell (PowerShell on Windows).
    let quote = |arg: &str| {
        let escape = if cfg!(windows) { "''" } else { "'\\''" };
        format!("'{}'", arg.replace('\'', escape))
    };
    let packaged = root.join(&out).join(&file);
    let signing = match (&o.key, &o.signature) {
        (Some(key), Some(signature)) => format!(
            " --key {} --signature {}",
            quote(key),
            quote(&signature.to_string_lossy()),
        ),
        _ => String::new(),
    };
    println!(
        "\nCheck the new file locally (registry CI checks the whole checkout):\n  cargo sunrise-mod check {}{signing}",
        quote(&packaged.to_string_lossy()),
    );
    println!("\nPull request: {title}\n\n{body}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12";

    fn manifest() -> sunrise_mod_package::Manifest {
        toml::from_str(
            "id = 'glowing_blocks'\nname = 'Glowing Blocks'\nversion = '0.2.0'\napi = '0.2'\ncapabilities = ['registries']\n\
             description = 'Blocks that glow in the dark, and the items to make them.'\nauthors = ['Ada']\nlicense = 'MIT'\nhomepage = 'https://github.com/ada/glowing_blocks'\n",
        )
        .unwrap()
    }

    fn row(version: &str) -> EntryVersion {
        EntryVersion {
            native_binaries: Vec::new(),
            version: version.into(),
            published: "2026-11-18".into(),
            game: "26.3".into(),
            file: format!("glowing_blocks-{version}.smod"),
            sha256: SHA.into(),
            signature: None,
            changelog: String::new(),
        }
    }

    #[test]
    fn the_entry_is_written_from_the_manifest_and_names_what_is_left() {
        let o = Options {
            owners: vec!["ada".into()],
            categories: vec!["decoration".into()],
            ..Options::default()
        };
        let entry = draft(&manifest(), Kind::Mod, &o, true, row("0.2.0"));
        let text = entry.to_toml().unwrap();
        // The registry reads back exactly what was written.
        assert_eq!(toml::from_str::<Entry>(&text).unwrap(), entry);
        assert_eq!(entry.name, "Glowing Blocks");
        assert_eq!(entry.source, "https://github.com/ada/glowing_blocks");
        assert_eq!(entry.icon.as_deref(), Some("icons/glowing_blocks.png"));
        assert_eq!(entry.versions[0].sha256, SHA);
        assert_eq!(entry.versions[0].file, "glowing_blocks-0.2.0.smod");
        let left = entry.check_fields();
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(
            left[0].contains(TODO) && left[0].contains("summary"),
            "{left:?}"
        );
    }

    #[test]
    fn the_staging_name_is_the_hash_and_the_extension() {
        assert_eq!(
            staged_name(SHA, "glowing_blocks-0.2.0.smod"),
            format!("{SHA}.smod")
        );
        assert_eq!(release_name("m", "1.0.0", "smod"), "m-1.0.0.smod");
    }

    #[test]
    fn an_icon_must_fit_both_the_package_and_the_registry() {
        let work = tempfile::tempdir().unwrap();
        let icon = work.path().join("icon.png");
        // Only the PNG header is needed for the dimension check; build validates the pixel data.
        let png = |side: u32| {
            let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            b.extend(side.to_be_bytes());
            b.extend(side.to_be_bytes());
            b
        };
        for (side, accepted) in [(63, false), (64, true), (512, true), (513, false)] {
            fs::write(&icon, png(side)).unwrap();
            assert_eq!(read_icon(&icon).is_ok(), accepted, "side {side}");
        }
        fs::write(&icon, vec![0; MAX_ICON + 1]).unwrap();
        assert!(matches!(
            bounded::read_bytes(&icon, MAX_ICON as u64, Symlinks::Refuse),
            Err(bounded::ReadError::TooLarge(..))
        ));
        fs::remove_file(&icon).unwrap();
        fs::create_dir(&icon).unwrap();
        assert!(matches!(
            bounded::read_bytes(&icon, MAX_ICON as u64, Symlinks::Refuse),
            Err(bounded::ReadError::NotRegular(..))
        ));
    }

    #[test]
    fn dates_count_from_1970() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(19_723), "2024-01-01");
        assert_eq!(date(19_782), "2024-02-29");
    }
}
