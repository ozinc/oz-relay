#!/usr/bin/env bash
# FIX #9: Safe worktree cleanup — checks for active processes before deleting.
# Only removes worktrees older than 2 hours that have no running builds.

set -euo pipefail

WORKTREE_DIR="/opt/arcflow/.relay-worktrees"

if [ ! -d "$WORKTREE_DIR" ]; then
    exit 0
fi

for dir in "$WORKTREE_DIR"/*/; do
    [ -d "$dir" ] || continue

    # Skip if modified less than 2 hours ago
    if [ "$(find "$dir" -maxdepth 0 -mmin -120 2>/dev/null)" ]; then
        continue
    fi

    # Skip if any process is using this directory
    if lsof +D "$dir" >/dev/null 2>&1; then
        echo "skipping $dir — active processes detected"
        continue
    fi

    echo "removing orphaned worktree: $dir"
    rm -rf "$dir"
done
