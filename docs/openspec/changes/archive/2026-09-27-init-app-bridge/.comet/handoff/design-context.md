# Comet Design Handoff

- Change: init-app-bridge
- Phase: design
- Mode: compact
- Context hash: 9a5fc2aa1bd81fbf6669cc0d421278c1e2664487b00f45d1fe0d3915416f0e01

Generated-by: comet-handoff.sh
Task hash policy: task-content-v1. Read tasks.md for live completion; excerpts are design-time context.

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## docs/openspec/changes/init-app-bridge/proposal.md

- Source: docs/openspec/changes/init-app-bridge/proposal.md
- Lines: 1-33
- SHA256: d075e6f0704153fee0feba383487a954317e1d7d9ed9208f75a5f2343a5b51f9

```md
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

```

## docs/openspec/changes/init-app-bridge/design.md

- Source: docs/openspec/changes/init-app-bridge/design.md
- Lines: 1-63
- SHA256: 6f69ab75b9d9f96631da9782823cd30d26964bf34e9cbbfe7c89872fa3d68e23

```md
---
comet_change: init-app-bridge
role: technical-design
canonical_spec: openspec
---

# Design

## Context

依赖 change `init-adb-core` 交付的 daemon、桥接 WS 监听端口与后端抽象层。动机与范围见 proposal.md。测试环境：MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a）。用户已确认：代理模式 = 脚本沙盒 + 设备操作 API；平台 Android 优先、协议预留多后端。

## Goals / Non-Goals

**Goals:**

- 定稿桥接 WS 协议并两端实现（daemon、App）
- Tauri 2 Android App 在 MuMu 上可安装、可连接、可执行命令与脚本
- 桥接设备接入 CLI 统一设备模型，控制命令契约与 ADB 后端一致

**Non-Goals:**

- iOS 端、root/注入式能力、脚本市场、App 加固
- ADB 后端已有命令的任何修改
- 无线 adb 与桥接的自动切换/择优策略

## Decisions

1. **App 采用 Tauri 2 + Kotlin 原生插件**：UI 与连接管理用 Tauri 2（WebView 前端 + Rust 核心），设备能力封装为 Tauri 插件调用 Kotlin 实现。备选纯 Kotlin 原生 App 放弃：用户明确建议 Tauri 2，且 UI/配置界面开发效率高。
2. **脚本引擎选 QuickJS**（JNI 集成）：包体积小、启动快、ES2020 支持足够。备选 V8 体积过大，Rhino 过旧。沙盒默认无文件/网络权限，仅注入 `mobile.*` API 绑定。
3. **桥接协议为 JSON 文本帧**：`hello`（设备信息+能力集）、`heartbeat`、`command`（id/method/params）、`result`（id/ok/result|error）、`script`/`script-result`。理由：与 CLI HTTP API 风格一致、调试友好；二进制图像数据（截图）以 base64 内联于 result。备选 protobuf 放弃：首版调试图优先级高于带宽。
4. **截图走 AccessibilityService.takeScreenshot**（API 30+，MuMu Android 12 满足）；失败时不自动回落 MediaProjection（需要用户授权弹窗），直接返回错误，留待后续版本。
5. **UI 树序列化复用 adb 模式同一简化模型**：AccessibilityService `rootInActiveWindow` 递归序列化为与 uiautomator dump 相同的节点结构（文本/类型/坐标区域），CLI 侧复用同一 `@eN` 引用分配逻辑，保证两后端输出结构一致。
6. **能力集协商**：`hello` 消息携带能力列表（tap/swipe/uiTree/screenshot/script），CLI 路由前按能力集校验，缺失即返回不支持错误（对应 app-backend 能力降级要求）。
7. **App 内连接保活**：前台服务（foreground service）维持 WS 连接与心跳，避免系统回收导致频繁掉线。
8. **配对认证模型（双通道）**：桥接监听启动时 daemon 生成 6 位一次性配对码（首次使用或 daemon 重启后失效），`agent-mobile-cli pair` 显示配对码与候选局域网 IP 并在终端输出配对二维码（unicode block），二维码内容 URI `agent-mobile://pair?host=<ip>&port=<port>&code=<code>`；App 可手动输入地址+配对码，或扫码（相机权限 + ML Kit/zxing）解析 URI 自动填入并发起配对。配对码验证通过后 daemon 下发长期 token（32 字节随机 hex，存 daemon 配置目录与 App 本地），后续 hello 凭 token 认证；`agent-mobile-cli pair --reset` 重新生成配对码并使全部已发 token 失效。理由：局域网明文 WS 的最低信任门槛，一次性配对码收窄泄露面，token 可撤销；扫码消除真机手动输入成本（MuMu 模拟器走手动通道）。备选无认证放弃：同网段任意主机可控制设备；备选 mDNS 自动发现放弃：多一套发现协议与平台权限，首版手动/扫码已够用。
9. **App UI 为多页诊断型**：连接页（daemon 地址/配对码输入/连接状态）、能力自检页（无障碍权限状态 + 逐项能力自测按钮）、日志页（最近命令与连接事件）。用户已确认（2026-09-26 晚对账）；替代此前单页基础 UI 的设想。
10. **协议层全在 App 的 Rust core**：WS 客户端（tokio-tungstenite）、协议消息（serde）、QuickJS 沙盒（rquickjs bundled）均在 Rust core；Kotlin 仅承担 AccessibilityService 能力桥、前台服务保活、扫码（ML Kit/zxing）。`mobile.*` 脚本调用经 Tauri 插件命令桥到 Kotlin 执行后回传。理由：协议/沙盒逻辑与 CLI 同栈（tokio/serde）可单测可复用，Kotlin 面最小化。
11. **协议类型单一来源**：CLI 侧协议模块（serde 消息类型）被 App Rust core 以 path 依赖直接引用，两端不各自维护消息定义。
12. **app-bridge 后端能力集**：桥接设备 id 形如 `bridge:<name>`；可用：snapshot/tap/swipe/input/key/screenshot/apps/launch；NOT_SUPPORTED：stop/logcat/shell（用户确认全命令对齐、受限降级），返回结构化能力不支持错误，不静默路由到其他后端。
13. **配对二维码与地址枚举**：qrcode crate 终端 unicode 输出配对 URI；局域网候选 IP 枚举默认路由网卡优先、全部候选列出（多网卡主机）。
14. **token 双侧存储**：daemon 侧配置目录（随 config.json 同目录 tokens 存储）；App 侧 SharedPreferences（私有模式）。`pair --reset` 重新生成配对码并清空全部已发 token。
15. **截图消息内联**：base64 PNG 内联于 result（WS 帧上限默认 16MB，1440×2560 PNG 约 1-3MB 足够）；二进制帧升级留后续。

