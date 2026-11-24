# Publishing your mod

Sunrise's registry, [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods), lists mods, modpacks,
resource packs and shader packs at [playsunrise.net/mods](https://playsunrise.net/mods). Listing a project is a pull
request that a person reviews. `cargo sunrise-mod publish` prepares the submission locally; it never uploads or opens
the pull request for you.

## 1. Build and write the submission

In your mod's folder (content, native or wasm; add `--wasm` for a wasm mod, as for `build`):

```sh
cargo sunrise-mod publish --summary "One line about the mod." --owner your-github-login --categories utility
```

It builds the package as `build` does, which may download the SDK, toolchain and dependencies. It does not contact the
registry. The submission goes into `target/sunrise-mod/publish/` (change it with `--out <dir>`):

| Path | What it is |
| --- | --- |
| `mods/<id>.toml` | The registry entry, filled from `sunrise-mod.toml`: name, description, authors, license, `homepage` as the source, and the version's file name, SHA-256 and date |
| `<id>-<version>.smod` | The package: attach exactly this file to a release of your repository |
| `files/<sha256>` | The same file under the name the registry's checks read |
| `icons/<id>.png` | For a new listing, your `icon.png` from beside `sunrise-mod.toml` |
| `PULL_REQUEST.md` | The text of the pull request |

What the manifest cannot say comes from flags: `--summary` (at most 160 characters), `--owner <login>` (repeat it for
several; the GitHub accounts allowed to publish updates), `--categories a,b` (1 to 3 from [your kind's
list](../reference/registry-index.md#submitting-a-project)), `--tags a,b` and `--changelog <text>`. Anything still
missing is written as a value that starts with `TODO(publish)`, and the command lists what remains. The registry
refuses an entry that holds one, so edit `mods/<id>.toml` (or the manifest, then run `publish` again) until the list is empty.

Save `icon.png` beside `sunrise-mod.toml` as a square PNG, 64 to 512 pixels per side and at most 256 KiB, so it fits
both the package and the registry.

## 2. Check the new file

Run the `cargo sunrise-mod check <file>` command that `publish` prints, from your mod's folder. For example:

```sh
cargo sunrise-mod check 'target/sunrise-mod/publish/my_mod-0.1.0.smod'
```

This checks the new package and, for a signed project, its signature. `publish` also lists anything left to fill in
on the entry or page. Registry CI checks the whole checkout and plans installations after staging, with all listed
files available. Your submission folder does not contain previously published packages or an existing project's images.

For a first listing, you can also try an index containing just your project once the entry and icon are complete:

```sh
cargo sunrise-mod registry build 'target/sunrise-mod/publish' --files 'target/sunrise-mod/publish/files' \
  --out 'target/sunrise-mod/publish/index.json' \
  --release 'https://github.com/sunrise-org/sunrise-mods/releases/download/'
```

## 3. Open the pull request

1. Create a release of your repository and attach `<id>-<version>.smod` to it. Its address is the artifact URL.
2. Fork [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods) and copy in `mods/<id>.toml`.
   For a first listing, copy in `icons/<id>.png` too. Do not commit the package or `files/`.
3. Commit with `git commit -s`: every commit needs a `Signed-off-by:` line.
4. Open the pull request with the text of `PULL_REQUEST.md`, after pasting the artifact URL into it.

## 4. What review and staging do

- A maintainer copies your file from the artifact URL to the registry's `staging` release, named
  `<sha256>.smod`. Nothing is fetched from an address in a pull request on its own, and you cannot stage a file yourself.
- The `check` workflow then downloads each listed file, compares its SHA-256 with the entry's, and runs
  `registry review`, `registry check` and `registry build` on the result.
- A reviewer reads the change and merges it. The `publish` workflow then releases the file as `<id>-<version>` and
  adds the project to the index.
- The hash is checked again before any download reaches the game folder, so a file changed after
  review is refused.

## 5. A new version

Raise `version` in `sunrise-mod.toml`, then run `publish --registry <your checkout of sunrise-mods>` with an output
folder separate from the checkout. For a signed project, [build and sign first](#signing). `publish` adds the new
version row to the entry from the checkout, leaving all existing fields and rows in place. The printed local check
reads only the new file, so you do not need to download the old files or copy over the icon and screenshots.
Repeat steps 2 to 4.

Published versions never change and are never removed: a version already in the entry is refused by name, and a file
that must go is taken down by the registry's maintainers in `revoked.toml`.

## Signing

Authors may sign with an Ed25519 key in [minisign](https://jedisct1.github.io/minisign/) format. The entry then
holds the public key, every version of the project must be signed with it, and an update signed with another key is
refused. `cargo sunrise-mod` verifies a signature but does not make keys or signatures: use `minisign`.

Build and sign before calling `publish`, including on the first listing. For example, for a mod with ID `my_mod`
(add `--wasm` to `build` for a wasm mod):

```sh
cargo sunrise-mod build
minisign -S -s my_mod.key -m target/sunrise-mod/my_mod.smod
cargo sunrise-mod publish --summary "..." --owner you --categories utility \
  --key "$(tail -n 1 my_mod.pub)" --signature target/sunrise-mod/my_mod.smod.minisig
```

For a new version of an already-signed project, use the same key and replace the page flags (`--summary`, `--owner`,
`--categories`) with `--registry <your checkout of sunrise-mods>`. With `--key` and `--signature`, `publish` checks
and stages the exact bytes you signed, without rebuilding. If you change the package, build and sign it again.

Generate a key pair once with `minisign -G -p my_mod.pub -s my_mod.key`. Keep the secret key (`my_mod.key`) out of
both your repository and releases; losing it means a new key and a review.

## A collection

Add `collections/<id>.toml` and a square PNG at `icons/<id>.png` in one pull request to
[sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods). A collection is not a package: there's no file
upload or version to publish.

```toml
id = "sunrise_creatures"
name = "Sunrise Creatures"
summary = "Fireflies, butterflies and their shared library."
owners = ["octocat"]
icon = "icons/sunrise_creatures.png"
members = [
  { id = "creature" },
  { id = "fireflies" },
  { id = "butterflies", default = false },
]
```

Members are selected by default unless you set `default = false`. You can add a Markdown `description` and record
numeric `owner_ids` in the same order as `owners`. `cargo sunrise-mod registry check <base> <head> --files <dir>`
checks the proposed registry, including the collection; see the [full rules](../reference/registry-index.md#collections).

To add a new mod, list it in the registry, then add its id to `members` in one collection pull request. If you own both
the collection and the mod, this is a routine membership edit; changes to the page, defaults or owners wait for review.

## Integrity and takedowns

- **Integrity.** Every download is checked against the SHA-256 the index lists before the file reaches the game
  folder. Unsigned local files stay allowed, with an "unsigned" mark.
- **Takedowns.** A file the registry takes down (`revoked.toml`) is refused and turned off where installed. What a
  listed project agrees to is in the registry's `MOD-POLICY.md`.

Once a project is published, players can rate it on its page after signing in with GitHub. A player can change their
rating later. Owners can't rate their own project, and nobody sees who rated.
