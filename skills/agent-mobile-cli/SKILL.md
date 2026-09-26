---
name: agent-mobile-cli
description: 使用 agent-mobile-cli 进行 Android 设备感知与控制。适用于设备扫描/选择、UI 快照（@eN 元素引用）、点击/滑动/输入/按键、截图、应用管理、logcat、shell 透传；ADB 直连模式，无需 root。
---

# agent-mobile-cli

面向 Agent 的 Android 设备感知与控制 CLI。ADB 直连模式：复用主机 adb，无需在设备安装任何组件。所有命令输出结构化 JSON（`ok`/`result`/`error`），失败时非零退出。

## 运行模型

- 首个设备命令自动拉起常驻 daemon（HTTP `127.0.0.1:18775`，桥接 WS `18777` 预留给调试 App）；后续命令经 daemon 复用执行，毫秒级响应。
- daemon 生命周期：`daemon-status` / `daemon-restart` / `daemon-stop`（仅本机直接执行，不经转发）。
- 配置文件 `~/.agent-mobile-cli/config.json`（缺失自动生成）：`http_port`、`bridge_port`、`adb_path`、`default_device`。

## 设备选择

多设备在线时，控制命令必须用 `--device <serial>` 指定目标，否则返回 `DEVICE_AMBIGUOUS` 并列出候选。解析顺序：`--device` → 配置 `default_device` → 仅一台在线。

```bash
agent-mobile-cli devices                          # 枚举设备（id/model/state/connection）
agent-mobile-cli connect 127.0.0.1:5555           # 连接网络设备（模拟器常用）
```

## UI 快照与元素引用

```bash
agent-mobile-cli snapshot --device 127.0.0.1:5555
# → { device, tree, refs }：tree 为简化缩进树；refs 为可交互元素表（@eN、text、center、bounds）
```

- `@eN` 引用仅对**最近一次 snapshot** 有效（存于 daemon 内存，daemon 重启失效）。操作前重新 snapshot。
- 简化规则：剔除零面积节点与无内容容器；`@eN` 只给可交互节点（clickable/focusable/scrollable），纯文本节点保留在树中但无引用。
- WebView/游戏 Surface 等页面 uiautomator 可能缺内容 → 用 `screenshot` 兜底感知。

## 交互命令

```bash
agent-mobile-cli tap @e3 --device <serial>              # 按元素引用点击
agent-mobile-cli tap 540 --y 300 --device <serial>      # 按坐标点击
agent-mobile-cli swipe 100 800 100 200 --duration 300 --device <serial>
agent-mobile-cli input "hello world" --device <serial>  # 仅 ASCII（空格自动转义）；非 ASCII 返回 NOT_SUPPORTED
agent-mobile-cli key KEYCODE_HOME --device <serial>     # 按键事件（名称或数字码）
```

## 感知与管理命令

```bash
agent-mobile-cli screenshot --out shot.png --device <serial>   # PNG（相对路径按执行命令时的目录解析）
agent-mobile-cli apps --filter settings --device <serial>      # 已安装应用（默认第三方；--all 全部）
agent-mobile-cli launch com.android.settings --device <serial>
agent-mobile-cli stop com.android.settings --device <serial>
agent-mobile-cli logcat --lines 50 --tag App --level E --device <serial>
agent-mobile-cli shell getprop ro.product.model --device <serial>   # 返回 stdout/stderr/exit_code
```

## 典型 SOP

1. **探索页面**：`snapshot` → 读 tree/refs → `tap @eN` → 再 `snapshot` 确认跳转。
2. **表单输入**：`tap` 聚焦输入框 → `input "text"`（ASCII）→ `key KEYCODE_ENTER`。
3. **视觉确认**：`screenshot --out x.png` 后读取图片核对。
4. **排障**：命令返回 `DEVICE_OFFLINE` 时先 `devices` 确认在线状态；`ADB_NOT_FOUND` 时按错误指引安装 platform-tools 或在配置中设置 `adb_path`。

## 错误码

`ADB_NOT_FOUND`（主机无 adb）、`DEVICE_OFFLINE`、`DEVICE_AMBIGUOUS`（多设备未指定）、`DEVICE_NOT_FOUND`、`NOT_SUPPORTED`（如非 ASCII 输入）、`TIMEOUT`、`ADB_ERROR`、`IO_ERROR`、`USAGE`（参数语义错误，退出码 2）。
