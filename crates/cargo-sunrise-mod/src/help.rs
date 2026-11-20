//! What `cargo sunrise-mod --help`, `help all`, `help <command>` and `<command> --help` print: one
//! table, every command once.

struct Command {
    name: &'static str,
    /// One of the commands a mod author uses every day: listed by plain `--help`.
    everyday: bool,
    usage: &'static str,
    summary: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "new",
        everyday: true,
        usage: "new <path> [--name \"Name\"] [--template content|native [--sunrise <build>]]",
        summary: "Makes a mod project in <path>; a native one is pinned to the current Nightly's SDK unless --sunrise names another build.",
    },
    Command {
        name: "build",
        everyday: true,
        usage: "build [--update-lock]",
        summary: "Packs the mod into target/sunrise-mod/<id>.smod; a native mod is compiled first, and --update-lock (native only) resolves its Cargo.lock again.",
    },
    Command {
        name: "run",
        everyday: true,
        usage: "run [--game <sunrise-client>] [--resources <folder>] [--jars <dir>] [-- <game args>]",
        summary: "Builds the mod, then starts the game with it; --game names the game for a content mod, --resources the prepared resources for a native mod (which runs on its pinned SDK's game), and --jars selects a folder of original Java jars named by SHA-512.",
    },
    Command {
        name: "check",
        everyday: true,
        usage: "check [--uses] | check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json]",
        summary: "Checks the mod in this folder, or a file as the registry would; --uses lists what the mod uses of its pinned build, and a native mod's Rust code is checked on this computer.",
    },
    Command {
        name: "update",
        everyday: true,
        usage: "update [--sunrise <build>]",
        summary: "Repins a native mod to another game build (the current Nightly unless --sunrise names one) and reports what that breaks; a content mod pins no build.",
    },
    Command {
        name: "hooks",
        everyday: true,
        usage: "hooks <text> [--all]",
        summary: "Searches the pinned build's hooks, or with --all every item of its API, offline.",
    },
    Command {
        name: "publish",
        everyday: false,
        usage: "publish [--out <dir>] [--registry <checkout>] [--summary <text>] [--owner <login>]… [--categories a,b] [--tags a,b] [--changelog <text>] [--key <public key> --signature <file.minisig>]",
        summary: "Builds the mod, then writes its registry entry, the staged file and the pull request text into a folder; it uploads nothing.",
    },
    Command {
        name: "internals",
        everyday: false,
        usage: "internals sync | internals add <path>… [--kind <kind>]",
        summary: "Writes the game internals a native mod names into sunrise-mod.toml, found by checking its code or taken from the pinned build's catalog.",
    },
    Command {
        name: "sdk",
        everyday: false,
        usage: "sdk diff <build> <build>",
        summary: "Lists what changed in the API surface between two builds.",
    },
    Command {
        name: "fits",
        everyday: false,
        usage: "fits [<file.smod>] [--build <name>]…",
        summary: "Says whether the mod fits each build; exits 0 when all do, 1 when one does not and 2 when that cannot be told.",
    },
    Command {
        name: "registry",
        everyday: false,
        usage: "registry build|review|check|list|release|page",
        summary: "The registry repository's CI steps.",
    },
    Command {
        name: "use-jar",
        everyday: false,
        usage: "use-jar <path>",
        summary: "Lists an exact original Java jar as a passive asset and data source and caches it for runs.",
    },
    Command {
        name: "native-build",
        everyday: false,
        usage: "native-build < plan.json",
        summary: "Compiles native mods against an unpacked SDK from a JSON plan on standard input (reference/sdk/08-native-build.md).",
    },
    Command {
        name: "native-check",
        everyday: false,
        usage: "native-check < plan.json",
        summary: "Says how native mods stand against one build before anything compiles.",
    },
    Command {
        name: "native-discover",
        everyday: false,
        usage: "native-discover < plan.json",
        summary: "Finds the game internals a native mod names, or the pinned game's build data.",
    },
    Command {
        name: "native-lock",
        everyday: false,
        usage: "native-lock < plan.json",
        summary: "Writes a native mod's registry lock from its pinned game's build data.",
    },
    Command {
        name: "native-layout",
        everyday: false,
        usage: "native-layout < plan.json",
        summary: "Checks a local prebuilt game and lays it out as an SDK's slices.",
    },
];

/// The short list `cargo sunrise-mod --help` prints.
pub fn short() -> String {
    let mut out = String::from("usage: cargo sunrise-mod <command> [options]\n\n");
    for command in COMMANDS.iter().filter(|c| c.everyday) {
        let first = command.summary.split(';').next().unwrap_or(command.summary);
        out += &format!("  {:<8}{}\n", command.name, first.trim_end_matches('.'));
    }
    out += "\n`cargo sunrise-mod <command> --help` says more about one command.\nmore: cargo sunrise-mod help all\n";
    out
}

/// Every command's usage, one per line: `help all`.
pub fn all() -> String {
    COMMANDS
        .iter()
        .map(|c| format!("cargo sunrise-mod {}\n", c.usage))
        .collect()
}

/// One command's usage and what it does, when `name` is a command.
pub fn of(name: &str) -> Option<String> {
    let command = COMMANDS.iter().find(|c| c.name == name)?;
    Some(format!(
        "usage: cargo sunrise-mod {}\n\n{}\n",
        command.usage, command.summary
    ))
}

/// The text for a `help` command's arguments, or why it names no command.
pub fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None => Ok(short()),
        Some("all") => Ok(all()),
        Some(name) => of(name).ok_or_else(|| unknown(name)),
    }
}

/// The error for a word that is not a command.
pub fn unknown(name: &str) -> String {
    format!("{name:?} is not a cargo sunrise-mod command; `cargo sunrise-mod --help` lists them")
}

/// Whether `args` (a command's, after its name) ask for its help: `--help` or `-h` right after the
/// command. Later it may be an option's value (`--summary --help`) or meant for the game (`run -- --help`).
pub fn asked(args: &[String]) -> bool {
    args.first().is_some_and(|a| a == "--help" || a == "-h")
}
