// postinstall：从 GitHub Releases 下载当前平台预编译二进制并做 sha256 校验。
// 发布流程：GitHub Release tag v<version> 上传 agent-mobile-cli-<platform>-<arch>[.exe] 与 checksums.txt，
//           然后 npm publish；安装时本脚本按 package.json version 拼出对应 Release 资产 URL。
// 下载源顺序：AGENT_MOBILE_CLI_GH_PROXY 环境变量 → GitHub 直连 → 内置加速代理（ghfast.top / gh-proxy.com / gh.llkk.cc）。
// 每个源独立下载 checksums.txt 与二进制并校验 sha256，失败换下一源（防代理缓存损坏/篡改）。
// 回退：仓库本地 target/release 存在构建产物时直接装配（源码开发）；全部失败仅警告，不使安装失败。
const fs = require("fs");
const path = require("path");
const https = require("https");
const crypto = require("crypto");

const REPO = "wbytts/agent-mobile-cli";
const plat = `${process.platform}-${process.arch}`;
const isWin = process.platform === "win32";
const exe = isWin ? "agent-mobile-cli.exe" : "agent-mobile-cli";
const vendorDir = path.join(__dirname, "vendor", plat);
const vendorExe = path.join(vendorDir, exe);

// 内置 GitHub 加速代理（前缀拼接式，实测可用性会变，置于直连之后兜底）
const MIRRORS = [
  process.env.AGENT_MOBILE_CLI_GH_PROXY, // 用户自定义优先
  "", // 空串 = GitHub 直连
  "https://ghfast.top",
  "https://gh-proxy.com",
  "https://gh.llkk.cc",
].filter((m) => m !== undefined);

function log(ok, msg) {
  console[ok ? "log" : "warn"](`agent-mobile-cli: ${msg}`);
}

function fetch(url, dest, timeoutMs = 45000) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "agent-mobile-cli-postinstall" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        return resolve(fetch(res.headers.location, dest, timeoutMs));
      }
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`HTTP ${res.statusCode}`));
      }
      if (dest) {
        const out = fs.createWriteStream(dest);
        res.pipe(out);
        out.on("finish", () => out.close(resolve));
        out.on("error", reject);
      } else {
        let body = "";
        res.setEncoding("utf8");
        res.on("data", (c) => (body += c));
        res.on("end", () => resolve(body));
      }
    });
    req.on("error", reject);
    req.setTimeout(timeoutMs, () => req.destroy(new Error("下载超时")));
  });
}

function sha256File(p) {
  return crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex");
}

// 从单一源下载并校验；返回实际 sha256 或抛错
async function trySource(base, asset, tmp) {
  const checksums = await fetch(`${base}/checksums.txt`);
  await fetch(`${base}/${asset}`, tmp, 120000);
  const want = checksums
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.endsWith(` ${asset}`) || l.endsWith(`*${asset}`));
  if (!want) throw new Error(`checksums.txt 无 ${asset} 条目`);
  const wantSha = want.split(/\s+/)[0].toLowerCase();
  const got = sha256File(tmp);
  if (got !== wantSha) throw new Error(`sha256 不匹配（期望 ${wantSha.slice(0, 12)}… 实际 ${got.slice(0, 12)}…）`);
  return got;
}

async function main() {
  if (fs.existsSync(vendorExe)) {
    log(true, `已就绪（${plat}）`);
    return;
  }

  // 源码开发回退：本地构建产物优先于网络下载
  const repoRelease = path.join(__dirname, "..", "..", "target", "release", exe);
  if (fs.existsSync(repoRelease)) {
    fs.mkdirSync(vendorDir, { recursive: true });
    fs.copyFileSync(repoRelease, vendorExe);
    fs.chmodSync(vendorExe, 0o755);
    log(true, `已从本地 target/release 装配（${plat}）`);
    return;
  }

  const version = require("./package.json").version;
  const relPath = `https://github.com/${REPO}/releases/download/v${version}`;
  const asset = `agent-mobile-cli-${plat}${isWin ? ".exe" : ""}`;
  const tmp = path.join(require("os").tmpdir(), `amc-${version}-${plat}-${Date.now()}`);
  const errors = [];

  for (const mirror of MIRRORS) {
    const base = mirror ? `${mirror}/${relPath}` : relPath;
    const tag = mirror || "github.com 直连";
    try {
      const sha = await trySource(base, asset, tmp);
      fs.mkdirSync(vendorDir, { recursive: true });
      fs.copyFileSync(tmp, vendorExe);
      fs.chmodSync(vendorExe, 0o755);
      try { fs.unlinkSync(tmp); } catch {}
      log(true, `已装配（${plat}，来源 ${tag}，sha256 ${sha.slice(0, 12)}… 已校验）`);
      return;
    } catch (e) {
      errors.push(`${tag}: ${e.message}`);
    }
  }

  try { fs.unlinkSync(tmp); } catch {}
  log(
    false,
    `警告——${plat} 二进制下载失败：\n  ${errors.join("\n  ")}\n` +
      `  可手动下载：https://github.com/${REPO}/releases/tag/v${version}\n` +
      `  或设代理环境变量后重试：AGENT_MOBILE_CLI_GH_PROXY=<代理前缀> npm install\n` +
      `  或源码构建：cargo build --release 后重新 npm install。`
  );
  // 安装不失败：bin 转发脚本在运行时给出结构化错误
}

main();
