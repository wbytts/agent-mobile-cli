# agent-mobile-cli

面向 Agent 的 Android 设备感知与控制 CLI：把真实 Android 设备变成可复用的 UI 快照、点击/滑动/输入、截图、应用管理、logcat 与 shell 能力。架构参照 [agent-browser-cli](https://github.com/sleepinginsummer/agent-browser-cli)（CLI + 常驻 daemon），移动端版本。

- **ADB 直连模式**（当前）：复用主机 adb，无需在设备安装任何组件
- **调试 App 代理模式**（规划中）：Tauri 2 调试 App 经 WS 桥接接入，支持设备端脚本沙盒

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
agent-mobile-cli daemon-status|daemon-restart|daemon-stop
```

多设备在线时必须用 `--device <serial>` 指定目标（否则返回 `DEVICE_AMBIGUOUS` 并列出候选）。完整命令与 SOP 见 [skills/agent-mobile-cli/SKILL.md](skills/agent-mobile-cli/SKILL.md)。

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
