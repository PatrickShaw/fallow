import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";

import {
  classifyFinding,
  evaluatePairCoverage,
  loadManifest,
  parseArgs,
  runConformance,
  validateFallowReport,
} from "./semantic-clone-conformance.mjs";
import { parseArgs as parseModelArgs } from "./semantic-clone-model-evidence.mjs";

const REPO_ROOT = resolve(import.meta.dirname, "..");
const MANIFEST = resolve(REPO_ROOT, "tests/semantic-clone-corpus/manifest.json");

const countLines = (path) => {
  const contents = readFileSync(path, "utf8");
  return contents.length === 0 ? 0 : contents.replace(/\n$/, "").split("\n").length;
};

const mockReport = (loaded, testCase, cloneGroups = []) => ({
  kind: "dupes",
  schema_version: loaded.manifest.deterministic_baseline.schema_version,
  clone_groups: cloneGroups,
  stats: {
    total_files: testCase.files.length,
    total_lines: testCase.files.reduce(
      (total, file) => total + countLines(resolve(loaded.root, file.fixture)),
      0,
    ),
  },
});

test("manifest locks public fixture provenance and relationship labels", () => {
  const { manifest } = loadManifest(MANIFEST);

  assert.equal(manifest.source.public_fixture, true);
  assert.match(manifest.source.revision, /^[0-9a-f]{40}$/);
  assert.equal(manifest.vector_fixture.dimensions, 256);
  assert.equal(manifest.vector_fixture.quality_evidence, false);
  assert.ok(manifest.cases.some((entry) => entry.truth.candidate_worthy));
  assert.ok(manifest.cases.some((entry) => !entry.truth.candidate_worthy));
});

test("candidate evidence pins local execution and complete case coverage", () => {
  const loaded = loadManifest(MANIFEST);

  assert.equal(loaded.candidateEvidence.length, 1);
  assert.equal(loaded.candidateEvidence[0].provider.source_left_machine, false);
  assert.match(loaded.candidateEvidence[0].provider.runtime_lock_sha256, /^[0-9a-f]{64}$/);
  assert.match(loaded.candidateEvidence[0].model.revision, /^[0-9a-f]{40}$/);
  assert.deepEqual(
    loaded.candidateEvidence[0].profiles.map((profile) => [
      profile.dimensions,
      profile.model_supports_dimensions,
    ]),
    [
      [768, true],
      [256, false],
    ],
  );
});

test("pair coverage requires both files in one clone group", () => {
  const result = evaluatePairCoverage(
    [
      {
        instances: [
          { file: "a.ts", start_line: 1, end_line: 8 },
          { file: "b.ts", start_line: 3, end_line: 10 },
        ],
      },
      {
        instances: [{ file: "a.ts", start_line: 1, end_line: 10 }],
      },
    ],
    ["a.ts", "b.ts"],
    new Map([
      ["a.ts", 10],
      ["b.ts", 20],
    ]),
  );

  assert.deepEqual(result, {
    best_pair_coverage: 0.4,
    matching_groups: 1,
  });
});

test("finding classification uses refactor safety rather than candidate value", () => {
  assert.equal(classifyFinding(true, true), "true_positive");
  assert.equal(classifyFinding(true, false), "false_positive");
  assert.equal(classifyFinding(false, true), "false_negative");
  assert.equal(classifyFinding(false, false), "true_negative");
});

test("conformance output keeps candidate gaps separate from findings", () => {
  const loaded = loadManifest(MANIFEST);
  const result = runConformance(
    loaded,
    (_binary, testCase) =>
      mockReport(
        loaded,
        testCase,
        testCase.id === "renamed-identifiers-ts-02"
          ? [
              {
                instances: [
                  { file: "a.ts", start_line: 1, end_line: 32 },
                  { file: "b.ts", start_line: 1, end_line: 32 },
                ],
              },
            ]
          : [],
      ),
    "/fixture/fallow",
  );

  assert.equal(result.cases[0].candidate_gap, true);
  assert.equal(result.cases[0].deterministic_finding.classification, "true_negative");
  assert.equal(
    result.cases.find((entry) => entry.id === "renamed-identifiers-ts-02").deterministic_finding
      .classification,
    "true_positive",
  );
  assert.equal(result.candidate_evidence[0].profiles[0].added_vs_deterministic, 1);
  assert.deepEqual(result.candidate_evidence[0].profiles[0].classifications, {
    true_positive: 2,
    false_positive: 0,
    false_negative: 1,
    true_negative: 4,
  });
  assert.equal(result.summary.baseline_drift, 0);
});

test("conformance rejects incomplete or out-of-scope fallow reports", () => {
  const loaded = loadManifest(MANIFEST);
  const testCase = loaded.manifest.cases[0];
  const fileNames = testCase.files.map((file) => file.fixture.split("/").at(-1));
  const fileLineCounts = new Map(
    testCase.files.map((file, index) => [
      fileNames[index],
      countLines(resolve(loaded.root, file.fixture)),
    ]),
  );

  assert.throws(
    () =>
      validateFallowReport(
        { clone_groups: [], stats: {} },
        testCase,
        fileNames,
        fileLineCounts,
        loaded.manifest.deterministic_baseline,
      ),
    /report kind must be dupes/,
  );
  assert.throws(
    () =>
      validateFallowReport(
        mockReport(loaded, testCase, [
          {
            instances: [
              { file: fileNames[0], start_line: 1, end_line: 2 },
              { file: "outside.ts", start_line: 1, end_line: 2 },
            ],
          },
        ]),
        testCase,
        fileNames,
        fileLineCounts,
        loaded.manifest.deterministic_baseline,
      ),
    /outside the locked pair/,
  );
});

test("conformance reports a locked baseline mismatch", () => {
  const loaded = loadManifest(MANIFEST);
  const result = runConformance(
    loaded,
    (_binary, testCase) => mockReport(loaded, testCase),
    "/fixture/fallow",
  );

  assert.equal(result.summary.baseline_drift, 1);
  assert.equal(
    result.cases.find((entry) => entry.id === "renamed-identifiers-ts-02").deterministic_finding
      .matches_expected,
    false,
  );
});

test("argument parsing is explicit", () => {
  assert.deepEqual(parseArgs(["--fallow-bin", "/tmp/fallow", "--pretty"]), {
    check: false,
    fallowBin: "/tmp/fallow",
    manifest: MANIFEST,
    pretty: true,
  });
  assert.equal(parseArgs(["--check"]).check, true);
  assert.throws(() => parseArgs(["--unknown"]), /unknown argument/);
});

test("model evidence requires locked runtime provenance", () => {
  assert.deepEqual(
    parseModelArgs([
      "--runtime-lock",
      "/tmp/package-lock.json",
      "--transformers-module",
      "/tmp/transformers.node.mjs",
      "--model-cache-state",
      "cold",
    ]),
    {
      manifest: MANIFEST,
      modelCacheState: "cold",
      runtimeLock: "/tmp/package-lock.json",
      transformersModule: "/tmp/transformers.node.mjs",
    },
  );
  assert.throws(
    () => parseModelArgs(["--transformers-module", "/tmp/transformers.node.mjs"]),
    /--runtime-lock is required/,
  );
  assert.throws(
    () =>
      parseModelArgs([
        "--runtime-lock",
        "/tmp/package-lock.json",
        "--transformers-module",
        "/tmp/transformers.node.mjs",
        "--model-cache-state",
        "maybe",
      ]),
    /requires cold, warm, or unknown/,
  );
});
