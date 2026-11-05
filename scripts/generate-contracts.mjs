import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = path.join(root, "packages/contracts/src/generated");
for (const [crate, example] of [["pkms-core", "export_core_contracts"], ["pkms-app", "export_app_contracts"]]) {
  const result = spawnSync("cargo", ["run", "--locked", "-p", crate, "--example", example, "--", output], { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
// ts-rs emits trailing spaces around field comments. Normalize generated output
// here so contract regeneration is reproducible and git diff --check is useful.
for (const entry of readdirSync(output, { withFileTypes: true })) {
  if (!entry.isFile() || !entry.name.endsWith(".ts")) continue;
  const filename = path.join(output, entry.name);
  const source = readFileSync(filename, "utf8");
  const normalized = source.split(/\r?\n/).map((line) => line.trimEnd()).join("\n");
  if (source !== normalized) writeFileSync(filename, normalized, "utf8");
}