## Risks / Trade-offs

- Tauri 2 mobile 构建链（Gradle + NDK + Rust targets）复杂度高 → 任务组 1 先行打通「构建出可安装 APK」再叠加能力；固定 toolchain 版本并记录
- 无障碍服务被系统或厂商回收导致命令失败 → App 侧检测服务状态并在 UI 提示；心跳附带权限状态，daemon 可据此标记设备降级
- MuMu arm64 对 QuickJS 原生库兼容性 → 先编译验证 arm64-v8a so 加载，再集成
- base64 截图消息体积大 → 首版接受；后续可升级二进制帧
- 前台服务带来常驻通知 → Android 平台要求，不可去除，UI 中说明用途

## Test Strategy

- Rust core（App 侧）：cargo 单测覆盖协议编解码、QuickJS 沙盒 `mobile.*` 绑定、配对状态机（配对码验证/token 签发与校验/重置失效）
- daemon WS：Rust 集成测试以模拟 WS client 完成 hello（配对码与 token 两种路径）/command/result/script/断线重连全流程
- app-bridge 后端：单测覆盖能力降级错误、devices 合并枚举、桥接路由输出结构与 ADB 后端一致
- Kotlin/UI：MuMu 手动验收（6.1 清单）+ 荣耀真机扫码验证；首版不引入 instrumented test 框架
- 端到端：安装 → 无障碍授权 → 手动与扫码配对 → devices 可见 → snapshot → `mobile.tap` 脚本 → screenshot → 断线重连 → `pair --reset` 旧 token 失效

