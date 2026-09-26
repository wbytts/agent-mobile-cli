# Comet Design Handoff

- Change: init-adb-core
- Phase: design
- Mode: compact
- Context hash: f62970ab124ea48d04be387ce0a8731bea3046cf676b34200291c707e1e772de

Generated-by: comet-handoff.sh
Task hash policy: task-content-v1. Read tasks.md for live completion; excerpts are design-time context.

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## docs/openspec/changes/init-adb-core/proposal.md

- Source: docs/openspec/changes/init-adb-core/proposal.md
- Lines: 1-36
- SHA256: 419407a6d9041974b6828d60ca48259049e7a4e52ada25b9a6c276998951b74f

```md
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

```

## docs/openspec/changes/init-adb-core/design.md

- Source: docs/openspec/changes/init-adb-core/design.md
- Lines: 1-79
- SHA256: 2e1c888177d9c9f09f20e13fc4c8fd37e04ddbf6fc5397df54825c1c7a4b3b91

```md
---
comet_change: init-adb-core
role: technical-design
canonical_spec: openspec
---

# Design

## Context

新仓库，无任何既有源码；动机与范围见 proposal.md。主机环境：macOS arm64，Rust 1.93 与 Node 22 就绪；adb 位于 `~/Library/Android/sdk/platform-tools`。测试设备：MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a）。架构参照 agent-browser-cli：CLI 短进程 + 常驻 daemon（HTTP API + 桥接 WS 双端口）+ npm wrapper 分发。本 change 只交付 ADB 后端；WS 桥接端口仅做监听与握手占位，消息协议随 change `init-app-bridge` 定稿。

## Goals / Non-Goals

**Goals:**

- 单二进制 CLI 内含 daemon，后续命令毫秒级复用连接
- backend 抽象使后续 App 桥接作为新后端类型接入，控制命令契约不变
- ADB 后端覆盖 specs 中全部感知与控制命令，在 MuMu 模拟器上可验收

**Non-Goals:**

- App 桥接协议的消息格式与设备端实现（属 `init-app-bridge`）
- 内置/自动下载 adb 二进制（依赖主机 platform-tools）
- adb 无线配对（`adb pair`）流程自动化、USB 授权引导
- 投屏/视频流

## 架构

```text
agent-mobile-cli (CLI 短进程, clap 解析参数)
   |  HTTP POST /cmd  (127.0.0.1:18775)
   v
daemon (同一二进制 daemon 子命令, 后台驻留, 无状态转发)
   |-- HTTP API: GET /health, POST /cmd {argv}
   |-- 桥接 WS 监听 (127.0.0.1:18777, 本期握手占位)
   |-- 启动锁 ~/.agent-mobile-cli/daemon.lock (pid + port + started_at)
   |-- 后端注册表 { "adb": AdbBackend }  (预留 "app-bridge")
   |-- 快照引用缓存 (内存, @eN -> 坐标, 每次 snapshot 刷新)
   v
