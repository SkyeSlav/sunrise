# Hosts

The compiled slices are built for these computers, the game's and, where listed, the dedicated server's:

{{#sdk-hosts}}

Building a native mod requires a matching prebuilt game slice. Sunrise does not publish game or SDK builds for
Intel Macs, Windows on ARM or musl Linux. Dedicated server slices are available only for Linux x86-64.

Writing and checking a mod does not require a compiled slice. The API source in `.sunrise/sdk/` is the same on every
host, so you can use `cargo sunrise-mod` to create and check a mod wherever Cargo runs.
