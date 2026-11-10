#!/usr/bin/env bash
# scripts/coupling_relabel.sh: Thin orchestrator for dbsrt coupling:relabel CLI subcommand
# Strict adherence to Pure Taskfile Orchestrator (Zero inline logic)
set -euo pipefail

exec target/release/dbsrt coupling:relabel "$@"
