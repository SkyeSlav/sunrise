//! Select the add-on's library projects before invoking the native build.
use crate::registry_source::{Registry, write_files};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    Manifest, Package,
    index::{self, Index, Kind, Project, Version, native::Bundle, resolve},
    manifest::Side,
};

struct LocalSource {
    path: PathBuf,
    package: Package,
}

pub(super) struct Libraries {
    local: BTreeMap<String, LocalSource>,
    index: Index,
    side: Side,
}

impl Libraries {
    pub(super) fn read(project: &Manifest, paths: &[&Path], side: Side) -> Result<Self, String> {
        let mut local = BTreeMap::new();
        for path in paths {
            let path =
                dunce::canonicalize(path).map_err(|e| format!("--with {}: {e}", path.display()))?;
            if !path.is_dir() && path.extension().and_then(|s| s.to_str()) != Some("smod") {
                return Err(format!(
                    "--with {}: expected a mod folder or .smod",
                    path.display()
                ));
            }
            let package =
                Package::read(&path).map_err(|e| format!("--with {}: {e}", path.display()))?;
            let id = &package.manifest.id;
            if *id == project.id || local.contains_key(id) {
                return Err(format!(
                    "duplicate mod id {id} in --with sources and the project"
                ));
            }
            local.insert(id.clone(), LocalSource { path, package });
        }
        // Walk libraries supplied with --with, including explicit optional libraries. Only an
        // unsupplied required edge needs the index.
        let mut todo = vec![project];
        let mut seen = BTreeSet::new();
        let mut index = Index::default();
        'local_closure: while let Some(manifest) = todo.pop() {
            for dep in
                sunrise_mod_package::dependencies::from_manifest(&manifest.dependencies, side)
            {
                if let Some(source) = local.get(&dep.id) {
                    if seen.insert(source.package.manifest.id.as_str()) {
                        todo.push(&source.package.manifest);
                    }
                } else if !dep.optional && dep.id != project.id {
                    index = Registry::default().read_index().map_err(|error| format!(
                        "{} needs {} {}. Supply the library with --with <path> or use the Sunrise mod registry. Could not read the mod registry: {error}",
                        manifest.id, dep.id, dep.requirement
                    ))?;
                    break 'local_closure;
                }
            }
        }
        Ok(Self { local, index, side })
    }

    fn plan(
        &self,
        project: &Manifest,
        game: &str,
        native_fits: impl Fn(&Project, &Version) -> bool,
    ) -> Result<resolve::ManifestPlan<'_>, String> {
        let archives: BTreeMap<_, _> = self
            .local
            .iter()
            .filter(|(_, source)| !source.path.is_dir())
            .map(|(id, source)| index::sha256_file(&source.path).map(|hash| (id, hash)))
            .collect::<Result<_, _>>()?;
        let locals: Vec<_> = self
            .local
            .iter()
            .map(|(id, source)| resolve::LocalMod {
                manifest: &source.package.manifest,
                sha256: archives.get(id).map(String::as_str),
            })
            .collect();
        resolve::for_manifest(&self.index, project, &locals, game, self.side, native_fits)
            .map_err(|error| format!("{}: {error}. Supply the library with --with <path> or use a matching version in the Sunrise mod registry.", project.id))
    }

    pub(super) fn stage(
        &self,
        bundle: &sunrise_native_build::Bundle,
        sdk: &super::NativeSdk,
        cache: &Path,
        root: &Path,
        release: bool,
    ) -> Result<BTreeMap<String, PathBuf>, String> {
        let registry = Registry::default();
        let build_data = RefCell::new(None);
        let rejected = RefCell::new(Vec::new());
        let unavailable = RefCell::new(None);
        let selected = self.plan(&bundle.manifest, &bundle.native().game, |project, row| {
            if unavailable.borrow().is_some() {
                return false;
            }
            // The old registry index's sources carry no attestation, so a release never
            // compiles one.
            if release {
                *unavailable.borrow_mut() = Some(format!(
                    "{} {}: a release can't take a library from the old registry index, whose sources aren't attested",
                    project.id, row.version
                ));
                return false;
            }
            let candidate = (|| {
                let source = registry.native_source(cache, &self.index, project, row)?;
                if source
                    .manifest
                    .native
                    .as_ref()
                    .expect("native source")
                    .sunrise
                    == sdk.index.build.commit
                {
                    return Ok(None);
                }
                let candidate = tempfile::tempdir_in(cache).map_err(|e| e.to_string())?;
                write_files(candidate.path(), source.files)?;
                Ok(check_fit(
                    &bundle.dir,
                    sdk,
                    candidate.path(),
                    &source.manifest,
                    &mut build_data.borrow_mut(),
                    None,
                )
                .err())
            })();
            match candidate {
                Ok(None) => true,
                Ok(Some(why)) => {
                    rejected
                        .borrow_mut()
                        .push(format!("{} {}: {why}", project.id, row.version));
                    false
                }
                Err(error) => {
                    *unavailable.borrow_mut() = Some(error);
                    false
                }
            }
        });
        if let Some(error) = unavailable.into_inner() {
            return Err(error);
        }
        let selected = selected.map_err(|error| {
            std::iter::once(error)
                .chain(rejected.into_inner())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        let mut build_data = build_data.into_inner();
        let mut origins = BTreeMap::new();
        for id in selected.local {
            let source = &self.local[&id];
            if let Some(native) = &source.package.manifest.native {
                let into = root.join(super::stage_name(&id));
                let files = if source.path.is_dir() {
                    origins.insert(id.clone(), source.path.clone());
                    directory_files(&source.package, &source.path)?
                } else {
                    Bundle::read(&source.path)?.files
                };
                write_files(&into, files)?;
                if native.sunrise != sdk.index.build.commit {
                    check_fit(
                        &bundle.dir,
                        sdk,
                        &into,
                        &source.package.manifest,
                        &mut build_data,
                        release.then_some(root),
                    )?;
                }
            } else {
                let into = root.join(format!("{}.smod", super::stage_name(&id)));
                if source.path.is_dir() {
                    std::fs::write(&into, Package::pack_directory(&source.path)?)
                        .map_err(|e| format!("{}: {e}", into.display()))?;
                } else {
                    source.package.read_runtime(&source.path)?;
                    super::copy(&source.path, &into)?;
                }
            }
            println!(
                "Using library {id} {} from {}",
                source.package.manifest.version,
                source.path.display()
            );
        }
        for (project, row) in selected.registry {
            if project.kind == Kind::NativeMod {
                let source = registry.native_source(cache, &self.index, project, row)?;
                write_files(&root.join(super::stage_name(&project.id)), source.files)?;
                println!(
                    "Using library {} {} from the Sunrise mod registry",
                    project.id, row.version
                );
            }
        }
        Ok(origins)
    }
}

/// Published SDKs compare the library's recorded uses, just as the install path does. Keep the
/// original pin in the source snapshot; only the SDK used for this particular build changes.
/// `private` is a release's private build folder, where its build data is fetched.
fn check_fit(
    project: &Path,
    sdk: &super::NativeSdk,
    root: &Path,
    manifest: &Manifest,
    build_data: &mut Option<PathBuf>,
    private: Option<&Path>,
) -> Result<(), String> {
    use sunrise_native_build::{
        check::{self, Unlisted, Verdict},
        contract::CONTRACT,
    };
    let build = &sdk.index.build.commit;
    let pin = &manifest.native.as_ref().expect("native library").sunrise;
    let id = &manifest.id;
    let mut plan = check::Plan {
        contract: CONTRACT,
        mods: root.to_owned(),
        build: build.clone(),
        surface: Some(sdk.dir.join(crate::sdk_fetch::SURFACE)),
        build_data: build_data.clone(),
    };
    let refused =
        |why: String| format!("library {id} pins {pin} and cannot be built for {build}: {why}");
    let judge = |plan: &check::Plan| {
        check::check(plan)?
            .mods
            .into_iter()
            .next()
            .ok_or_else(|| "native-check returned no verdict for this library".to_owned())
    };
    let mut row = judge(&plan).map_err(refused)?;
    if row.needs == Some("build-data") {
        // A release judges with build data fetched from the signed SDK into its private build
        // folder, never the project's own cache, which the checkout can edit.
        *build_data = Some(match private {
            Some(dir) => {
                let staging = dir.join("sdk-data");
                crate::sdk_fetch::release_data(
                    &crate::sdk_fetch::Base::from_env(),
                    &crate::sdk_fetch::keys(),
                    build,
                    &staging,
                    true,
                )
                .map_err(refused)?;
                staging.join(build).join(crate::sdk_fetch::BUILD_DATA)
            }
            None => crate::sdk_fetch::build_data(project, build).map_err(refused)?,
        });
        plan.build_data = build_data.clone();
        row = judge(&plan).map_err(refused)?;
    }
    match row.verdict {
        Some(verdict) if verdict.usable() => (),
        Some(Verdict::DoesNotFit { misfits, .. }) => {
            return Err(refused(
                misfits
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        Some(Verdict::NoList { why, hooks, .. }) => {
            let detail = match why {
                Unlisted::Missing => "it has no uses list".into(),
                Unlisted::Unreadable => "its uses list is unreadable".into(),
                Unlisted::NewerSchema => "its uses list requires a newer tool".into(),
                Unlisted::OtherBuild => "its uses list is for another build".into(),
                Unlisted::HookVersions => {
                    format!("its uses list omits versions for {}", hooks.join(", "))
                }
                Unlisted::GameCode => "its uses list omits declared game code".into(),
            };
            return Err(refused(format!(
                "{detail}; build or repin the library against this SDK"
            )));
        }
        Some(Verdict::Exact { why, .. }) => {
            return Err(refused(format!(
                "it declares {why}, which needs the exact SDK"
            )));
        }
        _ => {
            return Err(refused(row.refusal.unwrap_or_else(|| {
                format!(
                    "native-check needs {}",
                    row.needs.unwrap_or("a fit verdict")
                )
            })));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn directory_files(package: &Package, dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = package.directory_files(dir)?;
    let uses = mod_surface::uses::USES_FILE;
    match crate::bounded::read_bytes(
        &dir.join(uses),
        mod_surface::uses::MAX_USES_BYTES as u64,
        crate::bounded::Symlinks::Refuse,
    ) {
        Ok(table) => {
            files.insert(uses.into(), table);
        }
        Err(error) if error.is_not_found() => (),
        Err(error) => return Err(error.into()),
    }
    Ok(files)
}
