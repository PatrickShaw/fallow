"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { getPlatformPackage, isPlatformPackage } = require("./platform-package");

test("maps every supported target to its platform package", () => {
  assert.equal(
    getPlatformPackage("darwin", "arm64"),
    "@fallow-cli/fallow-similar-code-darwin-arm64",
  );
  assert.equal(
    getPlatformPackage("linux", "x64", "glibc"),
    "@fallow-cli/fallow-similar-code-linux-x64-gnu",
  );
  assert.equal(
    getPlatformPackage("linux", "arm64", "musl"),
    "@fallow-cli/fallow-similar-code-linux-arm64-musl",
  );
  assert.equal(
    getPlatformPackage("win32", "x64"),
    "@fallow-cli/fallow-similar-code-win32-x64-msvc",
  );
  assert.equal(getPlatformPackage("freebsd", "x64"), undefined);
  assert.equal(isPlatformPackage("@fallow-cli/fallow-similar-code-darwin-arm64"), true);
  assert.equal(isPlatformPackage("@fallow-cli/darwin-arm64"), false);
});
