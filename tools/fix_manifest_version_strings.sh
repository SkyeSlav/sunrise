#!/bin/sh
# One-off, kept for the record: `Manifest.version` became a semver::Version, so callers that
# passed `manifest.version.clone()` where a String is wanted pass `manifest.version.to_string()`.
set -e
rg -l 'manifest\.version\.clone\(\)' crates --glob '*.rs' \
  | rg -v '^crates/(sunrise-mod-package|sunrise-port-pack)/' \
  | xargs sed -i 's/manifest\.version\.clone()/manifest.version.to_string()/g'
