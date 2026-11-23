# Hosts

The compiled slices are built for these computers, the game's and, where listed, the dedicated server's:

{{#sdk-hosts}}

Building a native mod requires a matching prebuilt game slice. Sunrise does not publish game or SDK builds for
Intel Macs, Windows on ARM or musl Linux. Dedicated server slices are available only for Linux x86-64.

Authoring a mod can use the API source in `.sunrise/sdk/` on any host where Cargo runs. Running native
`cargo sunrise-mod check` or `cargo sunrise-mod build` requires matching compiled SDK slices for the target host.
