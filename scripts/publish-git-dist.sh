#!/usr/bin/env bash
# Publishes the built web package (web/ + web/pkg) as one installable commit on the
# `dist/game-audio` branch. Package managers install Git dependencies from a repository
# root, so the distribution commit holds exactly the package contents at its root.
#
# The branch only fast-forwards (each commit's parent is the previous tip) and an
# unchanged package adds no commit, so pinned commits stay reachable.
#
# Usage: scripts/publish-git-dist.sh [--push]   (run scripts/build-web.sh first)
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
branch="dist/game-audio"
push=false
[[ "${1:-}" == "--push" ]] && push=true

[[ -f web/pkg/game_audio_wasm_bg.wasm ]] || { echo "run scripts/build-web.sh first" >&2; exit 1; }

stage="$(mktemp -d)"
index="$(mktemp)"
trap 'rm -rf "$stage" "$index"' EXIT
(cd web && bun pm pack --quiet --destination "$stage" >/dev/null)
tar -xzf "$stage"/*.tgz -C "$stage"
rm "$stage"/*.tgz
cp LICENSE-MIT LICENSE-APACHE "$stage/package/"
cp README.md "$stage/package/README.md"

git fetch --quiet origin "$branch" 2>/dev/null || true
parent="$(git rev-parse --quiet --verify "origin/$branch^{commit}" || true)"

rm -f "$index"
tree="$(GIT_INDEX_FILE="$index" GIT_WORK_TREE="$stage/package" git add --all . >/dev/null \
  && GIT_INDEX_FILE="$index" git write-tree)"

source_commit="$(git rev-parse HEAD)"
if [[ -n "$parent" && "$(git rev-parse "$parent^{tree}")" == "$tree" ]]; then
  commit="$parent"
  echo "package unchanged; keeping $commit"
else
  commit="$(git -c user.name="game-audio dist" -c user.email="actions@users.noreply.github.com" \
    commit-tree "$tree" ${parent:+-p "$parent"} -m "dist: @moritzbrantner/game-audio from $source_commit")"
  echo "new distribution commit $commit"
fi

if $push && [[ "$commit" != "$parent" ]]; then
  git push origin "$commit:refs/heads/$branch"
fi

mkdir -p target/npm
cat > target/npm/git-dist.json <<JSON
{
  "package": "@moritzbrantner/game-audio",
  "branch": "$branch",
  "source": "$source_commit",
  "commit": "$commit",
  "specifier": "github:moritzbrantner/game-audio#$commit"
}
JSON
cat target/npm/git-dist.json
