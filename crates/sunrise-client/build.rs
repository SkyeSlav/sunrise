//! Generates the native mod API, and with `parity-fixtures` the table of the parity harnesses' fixtures.

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = std::path::Path::new(&dir).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    sunrise_mod_api_gen::generate(&root, &out_dir, sunrise_mod_api_gen::Target::Client)
        .expect("generating native mod API");
    if std::env::var_os("CARGO_FEATURE_PARITY_FIXTURES").is_some() {
        let table =
            parity_fixtures(std::path::Path::new(&dir)).expect("finding the parity fixtures");
        std::fs::write(out_dir.join("parity_fixtures.rs"), table)
            .expect("writing the parity fixtures");
    }
}

/// `game::parity_fixtures`' table: every `#[test]` whose body is one call to a function beside it that a build
/// without `test` still compiles (its module is `any(test, feature = "parity-fixtures")`), by its libtest name.
/// The bin `sunrise-parity-fixtures` runs them as the lib test would, without compiling the lib's other tests.
fn parity_fixtures(manifest: &std::path::Path) -> Result<String, String> {
    use sunrise_mod_source::modules::{test_only, walk_module_files};
    let lib = manifest.join("src/lib.rs");
    let files = walk_module_files(
        "root".into(),
        lib.clone(),
        manifest.join("src"),
        false,
        false,
        |path, source| syn::parse_file(source).map_err(|e| format!("{}: {e}", path.display())),
    )?;
    let mut fixtures = Vec::new();
    for file in files {
        println!("cargo::rerun-if-changed={}", file.path.display());
        let source = std::fs::read_to_string(&file.path)
            .map_err(|e| format!("{}: {e}", file.path.display()))?;
        let parsed =
            syn::parse_file(&source).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let module = file
            .module
            .strip_prefix("root")
            .unwrap_or(&file.module)
            .trim_start_matches("::");
        collect(module, &parsed.items, &mut fixtures);
    }
    fixtures.sort();
    let mut table = String::from("pub(super) const FIXTURES: &[(&str, fn())] = &[\n");
    for (name, function) in fixtures {
        if !name.starts_with("game::") {
            return Err(format!(
                "{name}: a parity fixture lives under `game`, where its table can reach it"
            ));
        }
        table.push_str(&format!("    ({name:?}, crate::{function}),\n"));
    }
    table.push_str("];\n");
    return Ok(table);

    fn collect(module: &str, items: &[syn::Item], fixtures: &mut Vec<(String, String)>) {
        let path = |name: &dyn std::fmt::Display| {
            if module.is_empty() {
                name.to_string()
            } else {
                format!("{module}::{name}")
            }
        };
        let compiled: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Fn(function) if !test_only(&function.attrs) => {
                    Some(function.sig.ident.to_string())
                }
                _ => None,
            })
            .collect();
        for item in items {
            match item {
                syn::Item::Mod(child) if !test_only(&child.attrs) => {
                    if let Some((_, items)) = &child.content {
                        collect(&path(&child.ident), items, fixtures);
                    }
                }
                syn::Item::Fn(function)
                    if function.attrs.iter().any(|a| a.path().is_ident("test")) =>
                {
                    if let [syn::Stmt::Expr(syn::Expr::Call(call), _)] =
                        function.block.stmts.as_slice()
                        && call.args.is_empty()
                        && let syn::Expr::Path(callee) = &*call.func
                        && let Some(callee) = callee.path.get_ident()
                        && compiled.contains(&callee.to_string())
                    {
                        fixtures.push((path(&function.sig.ident), path(callee)));
                    }
                }
                _ => {}
            }
        }
    }
}
