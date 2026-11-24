use super::*;
use std::fs;

fn manifest() -> Manifest {
    Manifest::parse(
        r#"
id = "runes"
name = "Runes"
version = "1.0.0"
authors = ["Mod Author"]
homepage = "https://example.org/runes"
issues = "https://example.org/runes/issues"
"#,
    )
    .unwrap()
}

const TEXT: &str = "summary = 'Summary'\ndescription = 'Description'\n";

#[test]
fn page_fields_default_to_manifest_and_explicit_values_win() {
    let manifest = manifest();
    let page = Page::parse(TEXT, &manifest).unwrap();
    assert_eq!(page.name, "Runes");
    assert_eq!(page.authors, ["Mod Author"]);
    assert_eq!(page.homepage.as_deref(), Some("https://example.org/runes"));
    assert_eq!(
        page.issues.as_deref(),
        Some("https://example.org/runes/issues")
    );
    let page = Page::parse(
        &format!("{TEXT}name = 'Rune Stones'\nauthors = []\nhomepage = 'https://example.com'\nissues = 'https://example.com/issues'\n"),
        &manifest,
    ).unwrap();
    assert_eq!(page.name, "Rune Stones");
    assert!(page.authors.is_empty());
    assert_eq!(page.homepage.as_deref(), Some("https://example.com"));
    assert_eq!(page.issues.as_deref(), Some("https://example.com/issues"));
}

#[test]
fn page_refuses_unknown_root_and_screenshot_fields() {
    for fields in [
        "summmary = 'extra'",
        "[[screenshots]]\npath = 'page/runes.jpg'\ntitle = 'extra'",
    ] {
        let error = Page::parse(&format!("{TEXT}{fields}\n"), &manifest()).unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
    }
}

#[test]
fn screenshot_paths_are_relative_and_strictly_below_page() {
    let manifest = manifest();
    let long = format!("page/{}.jpg", "x".repeat(100));
    for (path, expected) in [
        ("runes.jpg", "under page/"),
        ("/page/runes.jpg", "under page/"),
        ("page/../runes.jpg", "under page/"),
        ("page/./runes.jpg", "under page/"),
        ("page/", "under page/"),
        ("page/C:/runes.jpg", "under page/"),
        ("page/sub\\runes.jpg", "under page/"),
        ("page/.. /runes.jpg", "dot or space"),
        (&long, "not a ustar path"),
    ] {
        let error = Page::parse(
            &format!("{TEXT}[[screenshots]]\npath = '{path}'\n"),
            &manifest,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn screenshot_format_comes_from_bytes_not_extension() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("page/nested/image.jpg");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let page = Page::parse(
        &format!(
            "{TEXT}[[screenshots]]\npath = 'page/nested/image.jpg'\ncaption = 'Rune Stones'\n"
        ),
        &manifest(),
    )
    .unwrap();
    let shot = &page.screenshots[0];
    assert_eq!(shot.caption.as_deref(), Some("Rune Stones"));
    fs::write(&path, b"not a JPG").unwrap();
    assert!(
        shot.read(root.path())
            .unwrap_err()
            .contains("not a PNG or JPEG")
    );
    for bytes in [b"\x89PNG\r\n\x1a\n".as_slice(), b"\xff\xd8\xff".as_slice()] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(shot.read(root.path()).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn linked_screenshot_subfolders_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("runes.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("page")).unwrap();
    let page = Page::parse(
        &format!("{TEXT}[[screenshots]]\npath = 'page/runes.png'\n"),
        &manifest(),
    )
    .unwrap();
    assert!(
        page.screenshots[0]
            .read(root.path())
            .unwrap_err()
            .contains("not a regular directory")
    );
}

#[test]
fn release_text_counts_trimmed_unicode_scalars() {
    let manifest = manifest();
    let metadata = |summary: &str, description: &str| {
        Page::parse(&format!("summary = ' {summary} '\ndescription = ' {description} '\n[[screenshots]]\npath = 'page/runes.png'\n"), &manifest).unwrap()
    };
    assert_eq!(
        metadata(&"🪨".repeat(24), &"🪨".repeat(124)).release_problems(None),
        [
            "page.toml: summary needs at least 25 characters",
            "page.toml: description needs at least 125 characters",
        ]
    );
    assert!(
        metadata(&"🪨".repeat(25), &"🪨".repeat(125))
            .release_problems(None)
            .is_empty()
    );
}

#[test]
fn release_checks_the_category_catalog_and_cap_but_allows_no_categories() {
    let catalog =
        Categories::parse("[[category]]\nid = 'utility'\nicon = 'hammer'\nkinds = ['mod']\n")
            .unwrap();
    let release = format!(
        "summary = '{}'\ndescription = '{}'\n",
        "s".repeat(25),
        "d".repeat(125)
    );
    let screenshots = "[[screenshots]]\npath = 'page/runestones.jpg'\n";
    let manifest = manifest();
    let check = |settings| {
        Page::parse(&format!("{release}{settings}\n{screenshots}"), &manifest)
            .unwrap()
            .release_problems(Some(&catalog))
    };
    assert!(check("").is_empty());
    assert!(check("categories = ['utility']").is_empty());
    assert_eq!(
        check("categories = ['not_a_category']"),
        ["page.toml: unknown registry category \"not_a_category\""]
    );
    assert_eq!(
        check(
            "categories = ['utility','utility','utility','utility','utility','utility','utility','utility','utility']"
        ),
        ["page.toml: choose at most 8 categories"]
    );
    assert_eq!(
        Page::parse(&release, &manifest)
            .unwrap()
            .release_problems(Some(&catalog)),
        ["page.toml: add at least one [[screenshots]] entry",]
    );
}
