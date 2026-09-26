# Proposal

## Why

Agent 缺少面向移动端的感知与控制 CLI：agent-browser-cli 通过「CLI daemon + 浏览器扩展桥」解决了浏览器场景，移动端（Android）没有等价工具。本 change 初始化 agent-mobile-cli 项目，先落地不依赖任何手机 App 的 ADB 直连模式，并把架构设计成可插拔后端，为后续「调试 App 代理」模式（change init-app-bridge）预留协议与路由层。

## What Changes

- 初始化 Rust workspace：单二进制 CLI（`agent-mobile-cli`），内含常驻 daemon（参考 agent-browser-cli 的「CLI HTTP API + 桥接 WS」双端口模型与启动锁设计）
- 设备后端抽象层：统一设备模型与 backend 路由，本期实现 ADB 后端，协议上预留 App 桥接后端
- ADB 直连能力：adb server/设备发现、设备选择（含 `adb connect host:port`）
- 面向 Agent 的设备控制命令集：
  - `devices` 设备扫描、`snapshot` 当前屏幕 UI 层级 dump 并简化为带元素引用的结构（类比 simphtml）
  - `tap` / `swipe` / `input` / `key` 交互操作、`screenshot` 截图
  - `apps` 应用列表 / `launch` / `stop` 应用管理、`logcat` 日志读取、`shell` 原生命令透传
- npm wrapper 分发：postinstall 按平台下载预编译二进制（与参考项目一致）
- `skills/agent-mobile-cli/SKILL.md`：面向 Agent 的使用参考

## Capabilities

### New Capabilities

- `cli-daemon`: CLI 命令入口与常驻 daemon 服务（HTTP API、桥接 WS 监听、启动锁、配置文件、daemon 生命周期管理），以及 npm wrapper 安装分发
- `device-backends`: 设备后端抽象（统一设备模型、backend 选择与路由）与 ADB 后端实现（adb server 发现、设备枚举与连接状态）
- `device-control`: 通过已连接设备执行感知与控制操作（UI 快照简化、触控输入、截图、应用管理、日志、shell 透传）

### Modified Capabilities

（无）

## Impact

- 新增：`Cargo.toml` workspace、`src/`（cli/daemon/backend/adb）、`npm/`（package.json + postinstall）、`skills/agent-mobile-cli/`
- 外部依赖：主机需可获取 Android platform-tools（adb）；测试设备为 MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a）
- 与 change `init-app-bridge` 的关系：本 change 定义的 backend 抽象与桥接 WS 协议是后者的依赖；本 change 不实现 App 端
- 不影响任何既有代码（本仓库此前无源码）
