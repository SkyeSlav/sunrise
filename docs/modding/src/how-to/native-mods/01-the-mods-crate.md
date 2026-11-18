# The mod's crate

The root `Cargo.toml` and `src/` form the mod's crate. It depends on `sunrise-mod-api` (lib
`mod_api`), never directly on the game. The launcher supplies the pinned SDK and declared
[mod dependencies](../../concepts/dependencies.md).

**Registry dependencies belong to the mod.** A mod may use crates.io packages that the game does
not use, or different versions of packages the game uses. Each package must have an exact version
and SHA-256 checksum in the mod's registry dependency lock, stored as `Cargo.lock` beside its
`Cargo.toml`. Keep this file in version control and include it in the mod package. Changing it
changes the native build's cache identity.

Generate it explicitly after choosing your registry dependencies:

```sh
sunrise-launcher native-lock path/to/mod
# To deliberately resolve fresh pins and replace an existing lock:
sunrise-launcher native-lock path/to/mod --update-lock
```

The launcher consumes this **registry dependency lock** during staging. Ordinary Cargo may not
resolve the original pinned SDK declaration offline. The command creates an inert copy
of the manifests, removes only SDK dependencies naming the exact pinned Sunrise repository and
revision, and resolves registry metadata. It does not copy or execute mod sources, build scripts
or proc macros. It does not modify the original manifests or Cargo configuration. Existing pins
are not replaced without `--update-lock`; a refused input leaves the existing lock untouched.
A starter that uses only the launcher-owned SDK does not need registry pins until it adds a
registry dependency.

Builds consume an existing lock; they never silently generate it or repin a missing dependency.
The launcher may resolve registry **index metadata** online using an inert manifest whose
requirements are exact pins. Before fetching crate archives, every resulting identity and
checksum must match a supplied lock. Cargo checks downloaded archives against those checksums.
Only after this fetch does the launcher resolve the actual staged workspace **offline**.

The game's own packages never change because of a mod. The game ships prebuilt, and a mod's registry
packages are compiled for the mod and linked into the mod's library, so a package both use exists
twice: the features and dependency versions of the mod's copy are the mod's own. What a build checks
is that every registry package it compiles is pinned, by version and checksum, in the game's lock or
in the lock of a mod in the build.

All native mods of an instance compile in one Cargo workspace, which resolves one version of a
package for every mod that uses it. When two mods' locks pin different versions that both their
requirements allow, both mods are built on one of the two. So a mod may be built on another
semver-compatible version of a dependency than the one its author locked; a requirement that must
not move is written as an exact one (`dtoa = "=1.0.9"`).

`native-lock` starts from the game's `Cargo.lock`, which it downloads once from the pinned game
commit. A package the game also locks takes the game's version where the mod's requirements allow
it, so mods made for one game build agree on those packages; the mod's other packages take the newest
version that fits. A requirement the game's version can't meet (`libc = "0.2.999"`) gives the mod
its own version of that package.

A package the SDK builds from its own patched source (the `[patch]` table of the SDK's
`engine/Cargo.toml`) is used in that form only: at the SDK's version a mod's graph gets the SDK's
source, never the registry's. A native library with conflicting providers cannot be silently
substituted.

Git dependencies other than the proven launcher-owned SDK declaration, alternate registries,
paths escaping the mod, linked dependency paths, `[patch]`, `[replace]`, and mod-owned Cargo source
configuration are refused. Local crates inside the mod are traversed without copying their code
into the registry resolver. Registry dependencies remain native code: their build scripts and
proc macros execute during compilation, and the resulting mod runs with your rights. Trust their
authors as well as the mod's author.

Before compiling these dependencies, the launcher shows how many registry crates the mod adds and
names the reachable build scripts and proc macros. Declining stops before any mod dependency is
compiled. Native-code permission is also required; downloading a checksummed crate
does not make its code trusted.

For native builds, Cargo compiles the mod's registry graph and the final wrapper
against immutable SDK binaries. Registry dependencies never cause an automatic source rebuild.
The SDK must include its locked graph and runtime native-library inventory; SDKs without
that information refuse registry builds. The launcher preserves the compiler/cache wrapper chain
and uses its pinned Rust compiler.

**SDK crate identities are not interchangeable.** A registry-built copy of a trait or type is not
the SDK's copy, even at the same version. For example, passing an SDK value implementing the SDK's
`serde::Serialize` to a registry-built `serde_json` can fail with Rust E0277. The launcher reports
that boundary; it does not replace either library or rebuild the game to hide it. Keep private
registry types behind your mod's API-facing values.

Runtime native libraries emitted by registry builds are collected only from managed build and
Cargo directories. Host-only build tools and proc macros are not packaged into the game. Conflicting
native providers or output filenames, escaping outputs, relative native search paths, and bundled
framework outputs are refused explicitly. System libraries remain system dependencies. These
restrictions do not trigger a source fallback.

## Build restrictions

A mod's own build script is refused with `E_NATIVE_BUILD_SCRIPT`: ship generated source instead.
`E_NATIVE_FEATURES` refuses mod feature declarations and custom SDK feature requests; use the
pinned SDK's feature set. These restrictions do not prohibit the separately approved build scripts
of registry dependencies.

The launcher adds a small generated module to the crate's root, the library entry that the game
loads, and that module exports two `#[unsafe(no_mangle)]` functions. A crate root may say
`#![deny(unsafe_code)]`, which the module overrides for itself alone, but not
`#![forbid(unsafe_code)]`, which nothing can override: such a mod is refused by name before it
compiles.
