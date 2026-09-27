// 把 release 构建产物装配为 GitHub Release 资产：dist/release-assets/
//   agent-mobile-cli-<platform>-<arch>[.exe]（asset 命名与 npm postinstall 下载约定一致）
//   checksums.txt（sha256 清单，postinstall 安装时校验）
// 用法：
//   本机平台：  cargo build --release && node scripts/package-platform.mjs
//   交叉平台：  cargo build --release --target x86_64-pc-windows-gnu
//              node scripts/package-platform.mjs --target x86_64-pc-windows-gnu --plat win32-x64
// 全部平台装齐后上传到同一 GitHub Release（tag 需与 npm version 对齐：v<version>）。
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : dflt;
};

const plat = opt("plat", `${process.platform}-${process.arch}`);
const target = opt("target", null);
const isWin = plat.startsWith("win32");
const exe = isWin ? "agent-mobile-cli.exe" : "agent-mobile-cli";
const srcDir = target
  ? path.join(here, "..", "target", target, "release")
  : path.join(here, "..", "target", "release");
const src = path.join(srcDir, exe);
const outDir = path.join(here, "..", "dist", "release-assets");
const asset = `agent-mobile-cli-${plat}${isWin ? ".exe" : ""}`;
const dest = path.join(outDir, asset);

if (!fs.existsSync(src)) {
  console.error(`未找到 ${src}；请先执行对应平台的 cargo build --release`);
  process.exit(1);
}
fs.mkdirSync(outDir, { recursive: true });
fs.copyFileSync(src, dest);
fs.chmodSync(dest, 0o755);

// 刷新 checksums.txt（幂等：按文件名去重重写）
const sha = crypto.createHash("sha256").update(fs.readFileSync(dest)).digest("hex");
const sumsPath = path.join(outDir, "checksums.txt");
const lines = fs.existsSync(sumsPath)
  ? fs.readFileSync(sumsPath, "utf8").split("\n").filter((l) => l.trim() && !l.trimEnd().endsWith(` ${asset}`))
  : [];
lines.push(`${sha}  ${asset}`);
lines.sort();
fs.writeFileSync(sumsPath, lines.join("\n") + "\n");

console.log(`已装配 ${asset}（sha256 ${sha.slice(0, 12)}…）→ dist/release-assets/`);