## Migration Plan

无历史包袱；依赖 `init-adb-core` 归档后产生的 specs 作为基线。

```

## docs/openspec/changes/init-app-bridge/tasks.md

- Source: docs/openspec/changes/init-app-bridge/tasks.md
- Lines: 1-35
- SHA256: ce6444f6bdf6c066f70bd121f5c3d1fce2af762c7740ae77a7b8defd6e510814

```md
# Tasks

## 1. Tauri 工程与构建链

- [ ] 1.1 创建 Tauri 2 Android 工程骨架（含 Gradle/NDK/Rust targets 配置），验证 debug APK 构建成功并在 MuMu 安装启动 <!-- comet-task:567a37b2-c2d7-4a66-afb3-70511fe05d2f -->
- [ ] 1.2 实现 App 多页诊断 UI（连接页：daemon 地址/配对码输入/扫码入口/连接状态；能力自检页：无障碍权限状态与逐项能力自测；日志页：命令与连接事件），验证三页渲染与状态切换 <!-- comet-task:2e4b6717-d3cf-4794-8296-f4441b6b7291 -->

## 2. 桥接协议

- [ ] 2.1 在 daemon 侧实现 WS 协议（hello 注册/heartbeat/command 分发/result 回传），验证模拟 WS 客户端握手注册后设备出现在枚举中 <!-- comet-task:02e6b5e2-7068-4e61-a243-996aae31e593 -->
- [ ] 2.2 编写协议消息 schema 文档（随代码同步维护），验证文档中每种消息与实现字段一致 <!-- comet-task:ce233ce2-7f38-4e65-b248-b6edfe45d979 -->
- [ ] 2.3 实现 daemon 侧配对认证（启动生成一次性配对码、验证配对并下发/校验长期 token、`agent-mobile-cli pair` 显示配对码/候选 IP/终端配对二维码与 `--reset` 重置），验证错误配对码被拒、配对后 token 重连成功、重置后旧 token 失效 <!-- comet-task:7cb6921e-aba4-4f43-b531-469b150c5e6b -->

## 3. Kotlin 原生能力

- [ ] 3.1 实现 AccessibilityService 与 Tauri 插件绑定（点击/滑动/UI 树读取），验证 MuMu 上经插件点击设置项生效 <!-- comet-task:545bd128-daa9-419b-b1f5-dcf22cc03965 -->
- [ ] 3.2 实现截图能力（takeScreenshot）并经插件回传，验证返回非空 PNG 数据 <!-- comet-task:40e47a10-0bd3-4527-a23c-052347446c7f -->
- [ ] 3.3 集成 QuickJS 沙盒插件并绑定 `mobile.*` API（tap/swipe/uiTree/screenshot），验证脚本执行返回结果且无法访问沙盒外资源 <!-- comet-task:ba80be11-91c1-434f-b3ed-1a43a25e9e46 -->
- [ ] 3.4 实现无障碍权限状态检测与开启引导（跳转系统设置），验证未授权时引导展示、授权后状态更新 <!-- comet-task:2a32ec5a-deb6-44e6-b589-b8cc6c589273 -->

## 4. App 内 WS 客户端

