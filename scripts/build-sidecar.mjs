import {
  existsSync,
  mkdirSync,
  readdirSync,
  statSync,
  utimesSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(import.meta.url);
const root = resolve(dirname(script), "..");
const sidecar = join(root, "sidecar");
const sidecarSource = join(sidecar, "src");
const target = execFileSync("rustc", ["--print", "host-tuple"], {
  encoding: "utf8",
}).trim();
const extension = process.platform === "win32" ? ".exe" : "";
const name = `chamber-agent-sidecar-${target}`;
const output = join(root, "src-tauri", "binaries", `${name}${extension}`);

const inputs = [
  script,
  join(sidecar, ".python-version"),
  join(sidecar, "pyproject.toml"),
  join(sidecar, "uv.lock"),
  ...readdirSync(sidecarSource, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => join(entry.parentPath, entry.name)),
];

if (
  existsSync(output) &&
  inputs.every((input) => statSync(output).mtimeMs >= statSync(input).mtimeMs)
) {
  process.exit(0);
}

mkdirSync(dirname(output), { recursive: true });

execFileSync(
  "uv",
  [
    "run",
    "--project",
    sidecar,
    "pyinstaller",
    "--noconfirm",
    "--onefile",
    "--copy-metadata",
    "genai-prices",
    "--copy-metadata",
    "pydantic-ai-slim",
    "--name",
    name,
    "--distpath",
    dirname(output),
    "--workpath",
    join(root, ".build", "pyinstaller"),
    "--specpath",
    join(root, ".build", "pyinstaller"),
    join(sidecar, "src", "server.py"),
  ],
  { stdio: "inherit" },
);

const now = new Date();
utimesSync(output, now, now);
