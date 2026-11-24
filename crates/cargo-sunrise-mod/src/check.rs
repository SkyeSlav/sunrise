//! Package inspection and the release attestation policy for `cargo sunrise-mod check <file>`.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;
use sunrise_attestation::{Attested, PUBLIC_GOOD_TRUSTED_ROOT};
use sunrise_mod_package::Package;
use sunrise_mod_package::index::{self, Facts, Kind};
use sunrise_mod_package::manifest::{MANIFEST, MAX_USES};
use sunrise_shader_api::mod_pipeline;

pub struct Report {
    pub kind: Kind,
    pub facts: Facts,
    pub sha256: String,
    pub size: u64,
    pub signed: bool,
    attestation: Option<Attested>,
}

impl Report {
    pub fn json(&self) -> Value {
        let requires: Vec<Value> = self
            .facts
            .requires
            .iter()
            .map(|r| json!({"id": r.id, "version": r.version, "sha256": r.sha256}))
            .collect();
        let mut report = json!({
            "kind": self.kind.name(),
            "id": self.facts.id,
            "version": self.facts.version,
            "api": self.facts.api,
            "side": self.facts.side,
            "capabilities": self.facts.capabilities,
            "game": self.facts.game,
            "dependencies": self.facts.dependencies,
            "requires": requires,
            "sha256": self.sha256,
            "size": self.size,
            "signed": self.signed,
        });
        if let Some(attested) = &self.attestation {
            report["attestation"] = json!({
                "repository_id": attested.repository_id,
                "repository_name": attested.repository_name,
                "signer_workflow_ref": attested.signer_workflow_ref,
                "source_ref": attested.source_ref,
                "source_commit": attested.source_commit,
                "predicate_type": attested.predicate_type,
            });
        }
        report
    }

    pub fn text(&self) -> String {
        let f = &self.facts;
        let mut out = format!(
            "{} {} ({}",
            f.id.as_deref().unwrap_or("resource pack"),
            f.version.as_deref().unwrap_or(""),
            self.kind.name()
        );
        if let Some(side) = f.side {
            let side = serde_json::to_value(side).unwrap_or_default();
            out += &format!(", API {}, {}", f.api, side.as_str().unwrap_or("?"));
        } else if self.kind == Kind::Shader {
            out += &format!(", shader API {}", f.api);
        } else if !f.api.is_empty() {
            out += &format!(", format {}", f.api);
        }
        if let Some(game) = &f.game {
            out += &format!(", for {game}");
        }
        let signed = if self.signed {
            "signature verified"
        } else {
            "unsigned"
        };
        out += &format!(
            ")\n  {} bytes, SHA-256 {}, {signed}\n",
            self.size, self.sha256
        );
        if let Some(attested) = &self.attestation {
            out += &format!(
                "  repository: {} (ID {})\n  workflow: {}\n  commit: {}\n",
                attested.repository_name,
                attested.repository_id,
                attested.signer_workflow_ref,
                attested.source_commit
            );
        }
        for c in &f.capabilities {
            out += &format!("  may {}\n", c.describe());
        }
        for dep in &f.dependencies {
            let label = if dep.optional {
                "optional:"
            } else {
                "requires"
            };
            let side = match dep.side {
                Some(sunrise_mod_package::manifest::Side::Client) => " (client only)",
                Some(sunrise_mod_package::manifest::Side::Server) => " (dedicated server only)",
                _ => "",
            };
            out += &format!("  {label} {} {}{side}\n", dep.id, dep.version);
        }
        for r in &f.requires {
            out += &format!("  needs {} {} ({})\n", r.id, r.version, &r.sha256[..12]);
        }
        if self.kind == Kind::NativeMod {
            out += "  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Sunrise and Mixins revisions before review.\n";
        }
        if self.kind == Kind::Mod {
            out += "  no code: content only, nothing to run\n";
        }
        out
    }
}