- [ ] 4.1 实现前台服务保活的 WS 客户端（连接/注册/心跳/自动重连），验证杀掉 App 重开后 daemon 侧设备先离线后恢复在线 <!-- comet-task:a4554601-2865-4c3c-b2f5-c9403c9b061b -->
- [ ] 4.2 实现命令处理循环（接收 command/script → 分发到插件/沙盒 → 回传 result），验证命令成功与失败两类回传 <!-- comet-task:8a12453e-410d-40ae-81f1-9728dec0ade8 -->
- [ ] 4.3 实现 App 配对与扫码流程（配对码输入提交、token 本地保存与自动凭 token 重连、相机扫码解析 `agent-mobile://pair` URI 自动填入），验证手动与扫码两条路径均配对成功且杀 App 重连免配对 <!-- comet-task:22a11751-6d93-4ed3-b560-514e1cc5f995 -->

## 5. CLI 桥接后端

- [ ] 5.1 实现 `app-bridge` 后端的统一接口（snapshot/tap/swipe/input/screenshot 经 WS 路由），验证对桥接设备执行命令输出结构与 ADB 后端一致 <!-- comet-task:ec4dc356-aefc-4499-8ce7-230b7a487b3d -->
- [ ] 5.2 实现能力集校验与降级错误，验证对桥接设备执行 shell 返回「不支持」结构化错误 <!-- comet-task:d7b70f80-52cc-4ebf-8672-538918c5f211 -->
- [ ] 5.3 实现 devices 合并枚举（adb + 桥接设备类型标识），验证两类设备同时在线时均可列出并区分 <!-- comet-task:ea47e93b-5534-41a6-9fbf-e4c5f5bb0766 -->

## 6. 端到端验收

- [ ] 6.1 在 MuMu 执行全链路验收：安装 App → 无障碍授权 → 手动与扫码配对 → devices 可见 → snapshot → 脚本 `mobile.tap` → screenshot → 断线重连恢复 → `pair --reset` 后旧 token 失效，逐项对照 specs 场景通过 <!-- comet-task:09ded594-1ab2-47ba-ac9c-bad71244d5ac -->

```

## docs/openspec/changes/init-app-bridge/.openspec.yaml

- Source: docs/openspec/changes/init-app-bridge/.openspec.yaml
- Lines: 1-2
- SHA256: 062e545e250441aa0692c22d7a65e07334f1c0f2e706c0e297f36065b6a0699c

```md
schema: spec-driven
created: 2026-09-26

```

## docs/openspec/changes/init-app-bridge/specs/agent-app/spec.md

- Source: docs/openspec/changes/init-app-bridge/specs/agent-app/spec.md
- Lines: 1-72
- SHA256: d5d4d9cd2d023b25b7ad0da56259afac3c055fd4c0f5a4fa5bbb9603c517e1a7

```md
# Spec Delta

## Purpose

提供安装在 Android 设备上的调试 App，作为 CLI 的设备端代理：维护与 daemon 的桥接连接，通过无障碍服务执行设备操作，并在脚本沙盒中运行下发的 JS 脚本。

## ADDED Requirements

### Requirement: daemon 连接管理

App SHALL 支持配置 daemon 地址（host:port）与配对码发起桥接连接，或扫码解析配对 URI 自动填入地址与配对码；连接后显示连接状态；连接断开后 SHALL 自动重连（已配对设备凭保存的 token 免配对）。

#### Scenario: 手动配置地址与配对码后连接成功

- **WHEN** 用户在 App 中输入主机、桥接端口与配对码并发起连接
- **THEN** App 显示已连接状态，daemon 侧出现对应设备注册

#### Scenario: 扫码快速配对

- **WHEN** 用户在 App 中扫码识别 `agent-mobile://pair?...` 二维码
- **THEN** App 解析 URI 自动填入地址与配对码并发起连接，无需手动输入

#### Scenario: 配对码错误提示

- **WHEN** App 提交的配对码错误或失效
- **THEN** App 显示认证失败原因并停留在连接页，不产生设备注册

### Requirement: 无障碍设备操作

App SHALL 通过无障碍服务执行点击、滑动与当前窗口 UI 树读取；无障碍权限未开启时 SHALL 提供跳转系统设置的引导并显示权限状态。

