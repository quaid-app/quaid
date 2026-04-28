#!/bin/sh
set -eu

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$repo_root" ]; then
  echo "error: run this from inside the quaid repository" >&2
  exit 1
fi

cd "$repo_root"

if [ ! -f ".githooks/pre-push" ]; then
  echo "error: expected .githooks/pre-push to exist" >&2
  exit 1
fi

chmod +x .githooks/pre-push 2>/dev/null || true
git config --local core.hooksPath .githooks

if ! git config --local --get quaid.protectedBranches >/dev/null 2>&1; then
  git config --local quaid.protectedBranches "main master"
fi

printf '%s\n' "Installed repo hooks:"
printf '  core.hooksPath=%s\n' "$(git config --local --get core.hooksPath)"
printf '  quaid.protectedBranches=%s\n' "$(git config --local --get quaid.protectedBranches)"
printf '%s\n' "Protected-branch pushes to main/master now fail locally before Git contacts origin."
