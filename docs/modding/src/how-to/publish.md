# Publishing your mod

Use [the Sunrise mod registry](https://playsunrise.net/mods) to distribute a content or native mod. Both ship as
`.smod` packages. A native release carries its source and its `Cargo.lock`, never compiled libraries: each player's
computer builds a native mod for the exact game it runs.

## Releasing

Commit your mod's source, [`page.toml` and screenshots](#your-mods-page), and `Cargo.lock` for a native mod.
`cargo sunrise-mod new` includes a release workflow for a new repository. For an existing repository or a
package in a subfolder, the first release writes the workflow and removes the old copied Sunrise release files
in its commit. To review those changes first, run `cargo sunrise-mod release setup` from the mod's folder on a
clean tree, then commit and push them. Register the repository at
[Submit a mod](https://playsunrise.net/mods/submit).

From the mod's folder on a clean branch that is up to date with its upstream:

```sh
cargo sunrise-mod release patch --check
cargo sunrise-mod release patch
```

Use `minor`, `major`, or a higher SemVer instead of `patch` to choose another version. `--check` runs the
release checks for the proposed version without changing the checkout. Without it, `release` refreshes the
workflow, commits the version bump, and pushes the branch and tag together to `origin`. The tag is `v<version>`
at the repository root or `<id>/v<version>` for a subfolder package. GitHub Actions builds and attests both the
`.smod` and its page bundle, with no signing key for you to manage. The registry bot submits them for review.

## How releasing works

1. You register your mod's repository once at [Submit a mod](https://playsunrise.net/mods/submit).
2. `cargo sunrise-mod release patch` (or `minor`, `major`, or a version) checks the mod, bumps its version, and
   pushes a release tag.
3. The tag runs [the shared release workflow](https://github.com/sunrise-org/sunrise-mods/blob/main/.github/workflows/release.yml),
   which builds the `.smod`, signs it with a GitHub artifact attestation that names your repository, and attaches
   it to a GitHub release. You keep no signing keys.
4. The registry checks the release and opens a registry pull request, which a reviewer approves before the version
   is listed.

The earlier route (`publish`, `keygen`, `sign`, `release plan` and prebuilt binaries) has been removed.

## Your mod's page

For a tagged registry release, edit `page.toml` beside `sunrise-mod.toml`:

- `summary`: a short description of at least 25 characters.
- `description`: a long description of at least 125 characters. You can use Markdown. Without it the page
  uses the manifest's `description`.
- `categories`: an optional list of up to eight distinct IDs from the registry's `registry/categories.toml`.
  Each category must apply to your package kind (`mod` or `native-mod`).
- `[[screenshots]]`: at least one entry, with a `path` such as `page/recipes.png` and an optional `caption`.
  Put the images under `page/`. Use readable PNG (`.png`) or JPEG (`.jpg` or `.jpeg`) files, at most 5 MiB each.

Text minimums count Unicode scalar values after trimming surrounding whitespace. `description`, `name`,
`authors`, `homepage` and `issues` default to the values in `sunrise-mod.toml`; set them in `page.toml` to
show different ones on the page. The name and each listed author must not be
blank. Links must be absolute HTTP(S) URLs without credentials. The page uses the mod's own `icon.png`, which a release needs, as it needs a `license` in
`sunrise-mod.toml`. Screenshot paths must be distinct, must not be `page/icon.png`, and must be names Windows
can create, and the page bundle must fit the registry's 64 MiB for one release file.

Run `cargo sunrise-mod check` to see what you still need before release. Run
[`cargo sunrise-mod check --release <tag>`](../reference/sdk/05-commands.md) to require a complete page and
check the tag against the manifest's version. `build` writes `target/sunrise-mod/<id>.page.tar` separately from the
`.smod` whenever the project has a `page.toml`.
