#!/usr/bin/env node
/**
 * 打包前把索引助手编出来并放到 src-tauri/binaries/，
 * 供 tauri.conf.json 的 bundle.resources 一起塞进安装包。
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const isWindows = process.platform === "win32";
const exeName = isWindows ? "xtools-indexd.exe" : "xtools-indexd";

console.log("[indexd] 编译 xtools-indexd …");
execFileSync("cargo", ["build", "-p", "xtools-indexd", "--release"], {
  cwd: root,
  stdio: "inherit",
});

const built = join(root, "target", "release", exeName);
if (!existsSync(built)) {
  console.error(`[indexd] 找不到构建产物：${built}`);
  process.exit(1);
}

const destDir = join(root, "src-tauri", "binaries");
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, exeName);
copyFileSync(built, dest);
console.log(`[indexd] 已复制到 ${dest}`);
