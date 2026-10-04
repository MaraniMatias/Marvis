/**
 * The one place a version is written, and the check that every copy of it agrees.
 *
 * The app version lives in three manifests because three toolchains read it: pnpm from
 * `package.json`, Cargo from `src-tauri/Cargo.toml`, and the bundler from `src-tauri/tauri.conf.json`.
 * Cargo also records the crate version in `src-tauri/Cargo.lock`.
 * A release tag that disagrees with any of them publishes a binary whose name and contents claim
 * different versions, so this checks and writes all four version records as one release.
 * Bumping by hand is what put that risk in place; the tag is still a deliberate act, and this does
 * the bookkeeping around it.
 *
 * Usage:
 *   node scripts/release.mjs --check [<tag>]        all versions agree, and match the tag
 *   node scripts/release.mjs --bump <version>       write them, commit, tag and push
 *   node scripts/release.mjs --bump <version> --write-only   write them without Git operations
 *   node scripts/release.mjs --bump <version> --dry-run   report the writes, change nothing
 *   node scripts/release.mjs --restore <version>         restore all four to a lower version, no Git operations
 *
 * `--check` takes no tag in CI, where the clone is shallow and has no tags to read; the release
 * workflow passes the tag it was triggered by.
 */
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Semver, with the prerelease and build metadata Cargo and the bundler both accept. */
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

