#!/usr/bin/env bash
# Ships one release: bumps the version, pushes the commit and the tag, waits for the build and reads
# back the release the run published.
#
# Usage: scripts/release.sh <version> [--dry-run]
#
# The version is bumped with scripts/release.mjs, which owns the manifests; this owns git and the
# GitHub release, which are the two things that cannot be undone from here. That split is why the
# tag is pushed before the build is known to pass: the alternative is a dance between bumping and
# tagging, and a dry run cannot even start until the bumped manifests are on the default branch.
# What this gives up instead is a human pausing between the artifacts being uploaded and the
# release going live, so the pause is conditional on the run instead: the run publishes only once
# every matrix job succeeded, which is also the only way a release exists at all. Publishing it there
# rather than here is what keeps a draft from sitting there holding the previous release as the
# latest one, and it leaves this script with a read-back where its publish used to be.
#
# Every step is skipped when the state already satisfies it, so the same command resumes a release
# instead of refusing it. That is what makes the two-command release work:
#
#   scripts/release.sh 0.2.1 --dry-run   # bump, push main, run the build, no tag
#   scripts/release.sh 0.2.1             # pushes that tag, waits, publishes
#
# The second command finds the manifests already at 0.2.1 and the tag already made, and only has the
# tag to push. It also resumes a release whose build failed, which `gh run rerun <id>` would also do.
#
# Two gates stand between this and a tag, and both are about the tree rather than about the version.
# The first is local and runs before anything is written: version consistency, formatting, both
# linters, the type checker and the tests, on this machine. The second waits for the checks workflow
# on the commit itself, which is the only answer that covers every platform a release is built for.
# Neither is skippable, because the tag is the one step here that cannot be undone: a version whose
# checks were never green stays on the remote forever with no release behind it, and every later
# attempt has to be a new number for a problem that was answerable before the first one.
#
# If the build fails after the tag, nothing was published and the run can be retried with
# `gh run rerun <id>`: the same tag, the same commit, no new version. If the checks fail before it,
# there is no tag to retry and nothing to clean up.
#
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
die() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }

# Waits for the checks run of one commit and says whether it passed.
#
# This is the gate that has to exist here. The local gate below answers for this machine, and the
# machine a release is built for is not it: a commit can be green on a Mac and red on Linux, which
# is where every release that failed so far failed. Nothing about that is visible before the tag
# goes out, because nothing else runs the suite on Linux, so this is where that answer is asked for.
#
# `gh run watch` is deliberately not used: it ends when the connection to the runner does, and a
# run that is still going reads as a release that failed. Polling cannot be confused with either --
# the run's own state is asked for, and a request that fails answers nothing rather than "failed" --
# so a flaky network costs a few seconds here instead of a version.
wait_for_checks() {
  local commit="$1" run="" short jobs reported="" status conclusion deadline
  short="$(git rev-parse --short "$commit")"
  for _ in $(seq 1 18); do
    run="$(gh run list --workflow ci.yml --commit "$commit" --event push --limit 1 \
      --json databaseId --jq '.[0].databaseId // empty')"
    [[ -n "$run" ]] && break
    sleep 5
  done
  [[ -n "$run" ]] || return 2
  echo "run $run: https://github.com/$repository/actions/runs/$run"
  # Sixty minutes is far longer than these checks take and is here so a runner that wedges cannot
  # leave this waiting for the rest of the day: the run is still there to be looked at.
  deadline=$((SECONDS + 3600))
  while ((SECONDS < deadline)); do
    status="$(gh run view "$run" --json status --jq .status 2>/dev/null)"
    if [[ "$status" == "completed" ]]; then
      break
    fi
    jobs="$(gh run view "$run" --json jobs --jq '[.jobs[] | "\(.name): \(.status)"] | join("  ")' 2>/dev/null)"
    if [[ -n "$jobs" && "$jobs" != "$reported" ]]; then
      printf '  %s\n' "$jobs"
      reported="$jobs"
    fi
    sleep 20
  done
  conclusion="$(gh run view "$run" --json conclusion --jq .conclusion 2>/dev/null)"
  if [[ "$conclusion" != "success" ]]; then
    gh run view "$run" --json jobs --jq '.jobs[] | select(.conclusion == "failure") | "  \(.name)"' >&2
    return 1
  fi
  echo "$short passed the checks on every platform"
}

usage() {
  cat <<'USAGE'
usage: scripts/release.sh <version> [--dry-run]
       pnpm release v0.2.0                     # bump, tag, build, publish
       pnpm release v0.2.0 --dry-run           # bump and build, no tag
       pnpm release:preview v0.2.0             # report the writes, change nothing
USAGE
}

dry_run=false
args=()
for argument in "$@"; do
  case "$argument" in
    --dry-run) dry_run=true ;;
    -h | --help) usage; exit 0 ;;
    *) args+=("$argument") ;;
  esac
