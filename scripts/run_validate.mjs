import { spawnSync } from "node:child_process";

const configuredPython = process.env.M2SHELF_PYTHON?.trim();
const candidates = [
  ...(configuredPython ? [[configuredPython, []]] : []),
  ...(process.platform === "win32"
    ? [["python", []], ["py", ["-3"]]]
    : [["python3", []], ["python", []]]),
];

for (const [command, prefix] of candidates) {
  const probe = spawnSync(command, [...prefix, "--version"], {
    encoding: "utf8",
    stdio: "ignore",
  });
  if (probe.error) {
    if (probe.error.code === "ENOENT") continue;
    throw probe.error;
  }
  if (probe.status !== 0) continue;

  const result = spawnSync(command, [...prefix, "-B", "scripts/validate_project.py"], {
    encoding: "utf8",
    stdio: "inherit",
    env: { ...process.env, PYTHONIOENCODING: "utf-8", PYTHONUTF8: "1" },
  });
  if (!result.error) process.exit(result.status ?? 1);
  if (result.error.code !== "ENOENT") throw result.error;
}

console.error(
  "Validation requires Python 3 on PATH, or an absolute interpreter path in M2SHELF_PYTHON.",
);
process.exit(1);
