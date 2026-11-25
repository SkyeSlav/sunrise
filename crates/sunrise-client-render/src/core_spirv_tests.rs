//! The core shaders' and post shaders' modules as SPIR-V, hashed, so a change that must leave
//! them alone (#5533 step 3: `sunrise-glsl` running the directives itself, not naga's
//! preprocessor with `Options.defines`) shows it by leaving the text identical.
//!
//! The manifest of vanilla's shaders is never committed (they are Mojang's): the run at the old
//! head writes it with `SUNRISE_GLSL_SPIRV_OUT=<new folder>`, the run at the new head compares
//! with it through `SUNRISE_GLSL_SPIRV_BASE=<that folder>`. The Polytone fixture's manifest is
//! committed (`tests/fixtures/polytone_directives.txt`); a missing one is written by running the
//! test with `SUNRISE_BLESS_SPIRV_DUMP=1`, at the head before the change.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_glsl::Glue;

use crate::core_shader::translate_core_with;
use crate::post_glsl::{Declarable, translate};

/// Every input a vanilla core shader's vertex stage reads.
const GLUE: Glue = Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Color", "vec4"),
        ("UV0", "vec2"),
        ("UV1", "ivec2"),
        ("UV2", "ivec2"),
        ("Normal", "vec3"),
        ("LineWidth", "float"),
    ],
    prelude: "layout(location = 0) in vec3 hz_position;",
    setup: "Position = hz_position; Color = vec4(1.0); UV0 = vec2(0.0); UV1 = ivec2(0); \
            UV2 = ivec2(0); Normal = vec3(0.0, 1.0, 0.0); LineWidth = 1.0;",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

/// A module's line: the SHA-256 of its SPIR-V words (little-endian bytes) and their count, or of
/// naga's `Debug` text when it doesn't validate or the writer refuses it. The writer's flags are
/// fixed here, as `crates/sunrise-render-shaderpack/tests/spirv_dump.rs` fixes them.
fn module_line(module: &naga::Module) -> String {
    use naga::back::spv::{Options, WriterFlags, write_vec};
    let options = Options {
        flags: WriterFlags::DEBUG
            | WriterFlags::ADJUST_COORDINATE_SPACE
            | WriterFlags::LABEL_VARYINGS
            | WriterFlags::CLAMP_FRAG_DEPTH,
        ..Default::default()
    };
    let validated = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module);
    let words = validated
        .map_err(|_| "invalid")
        .and_then(|info| write_vec(module, &info, &options, None).map_err(|_| "spirv-refused"));
    let hex = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
    };
    match words {
        Ok(words) => {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            format!("spirv {} words {}", hex(&bytes), words.len())
        }
        Err(why) => format!("{why} module {}", hex(format!("{module:?}").as_bytes())),
    }
}

fn program_lines(
    out: &mut String,
    name: &str,
    program: Result<(&naga::Module, &naga::Module), String>,
) {
    match program {
        Ok((vertex, fragment)) => {
            let _ = writeln!(out, "{name}\n  vertex {}", module_line(vertex));
            let _ = writeln!(out, "  fragment {}", module_line(fragment));
        }
        Err(why) => {
            let _ = writeln!(out, "{name}\n  refused {why}");
        }
    }
}