#### Scenario: 权限引导

- **WHEN** 无障碍权限未开启
- **THEN** App 显示权限状态与开启引导，可一键跳转系统无障碍设置页

#### Scenario: 开启权限后执行点击

- **WHEN** 无障碍权限已开启且收到点击命令
- **THEN** 设备上对应坐标的界面元素收到点击并产生响应

### Requirement: 脚本沙盒

App SHALL 内嵌 JS 引擎执行 daemon 下发的脚本，向脚本暴露 `mobile.*` 设备操作 API（点击、滑动、UI 树、截图等）；脚本不得访问沙盒外的设备资源，沙盒外能力仅限显式暴露的 API。

#### Scenario: 脚本调用设备 API

- **WHEN** 沙盒执行包含 `mobile.tap(x, y)` 的脚本
- **THEN** 设备对应位置收到点击，脚本继续执行并返回结果

### Requirement: 多页诊断界面

App SHALL 提供多页诊断界面：连接页（地址/配对码/扫码入口/连接状态）、能力自检页（无障碍权限状态与逐项能力自测）、日志页（最近命令与连接事件）。

#### Scenario: 三页切换与状态展示

- **WHEN** 用户在三个页面间切换
- **THEN** 各页正确渲染当前状态（连接状态、权限状态、日志列表）

#### Scenario: 能力自检

- **WHEN** 用户在能力自检页执行某项能力自测（如截图、UI 树读取）
- **THEN** App 在本地执行该项能力并展示成功或失败原因，无需经 daemon 下发命令

### Requirement: 屏幕截图回传

App SHALL 提供当前屏幕截图能力，并将图像数据经桥接连接回传 daemon。

#### Scenario: 截图命令返回图片

- **WHEN** daemon 向 App 下发截图命令
- **THEN** App 截取当前屏幕并回传 PNG 图像数据，CLI 保存为非空文件

```

## docs/openspec/changes/init-app-bridge/specs/app-backend/spec.md

- Source: docs/openspec/changes/init-app-bridge/specs/app-backend/spec.md
- Lines: 1-39
- SHA256: 75d6e0c7c52f495436f24c9150a7c866989e521e624e77a5ed09bcdf22348acd

```md
# Spec Delta

## Purpose

在 CLI 侧实现统一后端接口的桥接后端，使全部设备控制命令可以不经修改地作用于桥接设备，并对桥接设备不支持的能力给出明确降级。

## ADDED Requirements

### Requirement: 桥接设备合并枚举

CLI 设备枚举 SHALL 合并展示 ADB 设备与桥接设备，输出中包含各设备的后端类型标识。

#### Scenario: 两类设备同时在线

- **WHEN** 一台设备经 adb 连接、另一台经 App 桥接注册，用户执行设备枚举
- **THEN** 输出同时列出两台设备且后端类型可区分

### Requirement: 控制命令桥接路由

对桥接设备执行 UI 快照、点击、滑动、文本输入、截图命令时，CLI SHALL 经桥接连接路由执行，命令参数与输出结构与 ADB 后端一致。

#### Scenario: 桥接设备快照

- **WHEN** 用户指定桥接设备执行快照命令
- **THEN** 输出简化 UI 结构，元素引用模型与 ADB 后端一致

#### Scenario: 桥接设备点击

- **WHEN** 用户指定桥接设备执行点击命令
- **THEN** 命令经桥接连接下发，设备界面产生相应响应

### Requirement: 能力降级

桥接设备不支持的能力（如 shell 透传）SHALL 返回明确的能力不支持错误，不得静默失败或路由到其他后端。

#### Scenario: 不支持的命令

- **WHEN** 用户对桥接设备执行 shell 透传命令
- **THEN** 命令以「该后端不支持此能力」的结构化错误结束

