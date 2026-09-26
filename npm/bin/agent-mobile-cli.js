#!/usr/bin/env node
// agent-mobile-cli npm 入口：转发参数到平台二进制（postinstall 已定位/下载）。
const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");

function resolveBinary() {
  const plat = `${process.platform}-${process.arch}`;
  const exe = process.platform === "win32" ? "agent-mobile-cli.exe" : "agent-mobile-cli";
  const vendor = path.join(__dirname, "..", "vendor", plat, exe);
  if (fs.existsSync(vendor)) return vendor;
  return null;
}

const bin = resolveBinary();
if (!bin) {
  console.log(
    JSON.stringify({
      ok: false,
      error: {
        code: "IO_ERROR",
        message: `未找到 ${process.platform}-${process.arch} 平台的 agent-mobile-cli 二进制；请重新安装（npm i -g agent-mobile-cli）或参考 README 从源码 cargo build --release 构建`,
      },
    })
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });
child.on("exit", (code) => process.exit(code === null ? 1 : code));
