# Group your mods into a collection

A collection shows several of your registered mods as one card in the launcher's Discover. A player opens it, ticks
the members they want and installs them together. Each member stays its own project, with its own page, releases and
updates. A collection has no file or version of its own.

## Who can make one

Every member must be published by the same GitHub account: the account that owns the repository each project is
registered to. Only that publisher curates the collection. The registry refuses a pull request that adds, changes or
removes a collection unless its author is listed in `owners` of every member's `project.toml`, both before and after
the change, as `owners` stands on the registry's `main` branch. You can't add someone else's mod to your collection.

## Find your project IDs

Each registered package has a permanent eight-character project ID. You can find it in either of these places:

- `registry/repositories/<repository ID>.toml` on
  [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods/tree/main/registry/repositories), where
  each `[[package]]` lists its `project`;
- the folder name under `registry/projects/` whose `project.toml` has your `mod` ID.

## Write the collection

Pick a new collection ID of eight characters from `0-9`, `A-Z` and `a-z`, for example with
`LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 8`. It is permanent, and it can't be a project's ID or another
collection's ID. In a fork of sunrise-org/sunrise-mods, create the folder `registry/collections/<collection ID>/` with
exactly two files:

- `collection.toml`:

  ```toml
  id = "Qm4tZ81c"
  name = "Wayfinding"
  summary = "The map and its markers."
  description = """
  Optional longer text shown when a player opens the collection.
  """

  [[member]]
  project = "Ab3dE9xQ"

  [[member]]
  project = "Zz91kLmN"
  default = false
  ```

  `id` matches the folder name. `name` and `summary` must not be blank, and `description` is optional. Each
  `[[member]]` names a registered `project`. Members appear in the order you list them. `default` says whether a
  member starts ticked when a player opens the collection, and is `true` when you leave it out. You need at least two
  different members, and at least one of them selected by default. Any other key is refused.
- `icon.png`: a square PNG of at most 512×512 pixels and 256 KiB, the same rules as a mod's icon.

## Open the pull request

Commit with a sign-off (`git commit -s`), push to your fork and open a pull request on sunrise-org/sunrise-mods from
the GitHub account that owns the members. The `registry-v2` check runs the registry's checker on the folder and
confirms that you own every member. A maintainer reviews the pull request and merges it. Change or remove a
collection the same way.

## What players see

- The collection is published once at least two of its members are listed, meaning each has a reviewed release, and
  at least one of those listed members is selected by default. Members without a release yet are left out until they
  have one.
- Discover shows the collection's card in place of its members, with "Show all" on or off. Without "Show all", it
  shows the card only when at least one member fits the player's game build. With a category chosen, it shows no
  collections.
- A search finds the collection by its name, summary or publisher, or by any member's name. A search also shows the
  member itself.
- Install installs the ticked members that fit the build and aren't installed yet. Each is then updated and removed
  on its own.
