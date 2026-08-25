"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { getPlatformPackage, isPlatformPackage } = require("./platform-package");
const { verifyBinary } = require("./verify-binary");
const ownManifest = require("../package.json");

const resolvePlatformPackage = () => {
  if (process.platform !== "linux") {
    return getPlatformPackage(process.platform, process.arch);
  }
  try {
    const { familySync } = require("detect-libc");
    return getPlatformPackage(process.platform, process.arch, familySync());
  } catch {
    return getPlatformPackage(process.platform, process.arch, "musl");
  }
};

const resolveBinaryArtifact = (
  packageName,
  resolve = require.resolve,
  readFile = fs.readFileSync,
  stat = fs.lstatSync,
  platform = process.platform,
) => {
  if (!isPlatformPackage(packageName)) {
    throw new Error(`unsupported similar-code platform package: ${String(packageName)}`);
  }
  const manifestPath = resolve(`${packageName}/package.json`);
  const manifestMetadata = stat(manifestPath);
  if (!manifestMetadata.isFile() || manifestMetadata.isSymbolicLink()) {
    throw new Error(`platform manifest is not a regular file: ${manifestPath}`);
  }
  const manifest = JSON.parse(readFile(manifestPath, "utf8"));
  if (manifest.name !== packageName) {
    throw new Error(
      `platform package ownership mismatch, expected ${packageName} but manifest declares ${String(manifest.name)}`,
    );
  }
  if (manifest.version !== ownManifest.version) {
    throw new Error(
      `version mismatch: fallow-similar-code ${ownManifest.version} requires ${packageName} ${ownManifest.version}, found ${manifest.version}`,
    );
  }
  const binaryName = platform === "win32" ? "fallow-similar-code.exe" : "fallow-similar-code";
  const binaryPath = path.join(path.dirname(manifestPath), binaryName);
  const metadata = stat(binaryPath);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new Error(`native sidecar is not a regular file: ${binaryPath}`);
  }
  return Object.freeze({
    packageName,
    packageVersion: manifest.version,
    manifestPath,
    binaryName,
    binaryPath,
  });
};

const resolveBinary = (...args) => resolveBinaryArtifact(...args).binaryPath;

const run = (args) => {
  try {
    const packageName = resolvePlatformPackage();
    const artifact = resolveBinaryArtifact(packageName);
    const verification = verifyBinary(artifact);
    if (!verification.ok) {
      throw new Error(`binary verification failed: ${verification.message}`);
    }
    const result = spawnSync(artifact.binaryPath, args, { env: process.env, stdio: "inherit" });
    if (result.error) throw result.error;
    if (result.signal) {
      process.stderr.write(`fallow-similar-code terminated by signal ${result.signal}\n`);
      process.exit(1);
    }
    process.exit(result.status ?? 1);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`fallow-similar-code: ${message}\n`);
    process.exit(1);
  }
};

module.exports = { resolveBinary, resolveBinaryArtifact, resolvePlatformPackage, run };
