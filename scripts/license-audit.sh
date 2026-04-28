#!/usr/bin/env bash
# Dependency license audit for CADIS.
# Requires: cargo install cargo-deny
set -euo pipefail

if command -v cargo-deny &>/dev/null; then
  echo "Running cargo deny check licenses..."
  cargo deny check licenses
elif command -v cargo-license &>/dev/null; then
  echo "Running cargo license..."
  cargo license --all-features --avoid-build-deps
else
  echo "ERROR: Install cargo-deny (preferred) or cargo-license." >&2
  echo "  cargo install cargo-deny" >&2
  exit 1
fi

echo "License audit passed."