/** The first `version = "..."` under `[package]`, which is the crate's own and not a dependency's. */
const CARGO_VERSION = /^version\s*=\s*"([^"]+)"/m;
const CARGO_PACKAGE = /^\[package\][\s\S]*?^version\s*=\s*"[^"]+"/m;
const CARGO_LOCK_VERSION = /^(\[\[package\]\]\nname = "marvis"\nversion = ")([^"]+)(")/m;
const JSON_VERSION = /^(\s*"version":\s*")[^"]+(")/m;

/** Reads and writes one version field, touching nothing else in the file. */
function jsonManifest(path, readVersion) {
  return {
    path,
    read: () => readVersion(readFileSync(path, "utf8")),
    write(version) {
      const content = readFileSync(path, "utf8");
      const matches = content.match(new RegExp(JSON_VERSION.source, "gm"));
      if (matches?.length !== 1) {
        throw new Error(`${path} has ${matches?.length ?? 0} version fields, expected exactly one`);
      }
      writeFileSync(path, content.replace(JSON_VERSION, `$1${version}$2`));
    },
  };
}

function cargoLockManifest(path) {
  function matches(content) {
    return [...content.matchAll(new RegExp(CARGO_LOCK_VERSION.source, "gm"))];
  }
  return {
    path,
    read() {
      const versions = matches(readFileSync(path, "utf8"));
      if (versions.length !== 1) {
        throw new Error(`${path} has ${versions.length} marvis package entries, expected exactly one`);
      }
      return versions[0][2];
    },
    write(version) {
      const content = readFileSync(path, "utf8");
      if (matches(content).length !== 1) {
        throw new Error(`${path} does not contain exactly one marvis package entry`);
      }
      writeFileSync(path, content.replace(CARGO_LOCK_VERSION, `$1${version}$3`));
    },
  };
}

const MANIFESTS = [
  jsonManifest("package.json", (content) => JSON.parse(content).version),
  jsonManifest("src-tauri/tauri.conf.json", (content) => JSON.parse(content).version),
  {
    path: join("src-tauri", "Cargo.toml"),
    read: () => readFileSync(join("src-tauri", "Cargo.toml"), "utf8").match(CARGO_VERSION)[1],
    write(version) {
      const path = join("src-tauri", "Cargo.toml");
      writeFileSync(
        path,
        readFileSync(path, "utf8").replace(CARGO_PACKAGE, (match) =>
          match.replace(CARGO_VERSION, `version = "${version}"`),
        ),
      );
    },
  },
  cargoLockManifest(join("src-tauri", "Cargo.lock")),
];

function git(args) {
  // stderr is captured rather than inherited: the callers here use a non-zero exit to ask a
  // question ("is this tag there?"), and the question's own answer is not a message for the user.
  return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function check(tag) {
  const versions = new Map(MANIFESTS.map((manifest) => [manifest.path, manifest.read()]));
  const [path, version] = versions.entries().next().value;
  for (const [otherPath, otherVersion] of versions) {
    if (otherVersion !== version) {
      fail(`${otherPath} is ${otherVersion}, ${path} is ${version}: the version files disagree`);
    }
  }
  if (tag !== undefined) {
    const tagged = tag.replace(/^v/, "");
    if (tagged !== version) {
      fail(`tag ${tag} does not match the ${version} in ${path}`);
    }
  }
  console.log(`every version file is ${version}${tag ? `, matching ${tag}` : ""}`);
}

function bump(version, { dryRun, push, writeOnly }) {
  // The tag is `v<version>` and the manifests hold the version bare, so both spellings are accepted.
  const bare = version.replace(/^v/, "");
  if (!SEMVER.test(bare)) {
    fail(`"${version}" is not a semantic version`);
  }
  version = bare;
  const versions = MANIFESTS.map((manifest) => [manifest.path, manifest.read()]);
  const [path, current] = versions[0];
  if (writeOnly) {
    for (const [otherPath, otherVersion] of versions) {
      if (otherVersion !== current) {
        fail(`${otherPath} is ${otherVersion}, ${path} is ${current}: the version files disagree`);
      }
    }
  }
  // Compared on the numeric core alone, so `0.2.0` may follow `0.2.0-rc.1` but not `0.3.0`.
  const core = (value) => value.split(/[-+]/)[0].split(".").map(Number);
  const [currentCore, nextCore] = [core(current), core(version)];
  const forward =
    nextCore[0] > currentCore[0] ||
    (nextCore[0] === currentCore[0] &&
      (nextCore[1] > currentCore[1] || (nextCore[1] === currentCore[1] && nextCore[2] >= currentCore[2])));
  if (!forward) {
    fail(`${path} is already ${current}: ${version} does not move the version forward`);
  }
  const tag = `v${version}`;
  if (!writeOnly) {
    try {
      git(["rev-parse", "--verify", "--quiet", `refs/tags/${tag}`]);
      fail(`tag ${tag} already exists`);
    } catch {
      // No such tag, which is the only way to get here.
    }
  }
  if (dryRun) {
    // A dry run changes nothing, so it is worth running against a working tree that has changes.
    for (const manifest of MANIFESTS) {
      console.log(`${manifest.path}: ${manifest.read()} -> ${version}`);
    }
    console.log(
      writeOnly
        ? "dry run: would write only; no commit, tag or push"
        : `dry run: would commit and tag ${tag}, and push nothing`,
    );
    return;
  }
  if (writeOnly) {
    for (const manifest of MANIFESTS) {
      manifest.write(version);
    }
    console.log(`wrote version ${version} to all four version files; no commit, tag or push`);
    return;
  }
  if (git(["status", "--porcelain"])) {
    fail("the working tree has uncommitted changes; commit or stash them first");
  }
  for (const manifest of MANIFESTS) {
    manifest.write(version);
  }
  git(["add", ...MANIFESTS.map((manifest) => manifest.path)]);
  git(["commit", "-m", `chore: release ${tag}`]);
  git(["tag", "-a", tag, "-m", `Marvis ${tag}`]);
  console.log(`tagged ${tag} at ${git(["rev-parse", "HEAD"])}`);
  if (!push) {
    return;
  }
  // One push for the commit and the tag: a tag whose commit is not on the remote never runs CI.
  git(["push", "--follow-tags", "origin", "HEAD"]);
  console.log(`pushed ${git(["rev-parse", "--abbrev-ref", "HEAD"])} and ${tag}`);
}

function restore(version) {
  const target = version.replace(/^v/, "");
  if (!SEMVER.test(target)) {
    fail(`"${version}" is not a semantic version`);
  }
  const versions = MANIFESTS.map((manifest) => [manifest.path, manifest.read()]);
  const [path, current] = versions[0];
  for (const [otherPath, otherVersion] of versions) {
    if (otherVersion !== current) {
      fail(`${otherPath} is ${otherVersion}, ${path} is ${current}: the version files disagree`);
    }
  }
  const [targetCore, currentCore] = [target, current].map((value) => value.split(/[-+]/)[0].split(".").map(Number));
  const lower =
    targetCore[0] < currentCore[0] ||
    (targetCore[0] === currentCore[0] &&
      (targetCore[1] < currentCore[1] || (targetCore[1] === currentCore[1] && targetCore[2] < currentCore[2])));
  if (!lower) {
    fail(`--restore target ${target} must be lower than current version ${current}`);
  }
  for (const manifest of MANIFESTS) manifest.write(target);
  console.log(`restored all four version files to ${target}; no commit, tag or push`);
}

const args = process.argv.slice(2);
if (args.includes("--check")) {
  const tag = args.find((argument) => argument.startsWith("v") && SEMVER.test(argument.slice(1)));
  check(tag);
} else if (args[0] === "--restore") {
  if (args.length !== 2 || args[1].startsWith("--")) {
    fail("--restore requires exactly one lower version, for example: node scripts/release.mjs --restore 0.13.2");
  }
  restore(args[1]);
} else if (args.includes("--bump")) {
  // The first argument that is not a flag, so `pnpm release:preview v0.2.0` and
  // `pnpm release v0.2.0 --dry-run` both read the same.
  const version = args.slice(args.indexOf("--bump") + 1).find((argument) => !argument.startsWith("--"));
  if (!version) {
    fail("--bump needs a version, for example: node scripts/release.mjs --bump 0.2.0");
  }
  bump(version, {
    dryRun: args.includes("--dry-run"),
    push: !args.includes("--no-push"),
    writeOnly: args.includes("--write-only"),
  });
} else {
  fail(
    "usage: node scripts/release.mjs --check [<tag>]\n" +
      "       node scripts/release.mjs --bump <version> [--dry-run] [--no-push]\n" +
      "       node scripts/release.mjs --restore <lower-version>",
  );
}
