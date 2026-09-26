// 把本机 cargo build --release 产物装配进 npm/vendor/<platform>-<arch>/，供 npm pack 预置。
// 用法：cargo build --release && node scripts/package-platform.mjs
// 多平台发布：在 CI 各平台构建后收集到同一 npm/vendor/ 再 pack（本脚本只管本机平台）。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const plat = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "agent-mobile-cli.exe" : "agent-mobile-cli";
const src = path.join(here, "..", "target", "release", exe);
const destDir = path.join(here, "..", "npm", "vendor", plat);
const dest = path.join(destDir, exe);

if (!fs.existsSync(src)) {
  console.error(`未找到 ${src}；请先执行 cargo build --release`);
  process.exit(1);
}
fs.mkdirSync(destDir, { recursive: true });
fs.copyFileSync(src, dest);
fs.chmodSync(dest, 0o755);
console.log(`已装配 ${plat} → npm/vendor/${plat}/${exe}`);
