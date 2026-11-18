# Commands

The `cargo sunrise-mod` commands that read the SDK. One tool serves every build: it reads each build's data from its
SDK, fetched as [In a mod project](03-in-a-project.md) says, and refuses a schema it doesn't know. Run them in a mod's
folder.

| Command | What it does |
|---|---|
| `new <path> --template native\|wasm [--sunrise <build>]` | Makes a code mod pinned to the current Nightly's build, or the one named, and fetches its SDK. Without `--template` it makes a content mod, which has no SDK. |
| `build`, `build --wasm` | Fetches the pinned SDK when needed, then packs the `.smod` with its [`uses.json`](04-fit.md#usesjson). For a native mod it then compiles the crate against the build's prebuilt game, as [`native-build`](08-native-build.md) does, with no launcher: see [`build` for a native mod](#build-for-a-native-mod). |
| `run [--resources <folder>] [-- <game args>]` | For a native mod, builds, stages the game with the mod in `target/sunrise-mod/run/` and starts it. |
| `check [--uses]` | The package rules, and every render pipeline shader (`assets/*/shaders/pipeline/**/*.wgsl`) composed and validated by the game's own code, failing with `<file>: <reason>` for each refusal ([render pipelines](../../how-to/client/render-pipelines.md)); then, for a code mod, fetches the pinned SDK when needed and runs `cargo check` for this computer and for wasm32, then what the mod uses of its pinned build: `Greeter uses 14 items of the API (6 hooks, 5 methods, 3 types). Made for nightly-2026-11-13 (afe471c).` `--uses` lists each item with the line that brought it in, and how the mod builds each struct it builds with a literal. |
| `hooks <text> [--all]` | Searches the pinned build's hooks offline, by id, doc and their NeoForge, Fabric and Paper names; every word must match. `--all` searches every record. Matches in the id come first, then in a counterpart, then in the doc. |
| `sdk diff <build> <build>` | What changed in the surface from the first build to the second, [classified](02-surface.md#what-a-change-is), with the second build's change notes (why, and what a mod does now) under the changes they name: the notes of the changes since the SDK published before it. It reads only the builds' small files, so it works for builds whose compiled files are pruned. |
| `fits [<file.smod>] [--build <build>]…` | Whether the mod, or a package, fits the current Nightly or each build named: one line per build, `made for`, `fits`, or `does not fit` with the changed items. |
| `update [--sunrise <build>] [--accept <id>]…` | Moves the pin to the current Nightly (or the build named). A native mod first gets what the move breaks in it; a wasm mod gets no report. |

## `build` for a native mod

After packing, `build` fetches the build's `runtime`, `api` and `engine` slices for this computer once, checks the
signed sums and each slice as the SDK fetch does, and unpacks them into the tool's cache, shared by every project:
`$XDG_CACHE_HOME/sunrise-mod` (else `~/.cache/sunrise-mod`) on Linux, `~/Library/Caches/sunrise-mod` on macOS,
`%LOCALAPPDATA%\sunrise-mod` on Windows; `SUNRISE_MOD_CACHE` names another folder. Deleting the folder clears it.
The Rust is the one `sdk.json` names: `rustup`'s, installed with `--profile minimal` when missing, or the `rustc` on
`PATH` when it is exactly that version; otherwise `build` stops and says the version and both ways to get it.
A mod with crates.io dependencies and no `Cargo.lock` gets one; `build --update-lock` resolves again. The compile
is [`native-build`'s](08-native-build.md), in process, with registry crates approved (the mod's own `Cargo.toml`),
for the `client` side. It prints `Built <library>` and the build log's path, and exits non-zero with the error on failure.

`run` additionally needs `--resources <folder>`, prepared resources made from your own Minecraft (`sunrise-prepare
--prepare --output <folder>`). It copies the built game program, its libraries, the `native-mods.json` record naming
the mod's library and the mod's package into `target/sunrise-mod/run/app/`, keeps the game folder and `SUNRISE_HOME`
in `run/game/` and `run/home/`, and starts the program with `--game-dir`, `--resources` and the arguments after `--`.

## `fits`

`fits` exits 0 when every build fits, 1 when one does not, and 2 when it cannot tell: an input it cannot read or
fetch, or a question that doesn't apply (a content mod, a mod made for exactly one build, a package made by an older
tool). A mod's own scheduled CI can run it to learn of a break before players do:

```text
Greeter:
nightly-2026-11-13 (afe471c)  made for
nightly-2026-11-14 (0c1d2e3)  fits
nightly-2026-11-20 (9a8b7c6)  does not fit: 2 items changed
    the hook `player_logged_in` (changed)
    the method `ServerHost::give_item` (removed)
```

## `update`

The API-use report is for native mods. A wasm mod's `update` resolves the build, moves the pin in its Cargo metadata, removes the fetched SDK and reports the new pin, with no compatibility check: it reads neither the old SDK nor the mod's source.

For a native mod, before it repins, `update` lists every item the mod uses that changed between the two builds, the source line that
uses it, for a removed item the closest one in the new build (one with the same counterpart name first, then the
nearest id and doc text), and the change note's `migrate` line when the change has one. A mod made for exactly one
build is only named; its internals and `[[inject]]` declarations are checked as the pin moves, and a change to an
injection's point refuses the update until `--accept <inject id>` takes it.

The same checks without the tool, from a build's `surface.json` and a mod folder, are `sunrise-stage uses` and
`sunrise-stage fits` ([Checking a mod against a build](../../how-to/native-mods/14-checking-a-mod-against-a-build.md)).
