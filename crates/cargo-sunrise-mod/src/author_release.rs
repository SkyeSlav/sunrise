//! The author-side release flow. GitHub runs the shared workflow after the tag is pushed.
use crate::{internals, options::Options, read_manifest};
use mod_surface::child::ChildEntry;
use semver::Version;
use serde::Deserialize;
use std::{
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use sunrise_mod_package::{Manifest, manifest::MANIFEST};
use toml_edit::DocumentMut;

pub(crate) const USAGE: &str = "release [<version>|patch|minor|major] [--check] | release setup";
const WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml@release-v";
const DEFAULT_WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml@release-v1";
const CALLER: &str = ".github/workflows/sunrise-release.yml";
const NOT_REGISTERED: &str =
    "This repository isn't registered yet: register it at https://playsunrise.net/mods/submit";

type Fetch<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

fn fetch(url: &str) -> Result<Option<String>, String> {
    match sunrise_proxy::get(url, &[("User-Agent", "cargo-sunrise-mod")], None) {
        Ok(response) => response
            .into_string()
            .map(Some)
            .map_err(|e| format!("{url}: {e}")),
        Err(e) if e.status() == Some(404) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn registry_file(fetch: Fetch<'_>, name: &str) -> Result<String, String> {
    let url = format!("{}/{name}", sunrise_addresses::MOD_REGISTRY_METADATA);
    fetch(&url)?.ok_or_else(|| format!("{url}: not found"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Watched {
    #[serde(default)]
    repository: Vec<Repository>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Repository {
    id: u64,
    name: String,
    package: Vec<WatchedPackage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WatchedPackage {
    path: String,
    #[serde(rename = "project")]
    _project: String,
}

#[derive(Deserialize)]
struct GitHubRepository {
    id: u64,
}

fn github_name(origin: &str) -> Option<String> {
    let url;
    let path = if origin.contains("://") {
        url = url::Url::parse(origin).ok()?;
        if !matches!(url.scheme(), "https" | "ssh")
            || !url.host_str()?.eq_ignore_ascii_case("github.com")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        url.path().strip_prefix('/')?
    } else {
        let (authority, path) = origin.split_once(':')?;
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    let valid = |part: &str| {
        !matches!(part, "" | "." | "..")
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    };
    (valid(owner) && valid(repo)).then(|| path.to_owned())
}

fn registered(fetch: Fetch<'_>, origin: &str, path: &str) -> Result<(), String> {
    let name = github_name(origin).ok_or(NOT_REGISTERED)?;
    let watched: Watched = toml::from_str(&registry_file(fetch, "watched.toml")?)
        .map_err(|e| format!("registry/watched.toml: {e}"))?;
    let repo = if let Some(repo) = watched
        .repository
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&name))
    {
        repo
    } else {
        let url = format!("https://api.github.com/repos/{name}");
        let text = fetch(&url)?.ok_or(NOT_REGISTERED)?;
        let GitHubRepository { id } =
            serde_json::from_str(&text).map_err(|e| format!("{url}: {e}"))?;
        watched
            .repository
            .iter()
            .find(|r| r.id == id)
            .ok_or(NOT_REGISTERED)?
    };
    if !repo.package.iter().any(|p| p.path == path) {
        return Err(NOT_REGISTERED.into());
    }
    Ok(())
}

fn workflow(fetch: Fetch<'_>) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Workflow {
        uses: String,
    }
    let Workflow { uses } = toml::from_str(&registry_file(fetch, "release-workflow.toml")?)
        .map_err(|e| format!("registry/release-workflow.toml: {e}"))?;
    if !uses
        .strip_prefix(WORKFLOW)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "registry/release-workflow.toml: expected uses = \"{WORKFLOW}<number>\""
        ));
    }
    Ok(uses)
}

fn caller(prefix: &str, path: &str, uses: &str) -> String {
    // Ordinary paths stay plain YAML. Quote strings that YAML might coerce, strip or interpret.
    let path = if path
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"_.-/".contains(&c))
        && path.starts_with(|c: char| c.is_ascii_alphabetic() || c == '.')
        && !matches!(
            path.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        ) {
        path.into()
    } else {
        serde_json::json!(path).to_string()
    };
    format!(
        "\
# Written by `cargo sunrise-mod release`, which rewrites it: edits here are replaced.
name: Sunrise release
on:
  push:
    tags: ['{prefix}v*']
permissions: {{}}
jobs:
  release:
    permissions:
      contents: write
      id-token: write
      attestations: write
    uses: {uses}
    with:
      path: {path}
"
    )
}

pub(crate) fn scaffold(root: &Path) -> Result<(), String> {
    Change::write(CALLER, caller("", ".", DEFAULT_WORKFLOW)).apply(root)
}

/// A package and its caller are both addressed relative to the Git root.
struct Project {
    root: PathBuf,
    package: PathBuf,
    path: String,
    prefix: String,
    caller: PathBuf,
    manifest: Manifest,
}

impl Project {
    fn read(package: &Path, git: &Git) -> Result<Self, String> {
        let root = git.text(package, ["rev-parse", "--show-toplevel"])?;
        let canonical =
            |path: &Path| dunce::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()));
        let root = canonical(Path::new(&root))?;
        let package = canonical(package)?;
        let relative = package
            .strip_prefix(&root)
            .map_err(|_| "the package must be in the Git working tree")?;
        let mut pieces = Vec::new();
        for part in relative.iter() {
            pieces.push(part.to_str().ok_or("the package path must be UTF-8")?);
        }
        let manifest = read_manifest(&package)?;
        let (path, prefix, caller) = if pieces.is_empty() {
            (".".into(), String::new(), CALLER.into())
        } else {
            let path = pieces.join("/");
            if path.contains("${{") || path.contains('\\') {
                return Err(
                    "the package path cannot be passed literally to the release workflow".into(),
                );
            }
            let id = &manifest.id;
            (
                path,
                format!("{id}/"),
                format!(".github/workflows/sunrise-release-{id}.yml").into(),
            )
        };
        let package = relative.to_path_buf();
        Ok(Self {
            root,
            package,
            path,
            prefix,
            caller,
            manifest,
        })
    }

    fn workflow_changes(&self, uses: &str) -> Result<Vec<Change>, String> {
        let root = &self.root;
        for folder in [".github", ".github/workflows", ".github/actions"] {
            internals::check_output_dir(&root.join(folder))?;
        }
        let mut changes = vec![Change::write(
            &self.caller,
            caller(&self.prefix, &self.path, uses),
        )];
        let legacy = ".github/workflows/release.yml";
        if let Some(meta) = metadata(&root.join(legacy))?
            && meta.is_file()
        {
            let text =
                fs::read_to_string(root.join(legacy)).map_err(|e| format!("{legacy}: {e}"))?;
            // Both shipped revisions named these; a differently named release workflow is not ours.
            if text.contains("sunrise-release-tool") || text.contains("sunrise_release.py") {
                changes.push(Change {
                    path: legacy.into(),
                    kind: Kind::RemoveFile,
                });
            }
        }
        for (path, directory) in [
            (PathBuf::from(".github/workflows/rebuild.yml"), false),
            (".github/workflows/sunrise-release-jobs.yml".into(), false),
            (".github/actions/sunrise-release-tool".into(), true),
        ]
        .into_iter()
        .chain(
            ["sunrise_release.py", "sunrise_release_github.py"]
                .into_iter()
                .flat_map(|name| {
                    std::iter::once((PathBuf::from(name), false)).chain(
                        (!self.package.as_os_str().is_empty())
                            .then(|| (self.package.join(name), false)),
                    )
                }),
        ) {
            if let Some(meta) = metadata(&root.join(&path))? {
                let kind = if meta.is_symlink() || (!directory && meta.is_file()) {
                    Kind::RemoveFile
                } else if directory && meta.is_dir() {
                    Kind::RemoveDir
                } else {
                    return Err(format!(
                        "{} is not the expected legacy release file or directory",
                        path.display()
                    ));
                };
                changes.push(Change { path, kind });
            }
        }
        Ok(changes)
    }

    fn bump(&self, version: &Version, changes: &mut Vec<Change>) -> Result<(), String> {
        let version = version.to_string();
        let path = self.package.join(MANIFEST);
        let mut mod_doc = read_toml(&self.root.join(&path))?;
        internals::set_string(
            mod_doc["version"]
                .as_value_mut()
                .expect("parsed manifest version"),
            &version,
        );
        changes.push(Change::write(path, mod_doc.to_string()));
        if self.manifest.native.is_none() {
            return Ok(());
        }
        let cargo_path = self.package.join("Cargo.toml");
        let lock_path = self.package.join("Cargo.lock");
        let mut cargo = read_toml(&self.root.join(&cargo_path))?;
        let mut lock = read_toml(&self.root.join(&lock_path))?;
        let package = cargo
            .get_mut("package")
            .and_then(|p| p.as_table_like_mut())
            .ok_or("Cargo.toml: no [package] table")?;
        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Cargo.toml: no package name")?
            .to_owned();
        let value = package
            .get_mut("version")
            .and_then(|v| v.as_value_mut())
            .ok_or("Cargo.toml: [package] must specify a version")?;
        let old = value
            .as_str()
            .ok_or("Cargo.toml: [package] must specify a version string")?;
        let packages = lock
            .get_mut("package")
            .and_then(|p| p.as_array_of_tables_mut())
            .ok_or("Cargo.lock: no package entries")?;
        // A root package entry has no source. Update it without running Cargo's resolver or
        // rewriting the checksums/versions of the SDK and crates.io dependencies.
        {
            let mut entries = packages.iter_mut().filter(|p| {
                p.get("name").and_then(|v| v.as_str()) == Some(&name)
                    && p.get("version").and_then(|v| v.as_str()) == Some(old)
                    && !p.contains_key("source")
            });
            let own = entries.next().ok_or_else(|| format!("Cargo.lock: no local entry for {name} {old}; refresh Cargo.lock before releasing"))?;
            if entries.next().is_some() {
                return Err(format!(
                    "Cargo.lock: more than one local entry for {name} {old}"
                ));
            }
            internals::set_string(
                own["version"]
                    .as_value_mut()
                    .expect("matched version string"),
                &version,
            );
        }
        // Cargo qualifies a dependency by version when several locked packages share a name.
        let old_id = format!("{name} {old}");
        let new_id = format!("{name} {version}");
        for p in packages.iter_mut() {
            if let Some(deps) = p.get_mut("dependencies").and_then(|v| v.as_array_mut()) {
                for dep in deps.iter_mut().filter(|v| v.as_str() == Some(&old_id)) {
                    internals::set_string(dep, &new_id);
                }
            }
        }
        internals::set_string(value, &version);
        changes.push(Change::write(cargo_path, cargo.to_string()));
        changes.push(Change::write(lock_path, lock.to_string()));
        Ok(())
    }
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn read_toml(path: &Path) -> Result<DocumentMut, String> {
    if !metadata(path)?.is_some_and(|m| m.is_file()) {
        return Err(format!("{}: expected a regular file", path.display()));
    }
    fs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))
}

