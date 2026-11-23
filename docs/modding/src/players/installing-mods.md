# Installing and using mods

Mods are off until you install and turn them on. A fresh Sunrise install has no mods folder and plays exactly
like vanilla 26.3.

## Installing a mod

Install a mod from the launcher's **Mods** page or from the game's **Options > Mods > Browse Mods**, or copy its
file by hand into the game folder (the folder that holds `options.txt`):

| Mod | Goes in | What happens next |
| --- | --- | --- |
| Content mod (`.smod`, or a folder holding `sunrise-mod.toml`) | `sunrise-mods/` | Read when the game starts |
| Native mod (`.smod` with Rust source) | `sunrise-native-mods/` | The launcher builds it for that instance ([Native mods](../how-to/native-mods.md)) |

**Browse Mods** in the game lists content mods only, checked against the same registry as the launcher. Native mods
install from the launcher or by hand.

Fabric and NeoForge jars do not run in Sunrise: they target Minecraft's Java runtime, which Sunrise does not have.
The data and assets inside a jar can be carried over; see
[Porting Java modpacks](https://github.com/sunrise-org/sunrise/blob/main/docs/modpacks.md).

## Signed, reviewed releases

A mod released to the [Sunrise mod registry](https://github.com/sunrise-org/sunrise-mods) comes with these checks:

- **Signed by where it was built.** Each release file carries a GitHub artifact attestation. It records the mod's
  GitHub repository, by its permanent numeric ID, and Sunrise's shared release workflow, which built the file from a
  tagged commit of that repository. Authors hold no signing keys, so the signature proves which repository a file
  came from. It says nothing about the person behind it. That repository is the mod's publisher, and only a reviewed
  change to the registry can move a mod to another one.
- **Reviewed for malware.** Before a release is listed, automatic checks verify its files and signature, scan its
  source and compile it. A reviewer then reads the code for malware, dangerous patterns and hidden or obfuscated
  code. The review does not judge whether a mod works well. Report bugs on the mod's own repository.
- **Listed for the builds it fits.** The registry lists a release only for the game builds it is known to fit: the
  stable releases and the newest Nightly and Dev builds. A Nightly or Dev release names the exact build it was made
  for. A native mod is released as source, which is built on your computer for the exact game you run.
- **One signed list.** The registry publishes its releases as one list signed with its own key, and a takedown
  there removes a release or a whole project.

To check a downloaded release yourself, keep its `.sigstore.json` file beside it and run
[`cargo sunrise-mod check <file.smod>`](../reference/sdk/05-commands.md#check-a-released-package). This
reports the repository and workflow that signed the file, without running any of its code.

## Turning mods on

A newly installed mod is off.

- A mod installed from the launcher's **Mods** page turns on there, or with `sunrise-launcher mods enable <project>`.
- A mod you copied in by hand turns on with a line `mod.<id>:true` in `sunrise-mods.txt`, beside `options.txt`.

A content mod that holds only assets and reloadable data (textures, models, sounds, languages, recipes, tags,
loot, advancements and functions) starts working as soon as it is installed from Browse Mods: the game reloads
its resources, then the world's data as `/reload` does, LAN guests included. A mod that adds registry entries
or changes world generation, or that carries Java jar layers, waits for a restart. On someone else's server, all
content waits: the server owns its data.

When two packages list the same extra namespace, the first admitted package owns it. The game leaves the later
package's content out and logs why. If a package lists another package's ID as an extra namespace, startup stops
with an error instead. Turn one of the conflicting mods off.

Benchmarks (`--benchmark`) never load mods.

## The Mods screen

**Options > Mods** lists Minecraft first (and Sunrise, in the official game with its Sunrise layer on), then each
installed mod with its icon, name and version. Once a package is installed, the title screen also carries a
full-width **Mods** button below Minecraft Realms, and it goes away again when the last package leaves. Search
the list and sort it A–Z or Z–A. Select a mod to read its
id, version, license, authors, credits and description:

- **Homepage** and **Issues** open the mod's links, after asking you to confirm.
- **Config** opens the settings page of a mod that registers settings.
- A mod that is installed but off says so. A mod that could not load shows the reason in red.

The screen has no on/off switch; `sunrise-mods.txt` decides which mods run.

## Making a mod

[Install the mod tool](../start/install-tool.md), then follow [Your first native mod](../start/first-mod.md) or
[Your first content mod](../start/first-content-mod.md).

## Sunrise's own mods

Sunrise's official mods are native mods, each in its own repository under
[sunrise-org](https://github.com/sunrise-org). The registry every mod is listed in is
[sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods).
