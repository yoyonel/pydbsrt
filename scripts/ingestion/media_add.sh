#!/usr/bin/env bash
set -euo pipefail
./target/release/dbsrt media:add "$@"