/// The kind of a package, inspecting mod archives for their native manifest.
pub fn kind_of(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()? {
        "smod" if index::native::holds(path) => Some(Kind::NativeMod),
        "smod" => Some(Kind::Mod),
        "spack" => Some(Kind::Modpack),
        "zip" => Some(sunrise_mod_package::index::zip_kind(path)),
        "sshader" => Some(Kind::Shader),
        _ => None,
    }
}

/// Inspect a downloaded package and require an attestation for mods.
pub fn check_release(
    path: &Path,
    kind: Kind,
    attestation: Option<&Path>,
    repository: Option<u64>,
) -> Result<Report, String> {
    if !matches!(kind, Kind::Mod | Kind::NativeMod)
        && path.extension().is_none_or(|ext| ext != "smod")
    {
        if attestation.is_some() || repository.is_some() {
            return Err("--attestation and --repository apply only to .smod packages".into());
        }
        return inspect(path, kind, true);
    }
    let default_sidecar;
    let sidecar = match attestation {
        Some(sidecar) => sidecar,
        None => {
            let mut name = path.as_os_str().to_owned();
            name.push(".sigstore.json");
            default_sidecar = std::path::PathBuf::from(name);
            &default_sidecar
        }
    };
    let bundle = std::fs::read(sidecar).map_err(|e| match e.kind() {
        ErrorKind::NotFound => format!(
            "not signed: the release workflow attests each .smod; bundle not found: {}",
            sidecar.display()
        ),
        _ => format!("{}: {e}", sidecar.display()),
    })?;
    let mut report = inspect(path, kind, true)?;
    let mut sha256 = [0; 32];
    hex::decode_to_slice(&report.sha256, &mut sha256).map_err(|e| e.to_string())?;
    report.attestation = Some(
        verify_release(&bundle, sha256, repository)
            .map_err(|e| format!("{}: {e}", sidecar.display()))?,
    );
    report.signed = true;
    Ok(report)
}

fn verify_release(
    bundle: &[u8],
    sha256: [u8; 32],
    repository: Option<u64>,
) -> Result<Attested, String> {
    const WORKFLOW: &str = "sunrise-org/sunrise-mods/.github/workflows/release.yml";
    const TAG_PREFIX: &str = "refs/tags/release-v";
    let attested = sunrise_attestation::verify(bundle, sha256, PUBLIC_GOOD_TRUSTED_ROOT)
        .map_err(|e| e.to_string())?;
    if let Some(expected) = repository
        && attested.repository_id != expected
    {
        return Err(format!(
            "expected repository ID {expected}, attestation names {} (ID {})",
            attested.repository_name, attested.repository_id
        ));
    }
    let allowed =
        attested
            .signer_workflow_ref
            .split_once('@')
            .is_some_and(|(workflow, git_ref)| {
                workflow == WORKFLOW
                    && git_ref
                        .strip_prefix(TAG_PREFIX)
                        .is_some_and(|version| !version.is_empty() && !version.contains('/'))
            });
    if !allowed {
        return Err(format!(
            "expected workflow {WORKFLOW}@{TAG_PREFIX}*, attestation names {}",
            attested.signer_workflow_ref
        ));
    }
    Ok(attested)
}

/// Validate a package's contents, independent of signature or registry policy.
/// A legacy reviewed source being republished with only a new build stays exact
/// to its pin and need not carry a uses list.
pub(crate) fn inspect(path: &Path, kind: Kind, require_uses: bool) -> Result<Report, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > index::MAX_FILE {
        return Err(format!("over {} MiB", index::MAX_FILE >> 20));
    }
    index::open(&kind)?;
    let facts = index::inspect(&kind, path)?;
    if require_uses && kind == Kind::NativeMod {
        uses_list(path)?;
    }
    Ok(Report {
        kind,
        facts,
        sha256: index::sha256_file(path)?,
        size,
        signed: false,
        attestation: None,
    })
}

