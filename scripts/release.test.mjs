import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const RELEASE_SCRIPT = fileURLToPath(new URL("./release.mjs", import.meta.url));

function fixture(t, lockVersion = "0.9.0") {
  const cwd = mkdtempSync(join(tmpdir(), "marvis-release-"));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  mkdirSync(join(cwd, "src-tauri"));
  writeFileSync(join(cwd, "package.json"), '{\n  "version": "0.9.0"\n}\n');
  writeFileSync(join(cwd, "src-tauri", "tauri.conf.json"), '{\n  "version": "0.9.0"\n}\n');
  writeFileSync(join(cwd, "src-tauri", "Cargo.toml"), '[package]\nname = "marvis"\nversion = "0.9.0"\n');
  writeFileSync(
    join(cwd, "src-tauri", "Cargo.lock"),
    `version = 4\n\n[[package]]\nname = "marvis"\nversion = "${lockVersion}"\n`,
  );
  execFileSync("git", ["init", "-q"], { cwd });
  execFileSync("git", ["config", "user.name", "Release Test"], { cwd });
  execFileSync("git", ["config", "user.email", "release-test@example.com"], { cwd });
  execFileSync("git", ["add", "."], { cwd });
  execFileSync("git", ["commit", "-qm", "fixture"], { cwd });
  return cwd;
}

function run(cwd, ...args) {
  return spawnSync(process.execPath, [RELEASE_SCRIPT, ...args], { cwd, encoding: "utf8" });
}

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
}

function status(cwd) {
  return execFileSync("git", ["status", "--porcelain"], { cwd, encoding: "utf8" }).trimEnd();
}

test("--check rejects a Cargo.lock version mismatch", (t) => {
  const cwd = fixture(t, "0.8.0");
  const result = run(cwd, "--check");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Cargo\.lock is 0\.8\.0, package\.json is 0\.9\.0/);
});

test("--bump synchronizes Cargo.lock in a temporary fixture", (t) => {
  const cwd = fixture(t);
  const head = git(cwd, "rev-parse", "HEAD");
  const result = run(cwd, "--bump", "0.10.0", "--no-push");
  assert.equal(result.status, 0, result.stderr);
  assert.notEqual(git(cwd, "rev-parse", "HEAD"), head);
  assert.equal(git(cwd, "tag", "--list"), "v0.10.0");
  assert.equal(status(cwd), "");
  assert.match(readFileSync(join(cwd, "src-tauri", "Cargo.lock"), "utf8"), /name = "marvis"\nversion = "0\.10\.0"/);
  const check = run(cwd, "--check");
  assert.equal(check.status, 0);
  assert.match(check.stdout, /every version file is 0\.10\.0/);
});

test("--write-only updates versions without Git side effects", (t) => {
  const cwd = fixture(t);
  const head = git(cwd, "rev-parse", "HEAD");
  const tags = git(cwd, "tag", "--list");
  const result = run(cwd, "--bump", "0.10.0", "--write-only");
  assert.equal(result.status, 0, result.stderr);
  assert.equal(git(cwd, "rev-parse", "HEAD"), head);
  assert.equal(git(cwd, "tag", "--list"), tags);
  assert.equal(git(cwd, "diff", "--cached", "--name-only"), "");
  assert.deepEqual(status(cwd).split("\n").sort(), [
    " M package.json",
    " M src-tauri/Cargo.lock",
    " M src-tauri/Cargo.toml",
    " M src-tauri/tauri.conf.json",
  ]);
  const check = run(cwd, "--check");
  assert.equal(check.status, 0);
  assert.match(check.stdout, /every version file is 0\.10\.0/);
});

test("--write-only --dry-run leaves files and Git untouched", (t) => {
  const cwd = fixture(t);
  const head = git(cwd, "rev-parse", "HEAD");
  const result = run(cwd, "--bump", "0.10.0", "--write-only", "--dry-run");
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /dry run: would write only/);
  assert.equal(git(cwd, "rev-parse", "HEAD"), head);
  assert.equal(git(cwd, "tag", "--list"), "");
  assert.equal(status(cwd), "");
});

test("--write-only rejects mismatches with and without --dry-run", (t) => {
  for (const args of [["--write-only"], ["--write-only", "--dry-run"]]) {
    const cwd = fixture(t, "0.8.0");
    const head = git(cwd, "rev-parse", "HEAD");
    const result = run(cwd, "--bump", "0.10.0", ...args);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Cargo\.lock is 0\.8\.0, package\.json is 0\.9\.0/);
    assert.equal(git(cwd, "rev-parse", "HEAD"), head);
    assert.equal(git(cwd, "tag", "--list"), "");
    assert.equal(status(cwd), "");
    assert.match(readFileSync(join(cwd, "src-tauri", "Cargo.lock"), "utf8"), /version = "0\.8\.0"/);
  }
});
