import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  statSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const rootDir = dirname(dirname(scriptPath));
const tauriDir = join(rootDir, "src-tauri");
const targetDir = join(tauriDir, "target");
const releaseDir = join(targetDir, "release");
const sourceDll = join(tauriDir, "avatar_data.dll");
const destinationDll = join(releaseDir, "avatar_data.dll");
const probeExe = join(
  releaseDir,
  process.platform === "win32" ? "avatar_resource_probe.exe" : "avatar_resource_probe",
);

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const build = spawnSync(
  cargo,
  [
    "build",
    "--manifest-path",
    join(tauriDir, "Cargo.toml"),
    "--target-dir", targetDir,
    "--release",
    "--bin", "avatar_resource_probe",
  ],
  { cwd: rootDir, stdio: "inherit" },
);
if (build.error) {
  throw build.error;
}
if (build.status !== 0) {
  process.exit(build.status ?? 1);
}
if (!existsSync(probeExe) || !statSync(probeExe).isFile()) {
  throw new Error(`资源排查工具构建完成但找不到可执行文件：${probeExe}`);
}
if (!existsSync(sourceDll) || !statSync(sourceDll).isFile() || statSync(sourceDll).size === 0) {
  throw new Error(`缺少本地 avatar_data.dll：${sourceDll}`);
}

mkdirSync(releaseDir, { recursive: true });
copyFileSync(sourceDll, destinationDll);

const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const sourceHash = sha256(sourceDll);
const destinationHash = sha256(destinationDll);
if (sourceHash !== destinationHash) {
  throw new Error("复制后的 avatar_data.dll SHA-256 与源文件不一致");
}

console.log(`资源排查工具：${probeExe}`);
console.log(`数据文件：${destinationDll}`);
console.log(`avatar_data.dll SHA-256：${destinationHash}`);