/// A native mod carries the list of what it uses, made for the build it pins: without it the mod
/// works on that one build only. A mod with rows no list can judge
/// ([`sunrise_native_build::game_uses::unjudged`]) is made for one build by nature and needs none.
fn uses_list(path: &Path) -> Result<(), String> {
    use sunrise_mod_package::mod_surface::uses::{USES_FILE, UsesTable};
    let manifest = Package::read(path)?.manifest;
    if sunrise_native_build::game_uses::unjudged(&manifest).is_some() {
        return Ok(());
    }
    let pin = manifest
        .native
        .ok_or_else(|| format!("no [native] table in {MANIFEST}"))?
        .sunrise;
    let repack = "Pack the mod with `cargo sunrise-mod build`, which writes the list; if it says it cannot read the mod's code, fix what it names first.";
    let Some(bytes) = Package::entry(path, USES_FILE, MAX_USES)? else {
        return Err(format!(
            "no {USES_FILE}: the package does not list what the mod uses, so it would work on game build {pin} only and stop at the next one. {repack}"
        ));
    };
    let list = UsesTable::read(&bytes).map_err(|e| format!("{USES_FILE}: {e}. {repack}"))?;
    if list.made_for != pin {
        return Err(format!(
            "{USES_FILE} was made for game build {}, and the mod pins {pin}, so the list says nothing about the pinned build. {repack}",
            list.made_for
        ));
    }
    Ok(())
}

/// The references among a project's content files ([`sunrise_mod_package::content::check_references`]):
/// warnings are printed, errors fail with one line each.
pub fn references(root: &Path, manifest: &sunrise_mod_package::Manifest) -> Result<(), String> {
    let content = sunrise_mod_package::Content::read(root, manifest)?;
    let findings = sunrise_mod_package::content::check_references(&content, manifest)?;
    for warning in &findings.warnings {
        eprintln!("warning: {warning}");
    }
    if findings.errors.is_empty() {
        Ok(())
    } else {
        Err(findings.errors.join("\nerror: "))
    }
}