done
[[ ${#args[@]} -eq 1 ]] || { usage >&2; exit 1; }

# The tag is `v<version>` and the manifests hold the version bare, so both spellings are accepted
# here and normalized once.
requested="${args[0]#v}"
[[ "$requested" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)*$ ]] ||
  die "\"${args[0]}\" is not a version like 0.2.0 or v0.2.0"
tag="v$requested"

step "Checking this repository can take a release"
for tool in git gh node pnpm; do
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

# What the release is resuming from. The manifests are read with node because they are JSON, and
# reading them any other way is the kind of parsing that breaks on the next format change.
current="$(node -p "require('./package.json').version")"
head="$(git rev-parse HEAD)"
# `set -e` does not fire on a failing left side of `&&`, so these read as questions, not commands.
tag_local=0
git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null && tag_local=1
tag_remote=0
git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1 && tag_remote=1

if ((tag_remote)) && ((!tag_local)); then
  # A resume from a fresh clone needs the tag before its commit can be read.
  git fetch -q origin "refs/tags/$tag:refs/tags/$tag"
  tag_local=1
fi
if ((tag_local)) && [[ "$(git rev-parse "$tag^{commit}")" != "$head" ]]; then
  die "$tag points at $(git rev-parse --short "$tag^{commit}"), not at HEAD ($(git rev-parse --short "$head"))"
fi
if [[ "$current" != "$requested" ]] && ((tag_local || tag_remote)); then
  die "$tag exists and the manifests are at $current, so $requested cannot be a new release of it"
fi
printf 'repository %s, branch main, manifests at %s\n' "$repository" "$current"

if ((tag_remote)); then
  step "Resuming $tag, which is already on the remote"
  release_commit="$(git rev-parse "$tag^{commit}")"
else
  # The gate runs on the tree that is about to be tagged, and only when something is about to be
  # written: a resume waits for a run and publishes nothing, so it has nothing of its own to prove.
  # A tag on a tree that fails cannot be taken back, and this is where that is caught rather than
  # minutes later on a machine that is not this one.
  #
  # These are the steps the `checks` workflow runs that answer the same thing on every machine:
  # version consistency, formatting, both linters, the type checker and the tests. What is left out
  # is everything whose answer is the machine rather than the tree -- the Linux build, the audits
  # and the coverage report -- because a network hiccup during an audit should not hold a release,
  # and none of them can be proved here anyway.
  for gate in release:check fmt:check lint typecheck lint:rust test; do
    step "pnpm $gate"
    pnpm "$gate" || die "pnpm $gate failed here, so nothing was written and no tag was made.
Fix it, commit, and run this again."
  done

  if [[ "$current" == "$requested" ]]; then
    if ((tag_local)); then
      # What --dry-run leaves behind: the version is committed and the tag was made with it.
      step "The manifests and the tag are already at $requested"
    else
      step "The manifests are already at $requested; tagging this commit"
      # Someone bumped the version and committed it, without a tag.
      git tag -a "$tag" -m "Marvis $tag"
    fi
  else
    step "Writing $tag into the manifests, and committing it"
    # --no-push: the two pushes below are separate so a failure leaves a commit with no tag rather
    # than a tag with no commit on the branch that was checked.
    node scripts/release.mjs --bump "$requested" --no-push
  fi

  step "Pushing the commit"
  git push origin HEAD:main
  # HEAD now, and not the one read before the bump: the manifests commit is what the workflow runs
  # against, and the earlier HEAD is a commit no run is ever made for, so polling for it waits out
  # the whole timeout and reports a release that is already building.
  release_commit="$(git rev-parse HEAD)"

  step "Waiting for the checks on $(git rev-parse --short "$release_commit")"
  # On the right of || rather than after a semicolon: `set -e` ends the script on a command that
  # fails on its own, and this one's answer is what decides which of the two failures below it is.
  checks=0
  wait_for_checks "$release_commit" || checks=$?
  case $checks in
    0) ;;
    1)
      die "the checks failed for $(git rev-parse --short "$release_commit") on the jobs named above.
No tag was pushed, so there is no version to redo: fix it on main and run this again."
      ;;
    *)
      die "no checks run appeared for $(git rev-parse --short "$release_commit").
Check https://github.com/$repository/actions and run this again."
      ;;
  esac

  if $dry_run; then
    step "Building $tag without tagging it"
    # The workflow refuses to start on a tag that does not exist, so the dry run is the one that
    # checks main against the version it claims to be releasing.
    gh workflow run release.yml -f tag="$tag"
    echo "When it is green, finish with: pnpm release ${args[0]}"
    exit 0
  fi

  step "Pushing the tag"
  git push origin "refs/tags/$tag"
fi

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

step "Reading the release the run published"
# The run publishes it, so this is a read-back and not a step. It is still where a tag whose run
# succeeded ends with no release to read, and the two ways that happens are read apart: a release
# that is not there at all, and one left as a draft.
gh release view "$tag" >/dev/null ||
  die "there is no release for $tag; check https://github.com/$repository/actions/runs/$run"
[[ "$(gh release view "$tag" --json isDraft --jq .isDraft)" == "false" ]] ||
  die "$tag is still a draft, so the run did not publish it.
Check https://github.com/$repository/actions/runs/$run"
assets="$(gh release view "$tag" --json assets --jq '.assets | length')"
[[ "$assets" -gt 0 ]] || die "$tag has no assets, so there is nothing to install"
gh release view "$tag" --json assets --jq '.assets[].name' | sed 's/^/  /'
# The last line of this script is the link somebody copies.
gh release view "$tag" --json url --jq .url
