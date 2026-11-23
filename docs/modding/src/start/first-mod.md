# Your first native mod

Make a mod that says hello in chat when a player joins. Unlike a [content mod](first-content-mod.md), which adds
things from data files, this mod reacts to something that happens in the game. You'll use a **hook**: a Rust
method Sunrise calls at a particular moment. There is still [one project folder and one `.smod` file to
share](../concepts/how-mods-work.md).

You'll start with a working greeting, then check, build and run it with `cargo sunrise-mod`. Have
[rustup](https://rustup.rs) ready so the mod tool can install the Rust version the SDK needs. You can check and
build before you have the [prepared resources](#prepared-resources) needed to play.

## 1. Install the mod tool

Follow [Install the mod tool](install-tool.md) for your computer. When `cargo sunrise-mod` prints its usage, come
back here. This page needs a tool that answers `cargo sunrise-mod --version` with a commit; one that prints the usage
instead is older, and the [newest tool from main](install-tool.md#newest-tool-from-main) replaces it.

## 2. Make the project

In your terminal, start in the folder where you keep your projects. Choose the native template to get a mod
with a Rust source file:

```sh
cargo sunrise-mod new greeter --template native
cd greeter
```

The tool records the current Nightly's exact Sunrise build in `sunrise-mod.toml` and fetches that build's
signed [SDK](../reference/sdk.md), which gives your code the API it can call. To start from a different build,
pass [`--sunrise <build>`](../reference/sdk/03-in-a-project.md#build-names) when you make the project. Open
`greeter` in your editor:

```text
greeter/
├── sunrise-mod.toml     # the manifest; [native] pins the exact Sunrise build
├── Cargo.toml           # depends on sunrise-mod-api in .sunrise/sdk/, never on the game
├── src/lib.rs           # the hooks
├── assets/
├── data/
└── .sunrise/sdk/        # the pinned SDK: fetched by the tool, never committed or packed
```

After a fresh clone, run `cargo sunrise-mod check` once before opening the project in an editor: it fetches
`.sunrise/sdk/` again. To move the project to a newer Nightly later, use `cargo sunrise-mod update`.

## 3. Read the hook

`src/lib.rs` implements one method of `mod_api::Hooks`:

```rust,ignore
{{#include ../../examples/src/greeter.rs:mod}}
```

- Each method of `Hooks` is one moment in the game. You implement only the ones you need; the game calls
  your mod only for those.
- `#[mod_api::sunrise_mod]` marks the one `impl Hooks` at the crate root and subscribes the mod to exactly the
  methods it implements.
- The `host` is your handle on the game. A server hook gets a `ServerHost`, a client hook a `ClientHost`, so a call
  on the wrong side does not compile.
- The `event` carries the moment's data. Values you may change are writable fields of the event.

[Writing hooks](writing-hooks.md) explains these rules in full.

## 4. Check that it compiles

```sh
cargo sunrise-mod check
```

`check` validates the package and runs `cargo check` against the pinned SDK's API source for this computer.
Look for the summary naming your mod, what it uses from the API, and the build it was made for. Run `check`
after an edit to catch mistakes without linking against the game.

## 5. Build against the prebuilt game

```sh
cargo sunrise-mod build
```

`build` packs the `.smod` and then compiles the crate against the prebuilt game of the pinned build, and ends with
the library it made and the build log's path:

```text
Downloading runtime-x86_64-unknown-linux-gnu.tar.xz (71 MB), once for every project
…
Compiling greeter (1/1)
Built …/sunrise-mod/work/greeter/prebuilt-build/…/libmod_greeter.so
Build log: …/sunrise-mod/work/greeter/build.log
```

The first build downloads the pinned build's compiled slices, checks Sunrise's signature on their checksums and
unpacks them into the tool's cache, shared by every project: `~/.cache/sunrise-mod` on Linux (`$XDG_CACHE_HOME`),
`~/Library/Caches/sunrise-mod` on macOS, `%LOCALAPPDATA%\sunrise-mod` on Windows, or the folder
`SUNRISE_MOD_CACHE` names. Delete that folder to clear it; the next build fetches again. It also finds the Rust the SDK
names: through `rustup` (installing it with `--profile minimal` when missing), or the `rustc` on `PATH` when that is
exactly the version; it never builds with another compiler.

A mod with crates.io dependencies is compiled with them, as `cargo build` would, and `build` pins them in
`Cargo.lock` the first time. After you change `Cargo.toml`, `cargo sunrise-mod build --update-lock` resolves again.
When a build stops on a package the lock doesn't pin, its error ends with that command. The
[build log](../how-to/native-mods/05-build-log.md) shows the compiler's output when a build fails.

For publishing later, the native template also writes `.github/workflows/release.yml`, `.github/workflows/rebuild.yml`
and their shared workflow and bootstrap action. They stay off until you opt in; you can keep developing against Nightly. See
[Tagged native releases in GitHub Actions](../how-to/publish.md#tagged-native-releases-in-github-actions) for setup.

## 6. Run it

<!-- quickstart-test: skip, needs the reader's game and prepared resources -->
```sh
cargo sunrise-mod run --resources <folder> -- --world Greeter --creative
```

`run` builds, stages the game with your mod in `target/sunrise-mod/run/` and starts it, with its game folder and its
logs inside that folder, so nothing of a player's is touched. The game's `logs/latest.log` includes native mods' and
their dependencies' `log` records through INFO, prefixed with `[target]`. Everything after `--` goes to the game. Join a world;
the greeting appears in chat, and the mod is listed on the game's **Mods** screen. On Linux without a display, start
`run` under `xvfb-run`. If your mod uses [`[[jar]]` originals](../how-to/content-mods.md#java-jars-as-external-assets),
`run` stages them in its development home. Use `--jars <dir>` to choose a different source folder.

### Prepared resources

The game needs resources prepared from your own Minecraft: Java Edition 26.3. The tool never downloads or bundles
Mojang's files. Make them once and name the folder with `--resources`:

1. Have your own copy of Minecraft: Java Edition 26.3 installed. Play it once so the client files are on
   your computer.
2. Download the 26.3 server jar from `minecraft.net/download/server` while 26.3 is the current release,
   otherwise use the address in `downloads.server.url` in your installation's `versions/26.3/26.3.json`.
3. Run `sunrise-prepare --prepare --server-jar <server.jar> --output <folder>`. `sunrise-prepare` is in the game
   download for your computer in the [Nightly release](https://github.com/sunrise-org/sunrise/releases/tag/nightly).
   If your client files are in a non-default location, `sunrise-prepare --help` explains how to point the tool
   at them. For a Prism or MultiMC installation, pass `--prism-dir <path>` instead of `--server-jar`.

## 7. Test it

```sh
cargo test
```

Keep what a hook decides in plain functions and test those: the template's `greeting` has one test, which runs
without the game.

### Without the tool's commands

A build server can run the compile itself: `native-build` takes the plan as one line of JSON on standard input and
prints one JSON event per line. That contract ([`native-build`](../reference/sdk/08-native-build.md)) is what `build`
itself calls.

## Next

- [Writing hooks](writing-hooks.md): the crate root, the hook factory, events and hosts.
- [Hooks and the Host](../concepts/hooks-and-the-host.md) and [Parts](../concepts/parts.md): how a mod reaches
  the game.
- [Native mods: building and shipping](../how-to/native-mods.md): how a native mod reaches players, trust,
  multiplayer and servers.
- [Using an AI agent](../for-agents.md): the same steps on one page for a coding agent.
