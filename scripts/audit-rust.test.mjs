import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { auditPackages, cargoMetadataArgs, isBlocking, loadLockedPackages, parseCargoMetadata } from "./audit-rust.mjs";
import { isAuditedPatchedGlib, PATCHED_GLIB_ADVISORY_IDS, PATCHED_GLIB_DIR } from "./patched-glib.mjs";

function response(results, ok = true, status = 200) {
  return { ok, status, json: async () => ({ results }) };
}

function jsonResponse(value, ok = true, status = 200) {
  return { ok, status, json: async () => value };
}

function vuln(id, aliases, severity = "MODERATE") {
  return { id, aliases, database_specific: { severity } };
}

function queryResult(ids) {
  return response(ids.map((id) => ({ vulns: id ? [{ id }] : [] })));
}

const APP_ID = "path+file:///fixture#app@0.1.0";
const CRATES_IO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index";

function patchedGlibPackage() {
  const manifestPath = join(PATCHED_GLIB_DIR, "Cargo.toml");
  return parseCargoMetadata({
    workspace_members: [APP_ID],
    packages: [
      {
        id: `path+file://${manifestPath}#glib@0.18.5`,
        name: "glib",
        version: "0.18.5",
        source: null,
        manifest_path: manifestPath,
      },
    ],
  })[0];
}

test("malformed Cargo.lock fails before any advisory lookup", (t) => {
  const directory = mkdtempSync(join(tmpdir(), "marvis-audit-lock-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const manifestPath = join(directory, "Cargo.toml");
  writeFileSync(manifestPath, '[package]\nname = "audit-fixture"\nversion = "0.1.0"\nedition = "2021"\n');
  writeFileSync(join(directory, "Cargo.lock"), "this is not valid TOML = [\n");
  assert.throws(() => loadLockedPackages({ manifestPath }), /could not validate Cargo\.lock/);
});

test("metadata selects locked all-feature dependencies without host-platform filtering", () => {
  const args = cargoMetadataArgs("fixture/Cargo.toml");
  assert.ok(args.includes("--locked"));
  assert.ok(args.includes("--all-features"));
  assert.ok(!args.includes("--filter-platform"));

  const packages = parseCargoMetadata({
    workspace_members: [APP_ID],
    packages: [
      { id: "registry+crates.io#windows-only@1.0.0", name: "windows-only", version: "1.0.0", source: CRATES_IO_SOURCE },
      {
        id: "registry+crates.io#optional-feature@2.0.0",
        name: "optional-feature",
        version: "2.0.0",
        source: CRATES_IO_SOURCE,
      },
      { id: APP_ID, name: "app", version: "0.1.0", source: null },
    ],
  });
  assert.deepEqual(
    packages.map(({ name }) => name),
    ["optional-feature", "windows-only"],
  );
  assert.ok(packages.every(({ source }) => source === CRATES_IO_SOURCE));
});

test("foreign registry alone fails closed instead of returning an empty audit set", () => {
  assert.throws(
    () =>
      parseCargoMetadata({
        workspace_members: [APP_ID],
        packages: [
          { id: "registry+private#dep@1.0.0", name: "dep", version: "1.0.0", source: "registry+https://private" },
        ],
      }),
    /unsupported registry source for dep@1\.0\.0/,
  );
});

test("same name and version from crates.io and a foreign registry fail before deduplication", () => {
  assert.throws(
    () =>
      parseCargoMetadata({
        workspace_members: [APP_ID],
        packages: [
          { id: "registry+crates.io#dep@1.0.0", name: "dep", version: "1.0.0", source: CRATES_IO_SOURCE },
          { id: "registry+private#dep@1.0.0", name: "dep", version: "1.0.0", source: "registry+https://private" },
        ],
      }),
    /unsupported registry source for dep@1\.0\.0/,
  );
});

test("external git and path packages fail closed", () => {
  const packages = [
    { id: "git+https://example/repo#dep@1.0.0", name: "dep", version: "1.0.0", source: "git+https://example/repo" },
    { id: "path+file:///external#dep@1.0.0", name: "dep", version: "1.0.0", source: null },
  ];
  assert.throws(
    () => parseCargoMetadata({ workspace_members: [APP_ID], packages: [packages[0]] }),
    /unsupported external dependency source for dep@1\.0\.0/,
  );
  assert.throws(
    () => parseCargoMetadata({ workspace_members: [APP_ID], packages: [packages[1]] }),
    /unsupported external path dependency for dep@1\.0\.0/,
  );
});

test("only the hash-pinned local glib backport is accepted as an external path package", () => {
  const glib = patchedGlibPackage();
  assert.equal(glib.name, "glib");
  assert.equal(glib.version, "0.18.5");
  assert.ok(isAuditedPatchedGlib(glib));

  assert.throws(
    () =>
      parseCargoMetadata({
        workspace_members: [APP_ID],
        packages: [
          {
            id: "path+file:///external/glib#glib@0.18.5",
            name: "glib",
            version: "0.18.5",
            source: null,
            manifest_path: "/external/glib/Cargo.toml",
          },
        ],
      }),
    /unsupported external path dependency for glib@0\.18\.5/,
  );
});

test("network timeout fails closed", async () => {
  const fetchImpl = (_url, { signal }) =>
    new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => reject(signal.reason), { once: true });
    });
  await assert.rejects(
    auditPackages([{ name: "fixture", version: "1.0.0" }], { fetchImpl, timeoutMs: 5 }),
    /timed out/,
  );
});

