# Your first content mod

A content mod adds blocks, items, recipes, loot, models and textures with data files alone. In this page you
make one block, see it in the game, and pack it as a `.smod`.

You need Rust 1.95 or newer (`cargo`). To play the mod you also need a Sunrise game and resources prepared from your
own Minecraft: Java Edition 26.3 (step 4).

## 1. Install the mod tool

Follow [Install the mod tool](install-tool.md). It adds the `cargo sunrise-mod` command.

## 2. Make the project

```sh
cargo sunrise-mod new glowing-blocks
cd glowing-blocks
```

The tool turns the folder name into the mod id (`glowing_blocks`) and writes a complete mod with one block:

```text
glowing-blocks/
├── sunrise-mod.toml                                   # the manifest: id, name, version, api, capabilities
├── assets/glowing_blocks/
│   ├── blockstates/glowing_amethyst.json
│   ├── models/block/glowing_amethyst.json
│   ├── items/glowing_amethyst.json                    # the item model
│   └── lang/en_us.json                                # the block's name
└── data/
    ├── glowing_blocks/sunrise/block/glowing_amethyst.json   # the block itself
    ├── glowing_blocks/loot_table/blocks/glowing_amethyst.json
    ├── glowing_blocks/recipe/glowing_amethyst.json
    └── minecraft/tags/block/mineable/pickaxe.json
```

The block declaration starts from a vanilla block and changes what differs:

```json
{
  "template": "minecraft:amethyst_block",
  "properties": { "hardness": 2.0, "light": 7 }
}
```

The manifest declares the capability `registries`: a mod needs it to add blocks, items, mob effects or entity
types to a world.

## 3. Build it

```sh
cargo sunrise-mod build
```

This validates the content with the same checks the game uses and writes `target/sunrise-mod/glowing_blocks.smod`,
the file you share.

## 4. Play it

```sh
cargo sunrise-mod run --game <path to sunrise-client> -- --resources <folder>
```

`run` builds the mod and starts that game with it always on. Everything after `--` goes to the game.

- `sunrise-client` is the game, from the download for your computer in the
  [Nightly release](https://github.com/sunrise-org/sunrise/releases/tag/nightly), or one you built yourself.
- `--resources` names resources prepared from your own Minecraft: play 26.3 once from the Minecraft Launcher, then
  make them with `sunrise-prepare --prepare --output <folder>` (`sunrise-prepare` is in the game's download). The tool
  never downloads or bundles Mojang's files.

Open a world and craft the block from an amethyst block and glowstone dust. The game reads content once, at
start-up: after you edit a file, run the command again.

To play it in a game folder you already use, copy `glowing_blocks.smod` into that folder's `sunrise-mods/` (the game
folder is the one that holds `options.txt`; create `sunrise-mods/` if it is missing) and turn it on with the line
`mod.glowing_blocks:true` in `sunrise-mods.txt` there ([Installing and using mods](../players/installing-mods.md)).

## 5. Check it before you publish

```sh
cargo sunrise-mod check target/sunrise-mod/glowing_blocks.smod
```

`check` runs what the mod registry runs on a submission ([Publishing your mod](../how-to/publish.md)).

## Next

- [Content mods](../how-to/content-mods.md): every kind of content file, from blocks to world generation.
- [The manifest and the package](../reference/manifest.md): every manifest field and package rule.
- [Publishing your mod](../how-to/publish.md): signing and the registry.