adb 子进程 (主机 platform-tools)  ->  adb server (5037)  ->  MuMu 127.0.0.1:5555
```

模块划分（单 crate 多模块）：`main`（CLI 入口）、`cli`（命令树与参数）、`config`（用户级配置）、`daemon`（lock/http/ws）、`backend`（统一接口 + 路由）、`adb`（子进程封装）、`ui`（uiautomator 解析 + 简化树 + @eN 引用）、`output`（JSON + 错误模型）。

## Decisions

1. **单进程双端口 daemon，无状态转发**：daemon 同时监听 HTTP API 与桥接 WS 端口；每条命令到达后 fork adb 子进程执行（adb server 5037 本身常驻，连接天然复用），daemon 不内嵌 adb 协议长连接。备选「daemon 内嵌 adb 长连接协议」放弃：adb fork 约 30-80ms 可接受，内嵌属过度设计；备选「无 daemon」放弃：WS 桥接端口必须常驻，且与参考项目模型不一致。
2. **HTTP API 单入口转发**：`GET /health` + `POST /cmd {argv}`。clap 在 CLI 侧解析参数，daemon 只做执行器；新增命令不需要改协议。备选「每命令一个 REST 端点」放弃：API 面随命令数膨胀。
3. **ADB 通过子进程调用主机 adb 实现**，不引入 Rust 原生 ADB 协议客户端。全部调用显式 `-s <serial>`；默认超时 10s（截图/文件推拉 30s）。探测顺序：配置文件 `adb_path` → `ANDROID_HOME`/`ANDROID_SDK_ROOT` → PATH → 平台常见安装路径。
4. **UI 快照走 uiautomator dump**：设备端生成 UI XML → 拉取解析 → 简化树 → 可交互节点按先序分配 `@eN` 引用。简化规则：剔除零面积节点与无文本无描述且不可交互的容器（递归上提子节点）；可交互判定为 clickable/long-clickable/focusable/scrollable 为真或带文本的叶子。引用表记录各 `@eN` 的坐标中心点，存于 daemon 内存，每次 snapshot 全量刷新，daemon 重启失效（文档明示「引用仅对最近一次快照有效」）。
5. **输入边界**：`input text` 仅支持 ASCII（adb input 限制，空格转义为 `%s`）；含非 ASCII 的输入返回 `NOT_SUPPORTED` 结构化错误，不引入 ADBKeyBoard 等 IME 依赖。`key` 接受 keyevent 名称或数字码。
6. **截图/日志/应用的命令选型**：截图 `adb exec-out screencap -p`（exec-out 避免 PTY 换行损坏二进制，需 platform-tools 28+，启动时检测版本）；logcat `adb logcat -d -t <lines>` dump 模式（非阻塞，支持 tag/级别过滤）；启动应用 `monkey -p <pkg> 1`（免查 launcher activity）；停止 `am force-stop`。
7. **输出与错误模型**：stdout 恒为 JSON——成功 `{"ok":true,"result":{...}}`，失败 `{"ok":false,"error":{"code","message","details"}}`；退出码 0 成功、1 执行错误（clap 用法错误为 2）。错误码枚举：`ADB_NOT_FOUND`、`DEVICE_OFFLINE`、`DEVICE_AMBIGUOUS`、`DEVICE_NOT_FOUND`、`NOT_SUPPORTED`、`TIMEOUT`、`ADB_ERROR`、`IO_ERROR`。
8. **启动锁**：锁文件 + 端口探测双重判定，锁内记录 pid 与启动时间，过期锁可回收；CLI 健康检查失败时自动清理并重启 daemon。
9. **配置与端口**：用户级 `~/.agent-mobile-cli/config.json`（`http_port` 默认 18775、`bridge_port` 默认 18777、`adb_path`、`default_device`），缺失自动生成默认值；端口默认值避开 agent-browser-cli 的 18765/18767。
10. **后端抽象层**：统一后端接口（设备枚举、快照、触控、截图、shell、日志、应用管理），设备记录携带后端类型标识，路由按设备 ID 分发；`adb` 为首个实现，`app-bridge` 预留类型位。
11. **分发**：GitHub Release 多平台预编译二进制，npm 包 postinstall 按 `process.platform`/`process.arch` 下载对应二进制（与参考项目模式一致）。

## 测试策略

- **单元测试（无设备依赖）**：adb 输出解析器（devices -l、uiautomator XML、pm list）、简化树与 @eN 分配算法、配置文件生成/读取、启动锁逻辑、错误模型序列化。
- **并发锁测试**：双进程同时抢锁，断言仅一个 daemon 绑定端口、另一个转为复用。
- **集成测试（默认 `#[ignore]`，`AGENT_MOBILE_TEST_DEVICE=127.0.0.1:5555` 启用）**：MuMu 全链路——devices/connect/snapshot/tap/input/screenshot/apps/logcat/shell，逐项对应 specs 场景。
- CI 跑单元与并发测试；集成测试本地手动触发。

## Risks / Trade-offs

- adb 子进程输出格式随 platform-tools 版本漂移 → 优先机器可读输出与显式 `-s`；验证环境固定 MuMu + 当前 platform-tools 版本
- uiautomator dump 在 WebView、游戏 Surface 等页面内容缺失 → spec 仅承诺「当前屏幕 UI 层级」，感知缺口由截图兜底
- MuMu 多实例 adb 端口不固定（5555/16384…） → 设备枚举覆盖网络设备，`connect host:port` 支持主动接入
- daemon 异常退出残留锁文件 → 锁内含 pid 存活检测与过期回收；健康检查失败自动清理重启
- 中文等非 ASCII 输入不支持 → spec 未承诺；返回明确 `NOT_SUPPORTED` 错误
- 引用表内存化，daemon 重启后 @eN 失效 → 换取实现简单，文档明示有效期
- daemon 内嵌会增加首版工作量 → 换来命令延迟与连接稳定性，与参考项目一致的收益，接受

