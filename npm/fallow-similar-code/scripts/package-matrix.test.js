"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const companion = require("../package.json");

test("all optional native packages are exact-version compatible", () => {
  for (const [name, version] of Object.entries(companion.optionalDependencies)) {
    assert.equal(version, companion.version);
    const directory = name.replace("@fallow-cli/fallow-similar-code-", "fallow-similar-code-");
    const manifestPath = path.join(__dirname, "..", "..", directory, "package.json");
    const native = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    assert.equal(native.name, name);
    assert.equal(native.version, companion.version);
  }
});
