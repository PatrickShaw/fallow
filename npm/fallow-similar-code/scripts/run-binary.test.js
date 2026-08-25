"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const path = require("node:path");
const { resolveBinary } = require("./run-binary");

test("resolves an exact-version regular native binary", () => {
  const manifestPath = path.join("/packages", "native", "package.json");
  const resolved = resolveBinary(
    "@fallow-cli/fallow-similar-code-darwin-arm64",
    () => manifestPath,
    () => JSON.stringify({ version: "3.18.0" }),
    () => ({ isFile: () => true, isSymbolicLink: () => false }),
  );
  assert.equal(resolved, path.join("/packages", "native", "fallow-similar-code"));
});

test("rejects a platform package from another release", () => {
  assert.throws(
    () =>
      resolveBinary(
        "native",
        () => "/native/package.json",
        () => JSON.stringify({ version: "3.17.0" }),
        () => ({ isFile: () => true, isSymbolicLink: () => false }),
      ),
    /version mismatch/,
  );
});

test("rejects symlinked binaries", () => {
  assert.throws(
    () =>
      resolveBinary(
        "native",
        () => "/native/package.json",
        () => JSON.stringify({ version: "3.18.0" }),
        () => ({ isFile: () => true, isSymbolicLink: () => true }),
      ),
    /not a regular file/,
  );
});