## Migration Plan

新仓库首次落地，无迁移。版本自 `0.1.0` 起。

```

## docs/openspec/changes/init-adb-core/tasks.md

- Source: docs/openspec/changes/init-adb-core/tasks.md
- Lines: 1-43
- SHA256: c24f588ad8056e3202f488de30f4edd8497162cb69d3cd265805336787f978d7

```md
# Tasks

## 1. 项目骨架与配置

- [ ] 1.1 创建 Cargo workspace 与 CLI 骨架（clap 命令树），验证 `cargo run -- --version` 与 `--help` 正常输出
- [ ] 1.2 实现用户级配置文件（`~/.agent-mobile-cli/config.json`，缺失自动生成默认值），单元测试覆盖生成与读取
- [ ] 1.3 完善 .gitignore 与基础 CI（fmt/clippy/test），验证 `cargo test` 全绿

## 2. 常驻 daemon

- [ ] 2.1 实现启动锁（锁文件 + pid 存活检测 + 过期回收），并发启动测试验证只有一个 daemon 绑定端口
- [ ] 2.2 实现 HTTP API 服务（健康检查 + 命令路由入口），验证 curl 健康检查返回 ok
- [ ] 2.3 实现桥接 WS 端口监听与握手占位，验证 WebSocket 客户端握手成功
- [ ] 2.4 实现 daemon 生命周期命令（status/restart/stop），验证 status 输出运行状态与端口

## 3. ADB 后端与路由抽象

- [ ] 3.1 实现 adb 探测链（配置 → 环境变量 → PATH → 常见安装路径），单元测试覆盖优先级与缺失时的指引性错误
- [ ] 3.2 实现设备枚举（解析 `adb devices -l`），验证 `devices` 列出 MuMu 模拟器 127.0.0.1:5555 且状态在线
- [ ] 3.3 实现 `connect host:port`，验证连接后设备出现在枚举结果中
- [ ] 3.4 实现 `--device` 目标选择与多设备歧义错误，验证两台在线设备未指定时输出候选列表
- [ ] 3.5 实现后端抽象层（统一后端接口 + 设备记录后端类型 + 命令路由，预留 app-bridge 类型），单元测试验证 adb 后端路由

## 4. 设备控制命令

- [ ] 4.1 实现 `snapshot`（uiautomator dump 拉取解析 + 简化树 + `@eN` 引用表），验证 MuMu 系统设置页快照含带坐标区域的元素引用
- [ ] 4.2 实现 `tap`/`swipe`（坐标与元素引用两种寻址），验证点击设置项后界面实际跳转
- [ ] 4.3 实现 `input` 文本输入与 `key` 按键事件，验证输入框出现目标文本
- [ ] 4.4 实现 `screenshot`（PNG 保存，支持输出路径），验证文件非空且可打开、内容与屏幕一致
- [ ] 4.5 实现 `apps` 列表（可过滤）与 `launch`/`stop`，验证启动系统设置后前台切换
- [ ] 4.6 实现 `logcat`（最近 N 行 + tag/级别过滤），验证输出行数不超过指定值
- [ ] 4.7 实现 `shell` 透传（返回 stdout/stderr/退出码），验证 `getprop ro.product.model` 返回设备型号
- [ ] 4.8 实现设备离线与操作超时的结构化错误，验证断开模拟器后命令以 device offline 错误结束且不挂起

## 5. 分发与 Agent 文档

- [ ] 5.1 实现 npm wrapper（package.json + postinstall 按平台定位二进制），验证本地打包安装后 `agent-mobile-cli --version` 可执行
- [ ] 5.2 编写 `skills/agent-mobile-cli/SKILL.md`（覆盖全部命令、选项与典型 SOP），验证文档中每条命令与已实现行为一致
- [ ] 5.3 编写 README（安装、快速自检、命令速览），验证快速自检段落命令实跑通过

## 6. 端到端验收

- [ ] 6.1 在 MuMu 模拟器执行全链路验收：devices → connect → snapshot → tap → input → screenshot → logcat → shell，逐项对照 specs 场景通过

```

## docs/openspec/changes/init-adb-core/.openspec.yaml

- Source: docs/openspec/changes/init-adb-core/.openspec.yaml
- Lines: 1-2
- SHA256: 062e545e250441aa0692c22d7a65e07334f1c0f2e706c0e297f36065b6a0699c

```md
schema: spec-driven
created: 2026-09-26

