"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const path = require("node:path");
const ownManifest = require("../package.json");
const { resolveBinary, resolveBinaryArtifact } = require("./run-binary");

const PACKAGE_NAME = "@fallow-cli/fallow-similar-code-darwin-arm64";
const manifest = (overrides = {}) =>
  JSON.stringify({ name: PACKAGE_NAME, version: ownManifest.version, ...overrides });

test("resolves an exact-version regular native binary", () => {
  const manifestPath = path.join("/packages", "native", "package.json");
  const resolved = resolveBinary(
    PACKAGE_NAME,
    () => manifestPath,
    () => manifest(),
    () => ({ isFile: () => true, isSymbolicLink: () => false }),
  );
  assert.equal(resolved, path.join("/packages", "native", "fallow-similar-code"));
});

test("rejects a platform package from another release", () => {
  assert.throws(
    () =>
      resolveBinary(
        PACKAGE_NAME,
        () => "/native/package.json",
        () => manifest({ version: "0.0.0" }),
        () => ({ isFile: () => true, isSymbolicLink: () => false }),
      ),
    /version mismatch/,
  );
});

test("rejects symlinked binaries", () => {
  assert.throws(
    () =>
      resolveBinary(
        PACKAGE_NAME,
        () => "/native/package.json",
        () => manifest(),
        (filePath) => ({
          isFile: () => true,
          isSymbolicLink: () => !filePath.endsWith("package.json"),
        }),
      ),
    /not a regular file/,
  );
});

test("rejects a symlinked platform manifest", () => {
  assert.throws(
    () =>
      resolveBinaryArtifact(
        PACKAGE_NAME,
        () => "/native/package.json",
        () => manifest(),
        () => ({ isFile: () => true, isSymbolicLink: () => true }),
      ),
    /platform manifest is not a regular file/,
  );
});

test("returns the exact owning manifest in the verification artifact", () => {
  const manifestPath = path.join("/packages", "native", "package.json");
  const artifact = resolveBinaryArtifact(
    PACKAGE_NAME,
    () => manifestPath,
    () => manifest(),
    () => ({ isFile: () => true, isSymbolicLink: () => false }),
  );
  assert.deepEqual(artifact, {
    packageName: PACKAGE_NAME,
    packageVersion: ownManifest.version,
    manifestPath,
    binaryName: "fallow-similar-code",
    binaryPath: path.join("/packages", "native", "fallow-similar-code"),
  });
});

test("rejects a manifest that does not belong to the resolved package", () => {
  assert.throws(
    () =>
      resolveBinaryArtifact(
        PACKAGE_NAME,
        () => "/native/package.json",
        () => manifest({ name: "@fallow-cli/other" }),
        () => ({ isFile: () => true, isSymbolicLink: () => false }),
      ),
    /ownership mismatch/,
  );
});

test("uses the Windows executable basename in the artifact", () => {
  const artifact = resolveBinaryArtifact(
    PACKAGE_NAME,
    () => path.join("C:\\packages", "native", "package.json"),
    () => manifest(),
    () => ({ isFile: () => true, isSymbolicLink: () => false }),
    "win32",
  );
  assert.equal(artifact.binaryName, "fallow-similar-code.exe");
  assert.match(artifact.binaryPath, /fallow-similar-code\.exe$/);
});
