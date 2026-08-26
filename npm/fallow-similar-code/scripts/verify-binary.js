"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { isPlatformPackage } = require("./platform-package");
const ownManifest = require("../package.json");

const PUBLIC_KEY = Buffer.from([
  131, 78, 111, 215, 115, 51, 230, 238, 223, 119, 147, 71, 199, 16, 172, 180, 3, 210, 216, 35, 77,
  85, 159, 94, 215, 200, 126, 85, 42, 222, 11, 209,
]);
const SPKI_HEADER = Buffer.from([
  0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
]);

const failure = (code, message) => ({ ok: false, code, message });

const validateArtifact = (artifact) => {
  if (
    !artifact ||
    typeof artifact !== "object" ||
    typeof artifact.packageName !== "string" ||
    typeof artifact.packageVersion !== "string" ||
    typeof artifact.manifestPath !== "string" ||
    typeof artifact.binaryName !== "string" ||
    typeof artifact.binaryPath !== "string"
  ) {
    return failure("artifact-invalid", "similar-code binary artifact is incomplete");
  }
  const expectedBinaryName = artifact.packageName.includes("-win32-")
    ? "fallow-similar-code.exe"
    : "fallow-similar-code";
  if (!isPlatformPackage(artifact.packageName) || artifact.binaryName !== expectedBinaryName) {
    return failure(
      "package-mismatch",
      "similar-code artifact does not identify a supported platform package and binary",
    );
  }
  const manifestDirectory = path.resolve(path.dirname(artifact.manifestPath));
  const binaryDirectory = path.resolve(path.dirname(artifact.binaryPath));
  if (
    manifestDirectory !== binaryDirectory ||
    path.basename(artifact.manifestPath) !== "package.json" ||
    path.basename(artifact.binaryPath) !== artifact.binaryName
  ) {
    return failure(
      "package-mismatch",
      "similar-code binary and package manifest do not have the same package owner",
    );
  }
  return { ok: true };
};

const readManifestDigest = (artifact, readFile) => {
  let manifest;
  try {
    manifest = JSON.parse(readFile(artifact.manifestPath, "utf8"));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return failure("manifest-invalid", `cannot read platform package manifest: ${message}`);
  }
  if (!manifest || typeof manifest !== "object") {
    return failure("manifest-invalid", "platform package manifest must be a JSON object");
  }
  if (manifest.name !== artifact.packageName) {
    return failure(
      "package-mismatch",
      `platform package ownership mismatch, expected ${artifact.packageName} but got ${String(manifest.name)}`,
    );
  }
  if (
    artifact.packageVersion !== ownManifest.version ||
    manifest.version !== artifact.packageVersion
  ) {
    return failure(
      "version-mismatch",
      `similar-code platform package must match companion version ${ownManifest.version}`,
    );
  }
  if (!manifest.fallowDigests || typeof manifest.fallowDigests !== "object") {
    return failure("digest-missing", "platform package manifest has no fallowDigests object");
  }
  const digest = manifest.fallowDigests[artifact.binaryName];
  if (digest === undefined) {
    return failure(
      "digest-missing",
      `platform package manifest has no SHA-256 digest for ${artifact.binaryName}`,
    );
  }
  if (typeof digest !== "string" || !/^sha256:[0-9a-f]{64}$/.test(digest)) {
    return failure(
      "digest-invalid",
      `platform package manifest has a malformed SHA-256 digest for ${artifact.binaryName}`,
    );
  }
  return { ok: true, digest: digest.slice("sha256:".length) };
};

const readBinary = (binaryPath, readFile) => {
  try {
    return { ok: true, bytes: readFile(binaryPath) };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return failure("read-error", `cannot read similar-code binary: ${message}`);
  }
};

const readSignature = (binaryPath, readFile) => {
  try {
    const signature = readFile(`${binaryPath}.sig`);
    return signature.length === 64
      ? { ok: true, signature }
      : failure("sig-invalid", "signature has an unexpected length");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return failure("sig-missing", `cannot read detached signature: ${message}`);
  }
};

const verifySignature = (binary, signature, verify, createPublicKey) => {
  try {
    const key = createPublicKey({
      key: Buffer.concat([SPKI_HEADER, PUBLIC_KEY]),
      format: "der",
      type: "spki",
    });
    return verify(null, binary, key, signature)
      ? { ok: true }
      : failure("sig-invalid", "Ed25519 signature verification failed");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return failure("sig-invalid", `Ed25519 signature verification failed: ${message}`);
  }
};

const verifyDigest = (binary, expectedDigest) => {
  const actual = crypto.createHash("sha256").update(binary).digest();
  const expected = Buffer.from(expectedDigest, "hex");
  return crypto.timingSafeEqual(actual, expected)
    ? { ok: true }
    : failure("digest-mismatch", "embedded SHA-256 digest does not match the binary");
};

const verifyBinary = (
  artifact,
  {
    readFile = fs.readFileSync,
    verify = crypto.verify,
    createPublicKey = crypto.createPublicKey,
  } = {},
) => {
  const artifactResult = validateArtifact(artifact);
  if (!artifactResult.ok) return artifactResult;

  const manifestResult = readManifestDigest(artifact, readFile);
  if (!manifestResult.ok) return manifestResult;

  const binaryResult = readBinary(artifact.binaryPath, readFile);
  if (!binaryResult.ok) return binaryResult;

  const signatureResult = readSignature(artifact.binaryPath, readFile);
  if (!signatureResult.ok) return signatureResult;

  const signatureVerification = verifySignature(
    binaryResult.bytes,
    signatureResult.signature,
    verify,
    createPublicKey,
  );
  if (!signatureVerification.ok) return signatureVerification;

  return verifyDigest(binaryResult.bytes, manifestResult.digest);
};

module.exports = {
  PUBLIC_KEY,
  SPKI_HEADER,
  verifyBinary,
};