struct Change {
    path: PathBuf,
    kind: Kind,
}

enum Kind {
    Write(String),
    RemoveFile,
    RemoveDir,
}

impl Change {
    fn write(path: impl Into<PathBuf>, text: String) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Write(text),
        }
    }

    fn apply(&self, root: &Path) -> Result<(), String> {
        let path = root.join(&self.path);
        let result = match &self.kind {
            Kind::Write(text) => {
                let parent = path.parent().expect("path in checkout");
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                return internals::atomic_write(&path, text);
            }
            Kind::RemoveFile => fs::remove_file(&path),
            Kind::RemoveDir => fs::remove_dir_all(&path),
        };
        match result {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn describe(&self) {
        let verb = match self.kind {
            Kind::Write(_) => "Write",
            Kind::RemoveFile | Kind::RemoveDir => "Remove",
        };
        println!("  {verb} {}", self.path.display());
    }
}

#[derive(Default)]
struct Git {
    #[cfg(test)]
    env: Vec<(&'static str, std::ffi::OsString)>,
}

impl Git {
    fn command(&self, root: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(root);
        #[cfg(test)]
        cmd.envs(self.env.iter().map(|(key, value)| (*key, value)));
        cmd
    }

    fn text(
        &self,
        root: &Path,
        args: impl IntoIterator<Item: AsRef<OsStr>>,
    ) -> Result<String, String> {
        let mut cmd = self.command(root);
        cmd.args(args);
        let Output {
            status,
            stdout,
            stderr,
        } = cmd.output().map_err(|e| format!("git: {e}"))?;
        if !status.success() {
            let operation = cmd.get_args().next().unwrap_or_default().to_string_lossy();
            return Err(format!(
                "git {operation}: {}",
                String::from_utf8_lossy(&stderr).trim_end()
            ));
        }
        let stdout = String::from_utf8(stdout).map_err(|e| format!("git: {e}"))?;
        Ok(stdout
            .strip_suffix("\r\n")
            .or_else(|| stdout.strip_suffix('\n'))
            .unwrap_or(&stdout)
            .to_owned())
    }

    fn clean(&self, root: &Path) -> Result<(), String> {
        if !self
            .text(
                root,
                [
                    "--no-optional-locks",
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--ignore-submodules=none",
                ],
            )?
            .is_empty()
        {
            return Err(
                "the Git work tree has uncommitted changes; commit or stash them before releasing"
                    .into(),
            );
        }
        Ok(())
    }
}

fn next_version(current: &Version, requested: &str) -> Result<Version, String> {
    let bump = |part: u64| {
        part.checked_add(1)
            .ok_or("the version component cannot be incremented")
    };
    let next = match requested {
        "major" => Version::new(bump(current.major)?, 0, 0),
        "minor" => Version::new(current.major, bump(current.minor)?, 0),
        "patch" => Version::new(current.major, current.minor, bump(current.patch)?),
        text => Version::parse(text).map_err(|e| format!("release version {text:?}: {e}"))?,
    };
    if !next.cmp_precedence(current).is_gt() {
        return Err(format!(
            "release version {next} must be higher than {current}"
        ));
    }
    Ok(next)
}

pub(crate) fn run(entry: ChildEntry, root: &Path, args: &[String]) -> Result<(), String> {
    release(root, args, &Git::default(), &fetch, |root, tag| {
        crate::sdk_cmds::check_mod(entry, root, false, &[], Some(tag))
    })
}

fn release(
    package: &Path,
    args: &[String],
    git: &Git,
    fetch: Fetch<'_>,
    check: impl FnOnce(&Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    if args == ["setup"] {
        let project = Project::read(package, git)?;
        for change in project.workflow_changes(&workflow(fetch)?)? {
            change.apply(&project.root)?;
            change.describe();
        }
        return Ok(());
    }
    let mut options = Options::new("release", args, &[]).usage(USAGE);
    let preview = options.switch("--check");
    let requested = options.at_most_one()?.unwrap_or("patch");
    options.finish()?;
    let project = Project::read(package, git)?;
    let root = &project.root;
    git.clean(root)?;
    let branch = git
        .text(root, ["symbolic-ref", "--quiet", "HEAD"])
        .map_err(|_| "check out a branch before releasing")?;
    let head = git.text(root, ["rev-parse", "HEAD"])?;
    let origin = git.text(root, ["remote", "get-url", "origin"])?;
    registered(fetch, &origin, &project.path)?;
    let version = next_version(&project.manifest.version, requested)?;
    let tag = format!("{}v{version}", project.prefix);
    let tag_ref = format!("refs/tags/{tag}");
    if !git.text(root, ["tag", "--list", &tag])?.is_empty() {
        return Err(format!("tag {tag} already exists locally"));
    }
    let remote = git.text(
        root,
        ["ls-remote", "--refs", "--", "origin", &branch, &tag_ref],
    )?;
    let find_ref = |name: &str| {
        remote
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .find_map(|(oid, r)| (r == name).then_some(oid))
    };
    if find_ref(&tag_ref).is_some() {
        return Err(format!("tag {tag} already exists on origin"));
    }
    let upstream = git.text(
        root,
        [
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)",
            &branch,
        ],
    )?;
    let mut upstream_head = None;
    if let Some((remote_name, remote_ref)) = upstream
        .split_once('\0')
        .filter(|(name, r)| !name.is_empty() && !r.is_empty())
    {
        if remote_name != "origin" || remote_ref != branch {
            let upstream =
                git.text(root, ["ls-remote", "--refs", "--", remote_name, remote_ref])?;
            upstream_head = upstream.split_once('\t').map(|(oid, _)| oid.to_owned());
        }
    }
    for oid in [find_ref(&branch), upstream_head.as_deref()]
        .into_iter()
        .flatten()
    {
        let status = git
            .command(root)
            .args(["merge-base", "--is-ancestor", oid, &head])
            .output()
            .map_err(|e| format!("git merge-base: {e}"))?
            .status;
        if !status.success() {
            return Err("the branch is not up to date with its upstream or origin; fetch and integrate the remote branch before releasing".into());
        }
    }

    let mut changes = project.workflow_changes(&workflow(fetch)?)?;
    project.bump(&version, &mut changes)?;

    // The same check as CI sees the proposed version and tag, not the previous version.
    // Clone to a separate Git repository: the check can fetch an SDK and build native code,
    // without writing version files, build output or worktree metadata in the author's repo.
    let scratch = tempfile::tempdir().map_err(|e| format!("release check: {e}"))?;
    let snapshot = scratch.path().join("checkout");
    git.text(
        root,
        [
            OsStr::new("clone"),
            OsStr::new("--shared"),
            OsStr::new("--no-checkout"),
            OsStr::new("--"),
            root.as_os_str(),
            snapshot.as_os_str(),
        ],
    )?;
    git.text(&snapshot, ["checkout", "--detach", &head])?;
    for change in &changes {
        change.apply(&snapshot)?;
    }
    check(&snapshot.join(&project.package), &tag)?;
    if preview {
        println!("Would release {version}:");
        for change in changes {
            change.describe();
        }
        println!(
            "  Commit as Release {version}, tag {tag}, and push {branch} and {tag_ref} to origin"
        );
        return Ok(());
    }

    git.clean(root)?;
    if git.text(root, ["rev-parse", "HEAD"])? != head
        || git.text(root, ["symbolic-ref", "--quiet", "HEAD"])? != branch
    {
        return Err(
            "the checked-out branch changed during the release check; run release again".into(),
        );
    }
    // Stage only explicit changes. Ignore paths whose only change was removing an ignored legacy file.
    let mut tracked = Vec::new();
    for change in &changes {
        let path = change.path.as_os_str();
        if matches!(change.kind, Kind::Write(_))
            || !git
                .text(
                    root,
                    [
                        OsStr::new("--literal-pathspecs"),
                        OsStr::new("ls-files"),
                        OsStr::new("--"),
                        path,
                    ],
                )?
                .is_empty()
        {
            tracked.push(path);
        }
        change.apply(root)?;
    }
    git.text(
        root,
        ["--literal-pathspecs", "add", "-f", "-A", "--"]
            .map(OsStr::new)
            .into_iter()
            .chain(tracked),
    )?;
    git.text(root, ["commit", "-m", &format!("Release {version}")])?;
    git.text(root, ["tag", &tag])?;
    // Publish both refs together, or neither. In particular, a rejected branch must not release a tag.
    git.text(root, ["push", "--atomic", "--no-follow-tags", "origin", &branch, &tag_ref])
        .map_err(|e| format!("{e}\nThe release commit and {tag} remain local. Resolve the push error, then push both refs to origin."))?;
    println!(
        "Released {version}: pushed {tag} to origin. GitHub Actions will build and attest the release."
    );
    Ok(())
}

#[cfg(test)]
mod tests;
