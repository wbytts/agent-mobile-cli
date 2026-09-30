# agent-mobile-cli

面向 Agent 的 Android 设备感知与控制 CLI：把真实 Android 设备变成可复用的 UI 快照、点击/滑动/输入、截图、应用管理、logcat 与 shell 能力。架构参照 [agent-browser-cli](https://github.com/sleepinginsummer/agent-browser-cli)（CLI + 常驻 daemon），移动端版本。

- **ADB 直连模式**：复用主机 adb，无需在设备安装任何组件
- **调试 App 代理模式**：Tauri 2 调试 App（Agent Mobile Bridge）经 WS 桥接接入，设备端 QuickJS 脚本沙盒（`mobile.*` API），扫码配对即可用

## AI 一句话安装

```text
请阅读 https://github.com/wbytts/agent-mobile-cli/blob/main/AI_INSTALL.md，按说明安装 CLI、配置 skill，并按需完成手机 App（Agent Mobile Bridge）安装与配对。
```

## 安装

```bash
npm install -g agent-mobile-cli
agent-mobile-cli devices
```

前置条件：主机可获取 Android platform-tools（`adb`）。探测顺序：配置 `adb_path` → `ANDROID_HOME`/`ANDROID_SDK_ROOT` → PATH → 常见安装路径。

源码构建：

```bash
cargo build --release
./target/release/agent-mobile-cli devices
```

## 快速自检

以 MuMu 模拟器为例（默认 adb 端口 5555）：

```bash
agent-mobile-cli connect 127.0.0.1:5555
agent-mobile-cli devices
# → { "ok": true, "result": { "devices": [ { "id": "127.0.0.1:5555", "state": "online", ... } ] } }
agent-mobile-cli snapshot --device 127.0.0.1:5555
# → 简化 UI 树 + @eN 元素引用
agent-mobile-cli screenshot --out shot.png --device 127.0.0.1:5555
```

## 常用命令

```bash
agent-mobile-cli devices                              # 枚举设备
agent-mobile-cli connect <host:port>                  # 连接网络设备
agent-mobile-cli snapshot [--device <serial>]         # UI 快照（简化树 + @eN 引用）
agent-mobile-cli tap @e1 --device <serial>            # 元素引用 / 坐标点击（tap 540 --y 300）
agent-mobile-cli swipe <x1> <y1> <x2> <y2> [--duration 300]
agent-mobile-cli input "hello" --device <serial>      # 仅 ASCII
agent-mobile-cli key KEYCODE_HOME --device <serial>
agent-mobile-cli screenshot [--out shot.png]
agent-mobile-cli apps [--filter xxx] [--all]
agent-mobile-cli launch|stop <package>
agent-mobile-cli logcat [--lines 100] [--tag T] [--level E]
agent-mobile-cli shell [--device <serial>] <cmd...>   # 选项需写在 cmd 之前
agent-mobile-cli script <file.js|-> --device bridge:<name>   # 桥接设备 JS 沙盒（mobile.*）
agent-mobile-cli pair [--reset]                            # 桥接配对码/二维码/重置
agent-mobile-cli daemon-status|daemon-restart|daemon-stop
```

多设备在线时必须用 `--device <serial>` 指定目标（否则返回 `DEVICE_AMBIGUOUS` 并列出候选）。完整命令与 SOP 见 [skills/agent-mobile-cli/SKILL.md](skills/agent-mobile-cli/SKILL.md)。

## 远程调试（公网代理）

手机与 CLI 主机不在同一局域网时，部署 `mobile-debug-proxy-server` 作为公共中继：

```bash
# 公网主机上（或任何双方可达的主机）
cargo build --release --manifest-path mobile-debug-proxy-server/Cargo.toml
./mobile-debug-proxy-server/target/release/mobile-debug-proxy-server --bind 0.0.0.0:28777
# 首启打印 owner token（64 hex，妥善保管）；数据存 ./proxy-data.json

# CLI 主机：~/.agent-mobile-cli/config.json 增加
#   "proxy": { "url": "http://<proxy-host>:28777", "token": "<owner-token>" }
agent-mobile-cli pair --proxy          # 输出代理配对码与二维码
# 手机 App 扫码，或手动输入 wss://<proxy-host> / http 地址连接
agent-mobile-cli devices               # 远端设备显示为 proxy:<name>
agent-mobile-cli snapshot --device proxy:<name>
```

说明：代理设备经统一后端路由，命令语义与本地桥接一致；`pair --proxy --reset` 重置代理配对。服务本身为明文 HTTP/WS，公网部署请在前面挂反向代理终结 TLS（如 caddy/nginx 反代到 28777），App 侧用 `wss://<域名>` 连接。协议细节见 [docs/bridge-protocol.md](docs/bridge-protocol.md)「公网中继模式」。

## 输出与退出码

所有命令输出 JSON：`{"ok":true,"result":{...}}` 或 `{"ok":false,"error":{"code","message","details"}}`。退出码：0 成功、1 执行错误、2 用法错误。

## 目录结构

```text
src/          # Rust CLI / daemon / ADB 后端 / UI 简化树
npm/          # npm wrapper（bin 转发 + postinstall 平台二进制）
scripts/      # package-platform.mjs（release 产物装配进 npm/vendor）
skills/agent-mobile-cli/  # Agent Skill 文档
docs/openspec/            # OpenSpec 规格与变更管理
```

## 许可

MIT