```

## docs/openspec/changes/init-adb-core/specs/cli-daemon/spec.md

- Source: docs/openspec/changes/init-adb-core/specs/cli-daemon/spec.md
- Lines: 1-66
- SHA256: 6539da8f7c0d3374256f669fdb41b28ebf7e5df7afd8bd491cd38744b16045b6

```md
# Spec Delta

## Purpose

为 Agent 提供稳定的命令行入口与常驻 daemon 服务，避免每次命令重复初始化设备连接，并为设备端桥接代理预留接入端口；同时提供跨平台的 npm 安装分发。

## ADDED Requirements

### Requirement: 结构化命令输出

所有 CLI 命令 SHALL 以 JSON 输出结构化结果（成功时 `ok: true` 与结果负载，失败时 `ok: false` 与错误描述），失败时以非零退出码结束。

#### Scenario: 命令成功

- **WHEN** 用户执行任意读取类命令且执行成功
- **THEN** stdout 输出 `ok: true` 的 JSON，进程退出码为 0

#### Scenario: 命令失败

- **WHEN** 命令因目标设备离线而失败
- **THEN** stdout 输出 `ok: false` 与错误原因的 JSON，进程退出码非零

### Requirement: 常驻 daemon 会话复用

首个需要设备连接的命令 SHALL 自动启动常驻 daemon；后续命令 SHALL 通过 daemon 的 HTTP API 复用已建立的连接，不重复初始化。

#### Scenario: 连续命令复用连接

- **WHEN** 用户连续执行两条需要设备连接的命令
- **THEN** 第二条命令复用 daemon 已有连接，不重新执行设备连接初始化

### Requirement: daemon 启动锁

并发启动 daemon 时 SHALL 通过启动锁保证同一时刻只有一个 daemon 实例绑定服务端口。

#### Scenario: 并发首跑

- **WHEN** 两个 CLI 进程同时执行各自的首条命令
- **THEN** 只有一个 daemon 成功绑定端口，另一个进程转为复用该 daemon

### Requirement: 用户级配置文件

CLI SHALL 支持用户级配置文件自定义服务端口等参数；配置文件缺失时 SHALL 自动生成默认值。

#### Scenario: 配置文件缺失

- **WHEN** 用户删除配置文件后执行任意命令
- **THEN** CLI 自动重新生成包含默认端口配置的文件并按默认值运行

### Requirement: 桥接 WebSocket 监听

daemon SHALL 在独立于 HTTP API 的端口上监听 WebSocket 连接，供设备端桥接代理接入；该端口可在配置文件中修改。

#### Scenario: daemon 启动后端口监听

- **WHEN** daemon 完成启动
- **THEN** 配置的桥接 WS 端口处于监听状态并接受 WebSocket 握手

### Requirement: npm 安装分发

项目 SHALL 提供 npm 包，安装时按当前操作系统与架构定位对应的 CLI 预编译二进制。

#### Scenario: 全局安装后可执行

- **WHEN** 用户通过 npm 全局安装该包
- **THEN** `agent-mobile-cli --version` 可直接执行并输出版本号

```

## docs/openspec/changes/init-adb-core/specs/device-backends/spec.md

- Source: docs/openspec/changes/init-adb-core/specs/device-backends/spec.md
- Lines: 1-52
- SHA256: a7c983728a6b153d605b0673a118a6461bb2d25835dd592259197e87ca69ae94

```md
# Spec Delta

## Purpose

定义统一的设备模型与后端路由层，使设备控制命令不感知具体连接方式（ADB、桥接代理等），并首先提供可用的 ADB 直连后端。

## ADDED Requirements

### Requirement: 设备发现与枚举

CLI SHALL 枚举当前可达的 Android 设备，输出设备标识、型号、连接方式（USB/网络/桥接）与在线状态。

#### Scenario: 列出模拟器设备

- **WHEN** MuMu 模拟器已通过 adb 连接（`127.0.0.1:5555`）且用户执行设备枚举
- **THEN** 输出中包含该设备，状态为在线，连接方式为网络

### Requirement: adb 环境检测

当主机上 adb 不可用或无法连接 adb server 时，CLI SHALL 返回明确的错误信息与安装/修复指引，不得静默成功或异常崩溃。

