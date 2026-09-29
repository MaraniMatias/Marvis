#!/usr/bin/env bash
# Ships one release: bumps the version, pushes the commit and the tag, waits for the build and
# publishes the draft it leaves behind.
#
# Usage: scripts/release.sh <version>          # v0.2.0 or 0.2.0
#
# The version is bumped with scripts/release.mjs, which owns the manifests; this owns git and the
# GitHub release, which are the two things that cannot be undone from here. That split is why the
# tag is pushed before the build is known to pass: the alternative is a three-run dance between
# bumping and tagging, and the dry run it needs cannot even start until the bumped manifests are on
# the default branch. What this gives up instead is a human pausing between the artifacts being
# uploaded and the release going live, so the pause is conditional on the run instead: nothing is
# published unless every matrix job succeeded, which is also the only way a release exists at all.
#
# If a build fails the tag is already pushed, so nothing was published and the run can be retried
# with `gh run rerun <id>`: the same tag, the same commit, no new version.
#
# Set RELEASE_SKIP_PUBLISH=1 to stop at the draft and publish it by hand.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
die() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }

[[ $# -eq 1 ]] || die "usage: scripts/release.sh <version>     # the whole release
       pnpm release:preview <version>       # report the writes, change nothing"

# The tag is `v<version>` and the manifests hold the version bare, so both spellings are accepted
# here and normalized once.
requested="${1#v}"
[[ "$requested" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)*$ ]] ||
  die "\"$1\" is not a version like 0.2.0 or v0.2.0"
tag="v$requested"

step "Checking this repository can take a release"
for tool in git gh node; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done
gh auth status >/dev/null 2>&1 || die "gh is not authenticated; run: gh auth login"
repository="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"

# A tag from anywhere but the default branch is a commit the gated workflow never validated, and the
# checks it runs are the ones the branch was protected for.
[[ "$(git rev-parse --abbrev-ref HEAD)" == "main" ]] ||
  die "releases are cut from main; you are on $(git rev-parse --abbrev-ref HEAD)"
[[ -z "$(git status --porcelain)" ]] ||
  die "the working tree has uncommitted changes; commit or stash them first"
# Checked on the remote as well as locally: a tag left behind by a failed attempt is not a reason to
# bump the version again, and pushing over it would move a release that may already be public.
git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null && die "$tag already exists locally"
git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1 &&
  die "$tag already exists on the remote"
printf 'repository %s, branch main, tag %s is free\n' "$repository" "$tag"

step "Writing $tag into the manifests, and committing it"
# --no-push: the two pushes below are separate so a failure leaves a commit with no tag rather than
# a tag with no commit on the branch that was checked.
node scripts/release.mjs --bump "$requested" --no-push

step "Pushing the commit and the tag"
git push origin HEAD:main
git push origin "refs/tags/$tag"
release_commit="$(git rev-parse "$tag^{commit}")"

step "Waiting for the Release workflow"
# Filtered by the tagged commit rather than by the tag as a ref: `--ref` is not a flag every gh has
# (`gh run list` takes `--branch` or `--commit`), and the commit is what the run is really about. The
# event filter keeps the CI runs on the same commit out of the answer.
run=""
for _ in $(seq 1 18); do
  run="$(gh run list --workflow release.yml --commit "$release_commit" --event push --limit 1 \
    --json databaseId --jq '.[0].databaseId // empty')"
  [[ -n "$run" ]] && break
  sleep 5
done
[[ -n "$run" ]] || die "no Release run appeared for $tag; check https://github.com/$repository/actions"
echo "run $run: https://github.com/$repository/actions/runs/$run"

# --exit-status is the whole point: a run that fails leaves a tag with no release, and this is where
# the script stops rather than publishing a release built from a job that did not pass.
if ! gh run watch "$run" --exit-status; then
  die "the release run failed; nothing was published. Retry it with:
  gh run rerun $run"
fi

step "Reading the draft the run left behind"
# Read through `gh` rather than a `jq` binary: `gh` is already required, and it already knows how to
# answer these questions, so the script needs nothing else installed.
[[ "$(gh release view "$tag" --json isDraft --jq .isDraft)" == "true" ]] ||
  die "$tag is not a draft, so it was published by something else; stopping before touching it"
assets="$(gh release view "$tag" --json assets --jq '.assets | length')"
[[ "$assets" -gt 0 ]] || die "$tag has no assets, so there is nothing to publish"
gh release view "$tag" --json assets --jq '.assets[].name' | sed 's/^/  /'
url="$(gh release view "$tag" --json url --jq .url)"

if [[ -n "${RELEASE_SKIP_PUBLISH:-}" ]]; then
  step "Leaving it as a draft, as asked"
  echo "$url"
  exit 0
fi

step "Publishing the release"
# `gh release edit` can only set a draft, never clear one, so the release is published through the
# API and then read back to confirm it is no longer a draft.
gh api -X PATCH "repos/$repository/releases/$(gh release view "$tag" --json databaseId --jq .databaseId)" \
  -F draft=false >/dev/null
[[ "$(gh release view "$tag" --json isDraft --jq .isDraft)" == "false" ]] ||
  die "the release still reads as a draft; publish it by hand at $url"
echo "$url"
