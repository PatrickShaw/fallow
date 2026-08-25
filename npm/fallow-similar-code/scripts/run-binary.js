"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { getPlatformPackage } = require("./platform-package");
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

const resolveBinary = (
  packageName,
  resolve = require.resolve,
  readFile = fs.readFileSync,
  stat = fs.lstatSync,
) => {
  if (typeof packageName !== "string") {
    throw new Error(`unsupported platform: ${process.platform}-${process.arch}`);
  }
  const manifestPath = resolve(`${packageName}/package.json`);
  const manifest = JSON.parse(readFile(manifestPath, "utf8"));
  if (manifest.version !== ownManifest.version) {
    throw new Error(
      `version mismatch: fallow-similar-code ${ownManifest.version} requires ${packageName} ${ownManifest.version}, found ${manifest.version}`,
    );
  }
  const binaryName =
    process.platform === "win32" ? "fallow-similar-code.exe" : "fallow-similar-code";
  const binaryPath = path.join(path.dirname(manifestPath), binaryName);
  const metadata = stat(binaryPath);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new Error(`native sidecar is not a regular file: ${binaryPath}`);
  }
  return binaryPath;
};

const run = (args) => {
  try {
    const packageName = resolvePlatformPackage();
    const binary = resolveBinary(packageName);
    const verification = verifyBinary(binary);
    if (!verification.ok) {
      throw new Error(`binary verification failed: ${verification.message}`);
    }
    const result = spawnSync(binary, args, { env: process.env, stdio: "inherit" });
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

module.exports = { resolveBinary, resolvePlatformPackage, run };