/// Composes and validates every render pipeline fragment file in a mod's `files`
/// (`assets/<namespace>/shaders/pipeline/<path>.wgsl`) with the game's own code
/// (`sunrise_shader_api::mod_pipeline`), returning how many there are and each refusal as
/// `<file>:<line>:<column>: <message>` (`<file>: <message>` where it has no span), the text the game logs after the pipeline's id. Whether a pipeline is
/// textured is in the mod's code, which this cannot see: a file that calls `pipeline_texture`
/// is composed as textured, any other as untextured.
pub fn pipeline_shaders(files: &BTreeMap<String, Vec<u8>>) -> (usize, Vec<String>) {
    let mut count = 0;
    let mut refused = Vec::new();
    for (name, bytes) in files {
        let mut parts = name.splitn(3, '/');
        let in_folder = parts.next() == Some("assets")
            && parts.next().is_some()
            && parts.next().is_some_and(|rest| {
                rest.strip_prefix(mod_pipeline::FOLDER)
                    .is_some_and(|rest| rest.starts_with('/') && rest.ends_with(".wgsl"))
            });
        if !in_folder {
            continue;
        }
        count += 1;
        let composed = std::str::from_utf8(bytes)
            .map_err(|e| e.to_string())
            .map_err(|e| vec![format!("{name}: {e}")])
            .and_then(|text| {
                mod_pipeline::compose_with(name, text, text.contains("pipeline_texture"))
                    .map_err(|problems| problems.iter().map(ToString::to_string).collect())
            });
        if let Err(reasons) = composed {
            refused.extend(reasons);
        }
    }
    (count, refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_files_are_composed_as_the_game_does() {
        let glow =
            "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return input.color; }";
        let sample = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return pipeline_texture(input.uv); }";
        let files: BTreeMap<String, Vec<u8>> = [
            ("assets/m/shaders/pipeline/glow.wgsl", glow),
            ("assets/m/shaders/pipeline/deep/sample.wgsl", sample),
            ("assets/m/shaders/pipeline/broken.wgsl", "fn nothing() {}"),
            (
                "assets/m/shaders/pipeline/typo.wgsl",
                "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {\n    let x: f32 = input.uv;\n    return vec4<f32>(x);\n}",
            ),
            // Not a pipeline file: other shaders are not this check's.
            ("assets/m/shaders/post/other.wgsl", "garbage"),
            ("assets/m/lang/en_us.json", "{}"),
        ]
        .map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec()))
        .into();
        let (count, refused) = pipeline_shaders(&files);
        assert_eq!(count, 4);
        assert_eq!(
            refused,
            [
                "assets/m/shaders/pipeline/broken.wgsl: no function `pipeline_fragment`: expected `fn pipeline_fragment(input: PipelineInput) -> vec4<f32>`",
                // The binding `x` is at line 2, column 9 of the mod's own file.
                "assets/m/shaders/pipeline/typo.wgsl:2:9: the type of `x` is expected to be `f32`, but got `vec2<f32>`",
            ]
        );
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sunrise-mod-package/tests/index")
            .join(name)
    }

    #[test]
    fn checks_each_kind_its_own_way() {
        let pack = fixture("cozy-1.0.0.spack");
        let report = inspect(&pack, kind_of(&pack).unwrap(), true).unwrap();
        assert_eq!(report.facts.requires[0].id, "coords_hud");
        assert!(
            report
                .text()
                .contains("cozy 1.0.0 (modpack, format 1, for 26.3)")
        );
        let smod = fixture("coords_hud-0.1.0.smod");
        let report = inspect(&smod, Kind::Mod, true).unwrap();
        assert!(report.text().contains("no code: content only"));
        let e = inspect(&pack, Kind::Shader, true).err().unwrap();
        assert!(e.contains("sunrise-shaders.toml"), "{e}");
        assert!(inspect(&pack, Kind::Mod, true).is_err());
    }

    #[test]
    fn unsigned_smod_is_refused_and_an_explicit_bundle_is_read() {
        let smod = fixture("coords_hud-0.1.0.smod");
        let missing = check_release(&smod, Kind::Mod, None, None).err().unwrap();
        assert!(missing.starts_with("not signed: the release workflow attests each .smod"));
        assert!(missing.ends_with("coords_hud-0.1.0.smod.sigstore.json"));

        let bundle =
            fixture("../../../sunrise-attestation/tests/fixtures/ruff-source.sigstore.json");
        let mismatch = check_release(&smod, Kind::Mod, Some(&bundle), None)
            .err()
            .unwrap();
        assert!(
            mismatch.contains("artifact hash does not match"),
            "{mismatch}"
        );
    }

    #[test]
    fn real_attestation_from_another_workflow_or_repository_is_refused() {
        let bundle =
            include_bytes!("../../sunrise-attestation/tests/fixtures/ruff-source.sigstore.json");
        let mut sha256 = [0; 32];
        hex::decode_to_slice(
            "0378ef60356caaad7b84653691a5d57a1d8b57fd59d173cfc27b0fd4a91a13f3",
            &mut sha256,
        )
        .unwrap();
        let workflow = verify_release(bundle, sha256, Some(523043277)).unwrap_err();
        assert!(workflow.starts_with("expected workflow sunrise-org/sunrise-mods/"));
        assert!(workflow.ends_with("astral-sh/ruff/.github/workflows/release.yml@refs/heads/main"));
        let repository = verify_release(bundle, sha256, Some(1)).unwrap_err();
        assert_eq!(
            repository,
            "expected repository ID 1, attestation names astral-sh/ruff (ID 523043277)"
        );
    }

    #[test]
    fn dependencies_are_reported_as_required_or_optional() {
        let dependencies = json!([
            {"id": "creature", "version": "^0.1"},
            {"id": "jei", "version": "^0.5", "optional": true, "side": "client"}
        ]);
        let report = Report {
            kind: Kind::NativeMod,
            facts: Facts {
                id: Some("fireflies".into()),
                version: Some("1.0.0".into()),
                dependencies: serde_json::from_value(dependencies.clone()).unwrap(),
                ..Facts::default()
            },
            sha256: "0".repeat(64),
            size: 1,
            signed: false,
            attestation: None,
        };
        let text = report.text();
        for word in ["creature", "^0.1", "jei", "^0.5"] {
            assert!(text.contains(word), "{text}");
        }
        assert_eq!(report.json()["dependencies"], dependencies);
    }
}
