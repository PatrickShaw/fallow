"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const ownManifest = require("../package.json");
const { verifyBinary } = require("./verify-binary");

const PACKAGE_NAME = "@fallow-cli/fallow-similar-code-darwin-arm64";
const BINARY_NAME = "fallow-similar-code";

const digest = (bytes) => `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;

const makeFixture = (t, options = {}) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "fallow-similar-code-verify-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const binaryPath = path.join(directory, BINARY_NAME);
  const manifestPath = path.join(directory, "package.json");
  const binary = options.binary ?? Buffer.from("trusted binary");
  fs.writeFileSync(binaryPath, binary);
  fs.writeFileSync(`${binaryPath}.sig`, options.signature ?? Buffer.alloc(64));
  const manifest = {
    name: PACKAGE_NAME,
    version: ownManifest.version,
    fallowDigests: { [BINARY_NAME]: options.digest ?? digest(binary) },
    ...options.manifest,
  };
  fs.writeFileSync(manifestPath, JSON.stringify(manifest));
  return {
    artifact: {
      packageName: PACKAGE_NAME,
      packageVersion: ownManifest.version,
      manifestPath,
      binaryName: BINARY_NAME,
      binaryPath,
    },
    manifestPath,
  };
};

const acceptSignature = {
  verify: () => true,
  createPublicKey: () => ({}),
};

test("verifies Ed25519 and the exact platform manifest digest before launch", (t) => {
  const binary = Buffer.from("signed trusted binary");
  const { privateKey, publicKey } = crypto.generateKeyPairSync("ed25519");
  const { artifact } = makeFixture(t, {
    binary,
    signature: crypto.sign(null, binary, privateKey),
  });
  assert.deepEqual(verifyBinary(artifact, { createPublicKey: () => publicKey }), { ok: true });
});

test("rejects the legacy path-only call instead of falling back to signature-only", () => {
  const result = verifyBinary("/native/fallow-similar-code", acceptSignature);
  assert.equal(result.ok, false);
  assert.equal(result.code, "artifact-invalid");
});

test("rejects malformed and invalid detached signatures", (t) => {
  const malformed = makeFixture(t, { signature: Buffer.alloc(63) });
  assert.equal(verifyBinary(malformed.artifact).code, "sig-invalid");

  const invalid = makeFixture(t);
  assert.equal(
    verifyBinary(invalid.artifact, {
      verify: () => false,
      createPublicKey: () => ({}),
    }).code,
    "sig-invalid",
  );
});

test("fails closed when the embedded digest is missing or malformed", (t) => {
  const absent = makeFixture(t, { manifest: { fallowDigests: undefined } });
  assert.equal(verifyBinary(absent.artifact, acceptSignature).code, "digest-missing");

  const missingEntry = makeFixture(t, { manifest: { fallowDigests: {} } });
  assert.equal(verifyBinary(missingEntry.artifact, acceptSignature).code, "digest-missing");

  const malformed = makeFixture(t, {
    manifest: { fallowDigests: { [BINARY_NAME]: "not-a-digest" } },
  });
  assert.equal(verifyBinary(malformed.artifact, acceptSignature).code, "digest-invalid");
});

test("rejects digest tampering even when signature verification succeeds", (t) => {
  const binary = Buffer.from("authentically signed but stale digest");
  const { privateKey, publicKey } = crypto.generateKeyPairSync("ed25519");
  const fixture = makeFixture(t, {
    binary,
    signature: crypto.sign(null, binary, privateKey),
    digest: `sha256:${"a".repeat(64)}`,
  });
  const result = verifyBinary(fixture.artifact, { createPublicKey: () => publicKey });
  assert.equal(result.ok, false);
  assert.equal(result.code, "digest-mismatch");
});

test("rejects a manifest owned by another package or version", (t) => {
  const wrongOwner = makeFixture(t, { manifest: { name: "@fallow-cli/other" } });
  assert.equal(verifyBinary(wrongOwner.artifact, acceptSignature).code, "package-mismatch");

  const wrongVersion = makeFixture(t, { manifest: { version: "0.0.0" } });
  assert.equal(verifyBinary(wrongVersion.artifact, acceptSignature).code, "version-mismatch");
});

test("rejects a digest from an adjacent manifest", (t) => {
  const fixture = makeFixture(t);
  const adjacent = fs.mkdtempSync(path.join(os.tmpdir(), "fallow-similar-code-adjacent-"));
  t.after(() => fs.rmSync(adjacent, { recursive: true, force: true }));
  const result = verifyBinary(
    { ...fixture.artifact, manifestPath: path.join(adjacent, "package.json") },
    acceptSignature,
  );
  assert.equal(result.ok, false);
  assert.equal(result.code, "package-mismatch");
});

test("rejects a missing platform manifest", (t) => {
  const fixture = makeFixture(t);
  fs.rmSync(fixture.manifestPath);
  assert.equal(verifyBinary(fixture.artifact, acceptSignature).code, "manifest-invalid");
});
