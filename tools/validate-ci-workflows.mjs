import fs from "node:fs";
import YAML from "../sdk/node_modules/yaml/dist/index.js";

const expectedPlatforms = [
  "windows-x64",
  "windows-arm64",
  "linux-x64",
  "linux-arm64",
  "macos-x64",
  "macos-arm64",
];
const expectedProvenance = {
  AURA_COMMIT: "c2d7ec3201825308c360c1a41aeafebcd7145e74",
  AURA_RUN_ID: "33196503483",
  AURA_JAR_SHA256: "2153be49da69c055232872c95a171091a526be24357b6f2b82b5af8f6d2a67c3",
};

function readWorkflow(file) {
  if (!fs.existsSync(file)) throw new Error(`workflow is missing: ${file}`);
  return YAML.parse(fs.readFileSync(file, "utf8"));
}

function triggerOf(workflow) {
  return workflow.on ?? workflow[true];
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function assertPinnedActions(workflow, label) {
  for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
    for (const step of job.steps ?? []) {
      if (typeof step.uses !== "string") continue;
      assert(
        /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.\/-]+)?@[0-9a-f]{40}$/.test(step.uses),
        `${label} job ${jobName} has an unpinned action: ${step.uses}`,
      );
    }
  }
}

function assertProvenance(workflow, label) {
  for (const [name, value] of Object.entries(expectedProvenance)) {
    assert(String(workflow.env?.[name]) === value, `${label} has wrong ${name}`);
  }
}

function matrixPlatforms(workflow) {
  const include = workflow.jobs?.build?.strategy?.matrix?.include;
  assert(Array.isArray(include), "CI build matrix is missing");
  return include.map((entry) => entry.platform);
}

function stepByName(job, name) {
  const step = job?.steps?.find((candidate) => candidate.name === name);
  assert(step !== undefined, `CI step is missing: ${name}`);
  return step;
}

const ci = readWorkflow(".github/workflows/ci.yml");
const release = readWorkflow(".github/workflows/release.yml");
const ciTriggers = triggerOf(ci);
const releaseTriggers = triggerOf(release);

assert(ciTriggers?.pull_request === undefined, "CI must not expose private Aura access to pull requests");
assert(releaseTriggers?.pull_request === undefined, "release workflow must not run for pull requests");
assert(releaseTriggers?.push?.tags !== undefined, "release workflow must be tag-triggered");
assert(ci.permissions?.contents === "read", "CI contents permission must be read-only");
assert(ci.permissions?.actions === "read", "CI actions permission must be read-only");
assert(release.permissions?.contents === "write", "release contents permission must be write-only scope");
assert(release.concurrency?.group !== undefined, "release concurrency is missing");
assert(release.concurrency?.["cancel-in-progress"] === false, "release must not cancel an active publication");
assertProvenance(ci, "CI");
assertProvenance(release, "release workflow");
assertPinnedActions(ci, "CI");
assertPinnedActions(release, "release workflow");

const actualPlatforms = matrixPlatforms(ci);
assert(
  JSON.stringify(actualPlatforms) === JSON.stringify(expectedPlatforms),
  `CI platform matrix differs: ${actualPlatforms.join(",")}`,
);
assert(ci.jobs?.build?.strategy?.["fail-fast"] === false, "CI matrix must keep all platform evidence");
assert(ci.jobs?.manifest?.needs?.includes?.("build"), "CI manifest job must depend on all platform builds");
const matrix = ci.jobs?.build?.strategy?.matrix?.include;
assert(matrix.find((entry) => entry.platform === "windows-x64")?.msvcArch === "x64", "Windows x64 MSVC architecture is wrong");
assert(matrix.find((entry) => entry.platform === "windows-arm64")?.msvcArch === "arm64", "Windows ARM64 MSVC architecture is wrong");
const msvcSetup = stepByName(ci.jobs?.build, "Set up MSVC");
assert(
  msvcSetup.uses === "ilammy/msvc-dev-cmd@0b201ec74fa43914dc39ae48a89fd1d8cb592756",
  "Windows builds must use the pinned MSVC environment action",
);
const compilerSetup = stepByName(ci.jobs?.build, "Configure native compiler discovery");
assert(
  compilerSetup.run.includes("TARGET_CC=cl.exe"),
  "Windows builds must select cl.exe for QuickJS C11 atomics flags",
);

const releaseText = JSON.stringify(release);
assert(releaseText.includes("verify-quickjs-host-artifacts.ps1"), "release must verify downloaded NPLs");
assert(releaseText.includes("--draft"), "release must remain draft until public verification");
assert(releaseText.includes("--prerelease"), "release must be marked prerelease");

process.stdout.write("QuickJS CI workflow contracts passed\n");
