#!/bin/sh
# Far terrain (LOD) deletion, step 1: the files that are far terrain only.
# Run from the repository root:  sh tools/delete_far_terrain_files.sh
# Each path is whole-file far terrain (no vanilla path lives in it); the shared
# files are edited by hand in the commits after this one.
set -eu
git rm -q -r \
  crates/sunrise-client-render/src/lod.rs \
  crates/sunrise-client-render/src/lod \
  crates/sunrise-client-render/src/gfx/lod.rs \
  crates/sunrise-client-render/src/gfx/sourced_geometry.rs \
  crates/sunrise-client-render/src/gfx/sourced_geometry \
  crates/sunrise-client-render/src/bluemap.rs \
  crates/sunrise-client-render/src/server_terrain.rs \
  crates/sunrise-client/src/lod_net.rs \
  crates/sunrise-client/src/lod_net \
  crates/sunrise-client/src/vanillagen/lod.rs \
  crates/sunrise-client/src/save/far_census.rs \
  crates/sunrise-client/src/save/far_regions.rs \
  crates/sunrise-client/src/game/network/lan_far.rs \
  crates/sunrise-client/src/game/world/far_terrain_edits.rs \
  crates/sunrise-world/examples/lod_grid_bench.rs \
  tools/server_lod_e2e.py \
  tools/ci/ignored-tests/daily-sunrise-client-render-sourced-geometry.txt
