"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { verifyBinary } = require("./verify-binary");

test("verifies the detached signature before launch", () => {
  const reads = [Buffer.from("binary"), Buffer.alloc(64)];
  const result = verifyBinary(
    "/native/fallow-similar-code",
    () => reads.shift(),
    () => true,
    () => ({}),
  );
  assert.deepEqual(result, { ok: true });
});

test("rejects malformed detached signatures", () => {
  const reads = [Buffer.from("binary"), Buffer.alloc(63)];
  const result = verifyBinary("/native/fallow-similar-code", () => reads.shift());
  assert.equal(result.ok, false);
  assert.match(result.message, /unexpected length/);
});