```

## docs/openspec/changes/init-app-bridge/specs/bridge-protocol/spec.md

- Source: docs/openspec/changes/init-app-bridge/specs/bridge-protocol/spec.md
- Lines: 1-77
- SHA256: b3c2107cc0d3dda22565ed529ee71bb835025f4440d1795aed650b110054339c

```md
# Spec Delta

## Purpose

定义 CLI daemon 与设备端调试 App 之间的 WebSocket 桥接协议，使设备无需 adb 连接即可接入 CLI 的统一设备模型，接收命令与脚本并回传结果。

## ADDED Requirements

### Requirement: 设备连接与注册

设备端 App 连接 daemon 的桥接 WS 端口后 SHALL 完成注册握手，上报设备名称、Android 版本与能力集；注册成功后 daemon SHALL 将该设备纳入设备枚举，后端类型标记为桥接类型。

#### Scenario: 注册后设备可见

- **WHEN** App 与 daemon 建立 WS 连接并完成注册
- **THEN** CLI 设备枚举结果中包含该设备，且后端类型与 ADB 设备可区分

### Requirement: 心跳与断线重连

桥接连接 SHALL 维持心跳；连接断开时 daemon SHALL 将对应设备标记为离线，App 恢复连接后 SHALL 自动重新注册并恢复在线状态。

#### Scenario: App 进程被杀后恢复

- **WHEN** App 进程被结束导致连接断开，随后用户重新打开 App
- **THEN** daemon 先将设备标记为离线，App 重连注册后恢复在线

### Requirement: 命令分发与结果回传

daemon SHALL 支持通过桥接连接向指定设备下发命令消息（请求标识、动作、参数），App 执行后 SHALL 回传与该请求关联的结构化结果或错误。

#### Scenario: 命令成功回传

- **WHEN** daemon 向已注册设备下发点击命令
- **THEN** App 执行后回传成功结果，CLI 输出与 ADB 后端一致的 `ok: true` 结构

#### Scenario: 命令失败回传

- **WHEN** App 执行命令失败（如目标元素不可交互）
- **THEN** 回传错误信息，CLI 输出结构化错误并以非零退出码结束

### Requirement: 配对认证

daemon SHALL 在桥接监听启动时生成一次性配对码，并提供 CLI 命令（`agent-mobile-cli pair`）显示配对码、候选局域网地址与配对二维码（URI 含 host/port/配对码）；App 首次注册 SHALL 提交配对码，验证通过后 daemon 下发长期 token，后续连接凭 token 认证；配对码错误或缺失时 daemon SHALL 拒绝注册。

#### Scenario: 显示配对信息

- **WHEN** 用户执行 `agent-mobile-cli pair`
- **THEN** 输出当前配对码、候选局域网地址与配对二维码（URI `agent-mobile://pair?host=...&port=...&code=...`）

#### Scenario: 首次配对成功

- **WHEN** App 首次连接并提交正确配对码（手动输入或扫码 URI 解析）
- **THEN** daemon 验证通过并下发长期 token，App 保存 token 并完成注册

#### Scenario: 凭 token 重连免配对

- **WHEN** 已配对 App 使用保存的 token 重新连接
- **THEN** daemon 验证 token 后直接完成注册，无需再次输入配对码

#### Scenario: 配对码错误被拒

- **WHEN** App 提交错误或已失效的配对码
- **THEN** daemon 拒绝注册并返回认证错误，设备不出现在枚举中

#### Scenario: 重置配对

- **WHEN** 用户执行 `agent-mobile-cli pair --reset`
- **THEN** daemon 重新生成配对码，此前下发的全部 token 失效，已连接设备被要求重新配对

### Requirement: 脚本下发执行

协议 SHALL 支持脚本动作，携带 JS 源码由 App 在脚本沙盒中执行，并将脚本返回值或异常回传 daemon。

#### Scenario: 脚本返回执行结果

- **WHEN** daemon 下发一段调用设备操作 API 的脚本
- **THEN** App 在沙盒中执行并回传脚本返回值；脚本抛异常时回传错误信息

```
