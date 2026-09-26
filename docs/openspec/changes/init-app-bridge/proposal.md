# Proposal

## Why

ADB 直连模式要求设备与主机 adb 可达，无法覆盖真机无线调试、无 USB 调试授权、以及需要在设备本地执行脚本的场景。参照 agent-browser-cli 的「Chrome 扩展桥」模型，本 change 开发一个基于 Tauri 2 的 Android 调试 App 作为设备端桥接代理：App 主动连接 CLI daemon，接收结构化命令或 JS 脚本，在设备本地通过无障碍服务执行并回传结果。

## What Changes

- 定稿桥接 WebSocket 协议：连接注册（设备信息 + 能力集）、心跳、命令分发、结果回传（在 `init-adb-core` 预留的 WS 监听之上实现）
- 配对认证：daemon 生成一次性配对码（`agent-mobile-cli pair` 显示配对码/局域网地址/配对二维码），App 手动输入或扫码配对后获得长期 token，token 可重置撤销
- 基于 Tauri 2 的 Android 调试 App 骨架与构建链（debug 构建可安装到 MuMu 模拟器）
- Kotlin 原生能力：AccessibilityService 设备操作（点击/滑动/UI 树读取/截图）、内嵌 JS 引擎的脚本沙盒（暴露 `mobile.*` 设备操作 API）、无障碍权限开启引导
- App 内 WS 客户端与多页诊断界面：连接页（地址/配对码/扫码）、能力自检页、日志页；自动重连
- CLI 侧 `app-bridge` 后端：实现 `init-adb-core` 定义的统一后端接口，把控制命令路由到桥接设备；设备枚举合并显示桥接设备；不支持的能力返回明确降级错误

## Capabilities

### New Capabilities

- `bridge-protocol`: CLI daemon 与设备端 App 之间的 WebSocket 桥接协议（注册、心跳、命令分发、脚本下发与结果回传）
- `agent-app`: Tauri 2 Android 调试 App 本体（WS 客户端、无障碍设备操作、脚本沙盒、权限引导）
- `app-backend`: CLI 侧桥接后端（统一后端接口实现、桥接设备枚举、能力降级）

### Modified Capabilities

（无；`device-backends` 的抽象契约已在 `init-adb-core` 定义，本 change 新增其实现而非修改其要求）

## Impact

- 新增：`app/`（Tauri 2 工程，含 Kotlin 原生插件）、CLI 新增 `app-bridge` 后端模块、协议文档
- 依赖：change `init-adb-core` 的 daemon、桥接 WS 端口与后端抽象层（须先归档或至少完成 Build）
- 测试环境：MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a），App 需安装 APK 并手动开启无障碍权限
- 不做：iOS、脚本市场、App 加固混淆、root/注入式能力
