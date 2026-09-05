import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import YAML from "../sdk/node_modules/yaml/dist/index.js";

const validator = fileURLToPath(new URL("./validate-ci-workflows.mjs", import.meta.url));
const ci = YAML.parse(fs.readFileSync(".github/workflows/ci.yml", "utf8"));
const release = fs.readFileSync(".github/workflows/release.yml", "utf8");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "aura-quickjs-ci-test-"));
const integration = (workflow) => workflow.jobs.build.steps.find(
  (step) => step.name === "Test Java Provider and build native Host",
);
const cases = [
  ["valid six-platform workflow", () => {}, null],
  ["missing native process", (workflow) => {
    integration(workflow).run = "gradle -p host-plugin test jar --rerun-tasks";
  }, "must receive the current platform native Host"],
  ["conditional test", (workflow) => { integration(workflow).if = "false"; }, "without a condition"],
  ["tolerated failure", (workflow) => { integration(workflow)["continue-on-error"] = true; }, "must not tolerate failure"],
  ["test exclusion", (workflow) => { integration(workflow).run += "\ngradle -x test"; }, "must not filter out tests"],
  ["missing build", (workflow) => {
    integration(workflow).run = integration(workflow).run.replace("cargo build --release", "cargo check");
  }, "must be built before mandatory Java"],
  ["missing NPL rejection gate", (workflow) => {
    const step = workflow.jobs.build.steps.find((candidate) => candidate.name === "Package and validate platform NPL");
    step.run = step.run.replace("test-built-host-npl.ps1", "verify-quickjs-host-artifacts.ps1");
  }, "must pass behavioral rejection tests"],
  ["wrong Aura provenance", (workflow) => { workflow.env.AURA_COMMIT = "0".repeat(40); }, "wrong AURA_COMMIT"],
];

try {
  fs.mkdirSync(path.join(temporary, ".github/workflows"), { recursive: true });
  fs.writeFileSync(path.join(temporary, ".github/workflows/release.yml"), release);
  for (const [label, mutate, expectedError] of cases) {
    const workflow = structuredClone(ci);
    mutate(workflow);
    fs.writeFileSync(path.join(temporary, ".github/workflows/ci.yml"), YAML.stringify(workflow));
    const result = spawnSync(process.execPath, [validator], { cwd: temporary, encoding: "utf8", timeout: 10_000 });
    assert.ifError(result.error);
    if (expectedError === null) {
      assert.equal(result.status, 0, `${label}: ${result.stderr}`);
    } else {
      assert.notEqual(result.status, 0, `${label}: unsafe workflow accepted`);
      assert.ok(result.stderr.includes(expectedError), `${label}: ${result.stderr}`);
    }
  }
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
process.stdout.write(`QuickJS workflow behavior: ${cases.length} fixtures passed\n`);