/// Stems of the `extension` files in `dir`, sorted.
fn stems(dir: &Path, extension: &str) -> Vec<String> {
    let mut stems: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| {
            let path = e.ok()?.path();
            (path.extension()? == extension)
                .then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    stems.sort();
    stems
}

/// Vanilla's core shaders (each with no defines and with `IS_GUI`) and post shaders under `root`.
fn vanilla(root: &Path) -> String {
    let mut out = String::new();
    let core = root.join("assets/minecraft/shaders/core");
    let read = |path: &Path| std::fs::read_to_string(path).ok();
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        read(&root.join(format!("assets/{namespace}/{path}")))
    };
    for stem in stems(&core, "vsh") {
        let (Some(vertex), Some(fragment)) = (
            read(&core.join(format!("{stem}.vsh"))),
            read(&core.join(format!("{stem}.fsh"))),
        ) else {
            let _ = writeln!(out, "core {stem}\n  no fragment stage");
            continue;
        };
        for defines in [&[][..], &["IS_GUI"][..]] {
            let translated =
                translate_core_with(&vertex, &fragment, defines, &include, &GLUE, &|_| true);
            let name = format!("core {stem} {defines:?}");
            program_lines(
                &mut out,
                &name,
                translated
                    .as_ref()
                    .map(|t| (&t.vertex, &t.fragment))
                    .map_err(|e| sunrise_glsl::describe(e)),
            );
        }
    }
    let source = ResourceSource::directory(root.to_path_buf());
    let declarable = Declarable {
        blocks: 64,
        samplers: 64,
        interface: naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
    };
    let post = root.join("assets/minecraft/shaders/post");
    for stem in stems(&post, "fsh") {
        let vertex = if post.join(format!("{stem}.vsh")).is_file() {
            stem.as_str()
        } else {
            "blit"
        };
        let translated = translate(
            &source,
            &format!("minecraft:post/{vertex}"),
            &format!("minecraft:post/{stem}"),
            declarable,
        );
        program_lines(
            &mut out,
            &format!("post {stem}"),
            translated
                .as_ref()
                .map(|p| (&p.vertex, &p.fragment))
                .map_err(|e| format!("{e:?}")),
        );
    }
    out
}

/// The committed Polytone GPU particle shader that is written with directives, translated as
/// the game translates a pack's: the record as its vertex format, the particle's block provided.
fn polytone_fixture() -> String {
    use crate::particles::polytone_gpu::GLUE;
    use sunrise_polytone::gpu_particle::GPU_BLOCK;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polytone_directives");
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let translated = translate_core_with(
        &read("spark.vsh"),
        &read("spark.fsh"),
        &[],
        &|_| None,
        &GLUE,
        &|name| name == GPU_BLOCK,
    );
    let mut out = String::new();
    program_lines(
        &mut out,
        "polytone spark",
        translated
            .as_ref()
            .map(|t| (&t.vertex, &t.fragment))
            .map_err(|e| sunrise_glsl::describe(e)),
    );
    out
}

#[test]
#[ignore = "daily"]
fn vanilla_core_and_post_shaders_and_a_polytone_fixture_keep_their_spirv() {
    let root =
        PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect("SUNRISE_TEST_RESOURCES"));
    let text = vanilla(&root);
    assert_eq!(text, vanilla(&root), "two runs give different modules");
    assert!(text.contains("spirv "), "no vanilla shader translated");
    if let Some(out) = std::env::var_os("SUNRISE_GLSL_SPIRV_OUT") {
        std::fs::create_dir(&out).unwrap_or_else(|e| panic!("{}: {e}", Path::new(&out).display()));
        std::fs::write(Path::new(&out).join("vanilla.txt"), &text).unwrap();
    }
    if let Some(base) = std::env::var_os("SUNRISE_GLSL_SPIRV_BASE") {
        let old = std::fs::read_to_string(Path::new(&base).join("vanilla.txt"))
            .unwrap_or_else(|e| panic!("{}: {e}", Path::new(&base).display()));
        assert!(
            text == old,
            "vanilla's shaders' SPIR-V changed: diff the manifests in SUNRISE_GLSL_SPIRV_OUT and SUNRISE_GLSL_SPIRV_BASE"
        );
    }
    let fixture = polytone_fixture();
    assert!(fixture.contains("spirv "), "{fixture}");
    assert_eq!(
        fixture,
        polytone_fixture(),
        "two runs give different modules"
    );
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polytone_directives.txt");
    match std::fs::read_to_string(&manifest) {
        Ok(committed) => assert_eq!(fixture, committed, "{}", manifest.display()),
        Err(e) if std::env::var("SUNRISE_BLESS_SPIRV_DUMP").as_deref() == Ok("1") => {
            std::fs::write(&manifest, &fixture).unwrap_or_else(|e| panic!("{e}"));
            eprintln!("wrote {} ({e})", manifest.display());
        }
        Err(e) => panic!(
            "{}: {e} (write it with SUNRISE_BLESS_SPIRV_DUMP=1 at the head before the change)",
            manifest.display()
        ),
    }
}
