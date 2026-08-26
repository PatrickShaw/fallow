"use strict";

const PACKAGES = Object.freeze({
  "darwin-arm64": "@fallow-cli/fallow-similar-code-darwin-arm64",
  "darwin-x64": "@fallow-cli/fallow-similar-code-darwin-x64",
  "linux-arm64-gnu": "@fallow-cli/fallow-similar-code-linux-arm64-gnu",
  "linux-arm64-musl": "@fallow-cli/fallow-similar-code-linux-arm64-musl",
  "linux-x64-gnu": "@fallow-cli/fallow-similar-code-linux-x64-gnu",
  "linux-x64-musl": "@fallow-cli/fallow-similar-code-linux-x64-musl",
  "win32-arm64-msvc": "@fallow-cli/fallow-similar-code-win32-arm64-msvc",
  "win32-x64-msvc": "@fallow-cli/fallow-similar-code-win32-x64-msvc",
});

const getPlatformPackage = (platform, arch, libc) => {
  const suffix =
    platform === "linux"
      ? `-${libc === "glibc" ? "gnu" : "musl"}`
      : platform === "win32"
        ? "-msvc"
        : "";
  return PACKAGES[`${platform}-${arch}${suffix}`];
};

const isPlatformPackage = (packageName) => Object.values(PACKAGES).includes(packageName);

module.exports = { getPlatformPackage, isPlatformPackage };