test("network errors fail closed without echoing underlying error details", async () => {
  await assert.rejects(
    auditPackages([{ name: "fixture", version: "1.0.0" }], {
      fetchImpl: async () => {
        throw new Error("secret transport detail");
      },
    }),
    (error) => error.message === "OSV request failed (network or TLS error)" && !error.message.includes("secret"),
  );
});

test("HTTP failure fails closed without exposing response contents", async () => {
  await assert.rejects(
    auditPackages([{ name: "fixture", version: "1.0.0" }], {
      fetchImpl: async () => response([], false, 503),
    }),
    /HTTP 503/,
  );
});

test("informational unmaintained advisories warn without becoming a false clean result", async () => {
  const advisories = await auditPackages([{ name: "fixture", version: "1.0.0" }], {
    fetchImpl: async (url, { method }) =>
      method === "POST"
        ? queryResult(["RUSTSEC-2024-0001"])
        : jsonResponse({ id: "RUSTSEC-2024-0001", summary: "fixture is unmaintained" }),
  });
  assert.equal(advisories.length, 1);
  assert.equal(advisories[0].severity, "INFO");
  assert.equal(isBlocking(advisories[0]), false);
});

test("moderate advisories block the audit", async () => {
  const advisories = await auditPackages([{ name: "glib", version: "0.18.5" }], {
    fetchImpl: async (url, { method }) =>
      method === "POST"
        ? queryResult(["RUSTSEC-2024-0429"])
        : jsonResponse(vuln("RUSTSEC-2024-0429", ["GHSA-aaaa-bbbb-cccc"])),
  });
  assert.equal(advisories[0].severity, "MODERATE");
  assert.equal(isBlocking(advisories[0]), true);
});

test("the exact patched glib advisory is reported fixed only for the pinned backport", async () => {
  const glib = patchedGlibPackage();
  const [id, alias] = PATCHED_GLIB_ADVISORY_IDS;
  const advisories = await auditPackages([glib], {
    fetchImpl: async (_url, { method }) => (method === "POST" ? queryResult([id]) : jsonResponse(vuln(id, [alias]))),
  });

  assert.equal(advisories.length, 1);
  assert.equal(advisories[0].severity, "MODERATE");
  assert.equal(advisories[0].status, "FIXED");
  assert.equal(isBlocking(advisories[0]), false);
});

test("other advisories in the patched glib version remain blocking", async () => {
  const glib = patchedGlibPackage();
  const advisories = await auditPackages([glib], {
    fetchImpl: async (_url, { method }) =>
      method === "POST" ? queryResult(["RUSTSEC-2025-9999"]) : jsonResponse(vuln("RUSTSEC-2025-9999", [])),
  });

  assert.equal(advisories.length, 1);
  assert.equal(advisories[0].status, "ACTIVE");
  assert.equal(isBlocking(advisories[0]), true);
});

test("OSV identifiers and aliases deduplicate across affected packages", async () => {
  const packages = [
    { name: "crate-a", version: "1.0.0" },
    { name: "crate-b", version: "2.0.0" },
  ];
  const advisories = await auditPackages(packages, {
    fetchImpl: async (url, { method }) => {
      if (method === "POST") return queryResult(["RUSTSEC-2024-0429", "GHSA-aaaa-bbbb-cccc"]);
      const id = decodeURIComponent(new URL(url).pathname.split("/").at(-1));
      return jsonResponse(
        id === "RUSTSEC-2024-0429" ? vuln(id, ["GHSA-aaaa-bbbb-cccc"]) : { id, aliases: ["RUSTSEC-2024-0429"] },
      );
    },
  });
  assert.equal(advisories.length, 1);
  assert.equal(advisories[0].severity, "MODERATE");
  assert.deepEqual(advisories[0].packages, ["crate-a@1.0.0", "crate-b@2.0.0"]);
});
