# Measuring a mod's calls

The server's `/modprofile` command measures how long native mods spend in their hooks and Parts. It needs
permission level 3 and is off until you start it.

1. `/modprofile start` starts a measurement. Starting twice reports an error.
2. Play through what you want to measure.
3. `/modprofile stop` ends it and saves `profiles/modprofile-<timestamp>-<counter>.json` in the game folder.
   Stopping with no measurement running reports an error.
4. `/modprofile report` prints the last finished summary, without saving a file or stopping a running measurement.

Replies go only to the player who ran the command. The same controls work through
`sunrise-ctl command modprofile start` (and `stop` or `report`).

## The profile file

Open the file in [Firefox Profiler](https://profiler.firefox.com) and look at its Marker Table. Sunrise never uploads
it.

- The file is written atomically and is at most 8 MiB. If saving fails, the command says so and the summary stays
  available to `report`.
- Each marker names a mod, a hook or Part and holds call counts, exact timings in nanoseconds and log2 histograms. It
  never holds arguments, player data or world data.
- Timings are inclusive (an outer call includes the calls inside it) and add up over the whole process: they are
  totals, not a trace of each call or a sampling profile.
- Only native mods' calls are measured.

## Named zones in a timeline

Use `mod_api::profiling::zone` to measure a specific part of your native mod on either client or server.
It needs no hook or `Host`. Zones appear under any enclosing engine or mod zone on the same thread.
Bind the returned guard to a name: it closes when the binding drops, including on early return.
Keep nested guards on the thread that opened them and let them drop in reverse order.

```rust
use mod_api::profiling;

fn process_batch(batch_size: usize) {
    let _batch = profiling::zone(&format!("example: batch ({batch_size} items)"));
    let _prepare = profiling::zone("example: prepare");
    if batch_size == 0 {
        return; // Drops _prepare, then _batch.
    }
    // Prepare the batch here.
    drop(_prepare);
    // Continue processing within _batch.
}
```

Open a capture with `sunrise-ctl profile start --timeline` or `sunrise-ctl trace FILE [SECONDS]`.
On a dedicated server, use `sunrise-report --timeline` in the console. Without an active timeline the guard
is inert, and opening a zone never starts a capture. `/modprofile` does not include these zones.
Labels are saved verbatim and will be visible to anyone you share a capture with. Do not include private data.