#### Scenario: adb 缺失

- **WHEN** 主机 PATH 与常见安装位置均不存在 adb，用户执行设备枚举
- **THEN** 命令失败并输出指引性错误信息

### Requirement: 多设备目标选择

存在多个在线设备时，CLI SHALL 支持通过命令参数指定目标设备；未指定且在线设备多于一台时 SHALL 返回列出候选设备的歧义错误。

#### Scenario: 多设备未指定目标

- **WHEN** 两台设备同时在线且用户执行控制命令时未指定目标设备
- **THEN** 命令失败并输出候选设备列表

### Requirement: 网络设备主动连接

CLI SHALL 支持通过 `host:port` 主动连接网络 Android 设备。

#### Scenario: 连接网络设备

- **WHEN** 用户执行连接命令指定 `127.0.0.1:5555`
- **THEN** 该设备随后出现在设备枚举结果中

### Requirement: 后端路由抽象

设备控制命令 SHALL 经由统一的后端接口路由到目标设备所属后端执行；新增后端类型时控制命令的行为契约保持不变。

#### Scenario: 同一命令跨后端

- **WHEN** 用户对 ADB 后端设备执行点击命令
- **THEN** 命令按目标设备的后端类型路由执行，命令参数与输出结构与后端类型无关

```

## docs/openspec/changes/init-adb-core/specs/device-control/spec.md

- Source: docs/openspec/changes/init-adb-core/specs/device-control/spec.md
- Lines: 1-75
- SHA256: 86d06e96e790ee8ae5b1161f82e329470310db25c8c8f35c2c4ac98f4d42ffcf

```md
# Spec Delta

## Purpose

让 Agent 对已连接的 Android 设备进行屏幕感知（UI 快照、截图、日志）与交互操作（触控、输入、应用管理），输出面向 Agent 优化的简化结构。

## ADDED Requirements

### Requirement: 屏幕 UI 快照

CLI SHALL 获取目标设备当前屏幕的 UI 层级，输出简化后的结构：为可交互元素分配稳定引用（如 `@e1`），并包含元素文本、类型与屏幕坐标区域。

#### Scenario: 快照包含元素引用

- **WHEN** 设备停留在包含可点击控件的页面，用户执行快照命令
- **THEN** 输出为简化结构，其中可交互元素带有引用标识、文本与坐标区域

### Requirement: 触控与文本输入

CLI SHALL 支持按屏幕坐标或元素引用执行点击、滑动，向焦点控件输入文本，以及发送按键事件。

#### Scenario: 按元素引用点击

- **WHEN** 用户对快照中的元素引用执行点击
- **THEN** 设备上对应位置的元素收到点击，界面产生相应响应

#### Scenario: 文本输入

- **WHEN** 焦点位于输入框时用户执行文本输入命令
- **THEN** 输入框中出现该文本

### Requirement: 屏幕截图

CLI SHALL 截取目标设备当前屏幕并保存为 PNG 文件，支持指定输出路径。

#### Scenario: 截图保存成功

- **WHEN** 用户执行截图命令并指定输出路径
- **THEN** 该路径下生成非空 PNG 文件且内容与设备当前屏幕一致

### Requirement: 应用管理

CLI SHALL 支持列出设备已安装应用（可按名称过滤），以及按包名启动、停止应用。

#### Scenario: 启动系统设置

- **WHEN** 用户执行启动命令指定系统设置包名
- **THEN** 设备前台切换到系统设置页面

### Requirement: 日志读取

CLI SHALL 支持读取设备 logcat 最近 N 行输出，并可按 tag 或级别过滤。

#### Scenario: 读取最近日志

- **WHEN** 用户请求最近 50 行日志
- **THEN** 输出不超过 50 行的日志内容

### Requirement: shell 命令透传

CLI SHALL 支持在目标设备上执行 shell 命令，返回其 stdout、stderr 与退出码。

#### Scenario: 查询设备型号

- **WHEN** 用户透传执行 `getprop ro.product.model`
- **THEN** 输出中包含设备型号字符串与退出码 0

### Requirement: 操作错误处理

当目标设备在操作过程中离线或操作超时时，CLI SHALL 返回结构化错误，不得挂起或无输出退出。

#### Scenario: 操作中设备离线

- **WHEN** 控制命令执行期间设备断开连接
- **THEN** 命令以设备离线的结构化错误结束

```
