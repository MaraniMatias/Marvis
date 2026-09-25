import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { exactPayloadCommand } from "../src/payload-command.js";

const { Terminal } = createRequire(import.meta.url)("@xterm/headless");

function write(terminal, bytes) {
  return new Promise((resolve) => terminal.write(bytes, resolve));
}

test("xterm parses split UTF-8 and ANSI output chunks", async () => {
  const terminal = new Terminal({ allowProposedApi: true, cols: 40, rows: 5 });
  const stream = new TextEncoder().encode("\x1b[31mred λ\x1b[0m\r\nsecond line\r\n");

  await write(terminal, stream.subarray(0, 5));
  await write(terminal, stream.subarray(5, 8));
  await write(terminal, stream.subarray(8));

  assert.equal(terminal.buffer.active.getLine(0).translateToString(true), "red λ");
  assert.equal(terminal.buffer.active.getLine(1).translateToString(true), "second line");
  terminal.dispose();
});

test("Node payload probe writes exact marker-delimited bytes", () => {
  const begin = Uint8Array.of(0x1e, 0x42, 0x45, 0x47, 0x49, 0x4e, 0x1f);
  const end = Uint8Array.of(0x1e, 0x45, 0x4e, 0x44, 0x1f);
  const byteCount = 128 * 1024 + 19;
  const result = spawnSync("/bin/sh", ["-c", exactPayloadCommand(begin, end, byteCount)], {
    encoding: null,
    maxBuffer: byteCount + begin.length + end.length,
  });

  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr?.toString());
  assert.deepEqual(
    result.stdout,
    Buffer.concat([Buffer.from(begin), Buffer.alloc(byteCount, 0x78), Buffer.from(end)]),
  );
});
