// postinstall：按平台定位预置二进制（vendor/<platform>-<arch>/）。
// 发布流程：scripts/package-platform.mjs 在发版前把各平台 cargo build --release 产物放入 vendor/。
// 缺失时给出可操作的指引（不静默成功，也不自动从网络下载未校验的二进制）。
const fs = require("fs");
const path = require("path");

const plat = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "agent-mobile-cli.exe" : "agent-mobile-cli";
const vendor = path.join(__dirname, "vendor", plat, exe);

if (fs.existsSync(vendor)) {
  fs.chmodSync(vendor, 0o755);
  console.log(`agent-mobile-cli: 已就绪（${plat}）`);
  process.exit(0);
}

// 本地开发回退：仓库根 target/release 存在构建产物时链接过来，便于源码安装调试
const repoRelease = path.join(__dirname, "..", "..", "target", "release", exe);
if (fs.existsSync(repoRelease)) {
  fs.mkdirSync(path.dirname(vendor), { recursive: true });
  fs.copyFileSync(repoRelease, vendor);
  fs.chmodSync(vendor, 0o755);
  console.log(`agent-mobile-cli: 已从本地 target/release 装配（${plat}）`);
  process.exit(0);
}

console.warn(
  `agent-mobile-cli: 警告——包内未包含 ${plat} 二进制，且未发现本地 target/release 构建产物。\n` +
    `  源码安装：在仓库根执行 cargo build --release 后重新 npm install；\n` +
    `  或等待发布版（vendor 预置多平台二进制）。`
);
// 安装不失败：bin 转发脚本在运行时会给出结构化错误
process.exit(0);
