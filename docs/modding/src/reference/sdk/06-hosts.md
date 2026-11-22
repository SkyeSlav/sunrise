# Hosts

The compiled slices are built for these computers, the game's and, where listed, the dedicated server's:

{{#sdk-hosts}}

On any other computer the launcher builds no native mods and says `E_NATIVE_HOST: no prebuilt game supports this
computer; use a supported host`. Sunrise publishes no game or SDK for Intel Macs, Windows on ARM or musl Linux
([#4556](https://github.com/sunrise-org/sunrise/issues/4556)), and a dedicated server takes native mods on Linux x86-64
only.

Writing a mod needs no compiled slice: the API source in `.sunrise/sdk/` is the same on every computer, so
`cargo sunrise-mod` writes and checks a mod on any computer `cargo` runs on. Only building it into the game needs the
slices of the computer that plays it.
