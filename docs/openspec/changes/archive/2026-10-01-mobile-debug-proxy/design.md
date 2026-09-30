---
comet_change: mobile-debug-proxy
role: technical-design
canonical_spec: openspec
archived-with: 2026-10-01-mobile-debug-proxy
status: final
---

# Design

## Context

现有桥接链路为「App →（LAN WS）→ 本机 daemon → CLI」。daemon 桥接 WS 绑 `0.0.0.0:18777`，App 必须直连主机；跨网段时该链路不可用。协议帧定义单一来源为 `src/bridge_proto.rs`（App Rust core 以 path 引用同一文件）。CLI 全部设备命令经统一 `Backend` trait 路由，桥接设备经 daemon registry + oneshot 同步等待。本设计在不改变上述本地链路的前提下，新增一条「App → 公网代理 → 本机 daemon → CLI」的通路。

## Goals / Non-Goals

**Goals:**

- 公网可部署的单二进制代理服务 `mobile-debug-proxy-server`（Axum），App 出站 WS 绑定、CLI 侧经 HTTP API 间接调试
- 复用 bridge_proto 帧协议，App 侧改动最小（仅连接目标可配置）
- 多 owner 隔离，配对码 + token 认证模型与现有语义一致
- 本地 LAN 模式零行为变化

**Non-Goals:**

- 服务多实例水平扩展/集群、外部数据库（单实例 JSON 文件持久化足够）
- 内置 TLS 终结（由部署侧反向代理负责，见决策 6）
- Web 管理界面、配额计费
- ADB 协议经公网转发（仅桥接通路）

## Decisions

### 决策 1：代理服务为仓库根目录独立 crate，path 引用共享协议文件

`mobile-debug-proxy-server/` 自带 `Cargo.toml` 独立构建（根 `Cargo.toml` 保持单 package，不转 workspace，避免影响现有 CLI/CI 发版矩阵）。协议类型用 `#[path = "../src/bridge_proto.rs"]` 引用，与 App Rust core 的做法一致，保持消息定义单一来源。

- 备选：根转 Cargo workspace + 抽 shared proto crate。结构更正规，但改动 CI 构建/发版路径与现有 `tests/` 的 `#[path]` 模块树，代价大于收益。
- 备选：proxy 内复制一份协议定义。违背单一来源原则，弃。

### 决策 2：CLI 侧经本机 daemon「uplink」接入代理，而非 CLI 直连

daemon 新增 proxy 客户端：配置代理地址 + owner token 后，daemon 周期性调用代理 `GET /devices` 刷新远端设备注册表（设备 id 形如 `proxy:<device_name>`，BackendKind 新增 `proxy`），控制命令经 `POST /devices/:name/commands|scripts` 同步等待结果。CLI 命令层、`resolve_target` 目标选择、输出契约全部复用，用户无感。

- 备选：CLI 直连代理 API 实现新 Backend。需要在同步 Backend trait 内新引入 HTTP client 依赖、改动目标解析与错误模型，且绕过「一切经 daemon」的既有架构，弃。
- 代价：链路多一跳（App→proxy→daemon），延迟略高于 LAN 直连，可接受（远程调试场景本就走公网）。

### 决策 3：代理服务 API 面

- `WS /ws/device`：App 绑定端点，帧协议与 bridge-protocol 完全一致（hello/heartbeat/command/script/result/result_ack/pong）。
- HTTP（`Authorization: Bearer <owner-token>`）：
  - `POST /pairing-codes`：签发一次性配对码（6 位，首次使用或 reset 后失效）
  - `GET /devices`：owner 名下设备枚举（在线状态、能力集、last_seen）
  - `POST /devices/:name/commands` `{method, params}`：中继 command，挂起等待 result（默认 30s 超时）后返回
  - `POST /devices/:name/scripts` `{source}`：同上，对应 script 帧
  - `POST /pairing-reset`：重置配对码并吊销 owner 全部设备 token
  - `GET /healthz`：无认证健康检查
- command/script 的请求 id 由代理服务按设备连接分配（`cmd-<n>`，与 daemon 语义一致），HTTP 请求与 WS result 按 id 关联。

### 决策 4：认证与持久化

- owner token：服务首次启动自动生成（64 hex）打印到 stdout 并写入数据文件；支持 `--owner-token`/环境变量显式注入（ systemd/docker 部署场景）。
- 设备 token：配对码验证通过时下发，语义与 daemon 一致。
- 持久化：单 JSON 数据文件（owners、设备记录、token、配对码状态），parking_lot 保护并发，与 daemon `tokens.json` 同款 boring 方案，不引入数据库。

### 决策 5：配对入口复用 `agent-mobile-cli pair`

daemon 配置代理后，`agent-mobile-cli pair --proxy` 经 daemon 调用代理 `POST /pairing-codes` 取配对码，输出配对 URI（`agent-mobile://pair?host=<proxy>&port=<port>&code=<code>`）与终端二维码；App 现有扫码解析无需改动。`--reset` 对应代理 `POST /pairing-reset`。

### 决策 6：传输安全由部署侧终结

服务监听明文 HTTP/WS（默认 `0.0.0.0:28777`，可配置），公网部署时由 nginx/caddy 等反向代理终结 TLS（wss/https）。不在服务内实现 TLS：证书管理不是调试工具的核心，反向代理是运维标准做法，文档给出配置示例。

### 决策 7：App 侧仅扩展连接目标

App 连接页增加「服务器类型」选择（本机 daemon / 代理服务器）或统一为地址输入 + 配对 URI 扫码（URI 天然携带目标地址）。Rust core 的 WS client、握手、心跳、重连逻辑不变；连接状态展示当前目标类型。不允许双连接：切换目标先断开原连接。

## Risks / Trade-offs

- 公网暴露面：owner token 泄露即全设备可控 → token 64 hex 随机、`pairing-reset` 一键吊销、文档强制建议 TLS + 防火墙最小暴露
- 中间网络设备断开空闲 WS → 心跳（30s）+ App 自动重连，与现有桥接一致
- HTTP 同步挂起等待设备 result 占用连接 → 30s 超时 + 每设备命令串行化（与 daemon 行为一致），超时返回结构化错误
- JSON 文件持久化在多设备/多 owner 规模下的上限 → 单实例定位为个人/小团队调试工具，超出规模属 Non-Goals
- daemon uplink 轮询枚举存在秒级延迟 → 命令路径为实时 API 调用不受影响；枚举延迟可接受

## Migration Plan

纯新增通路，无既有数据/行为迁移。部署步骤：构建并启动 `mobile-debug-proxy-server` → 记录 owner token → CLI 配置代理地址与 token → `pair --proxy` 出码 → App 扫码绑定。回滚：移除 CLI 代理配置即恢复纯本地模式。

> Build 实施调整（2026-09-29，详见 .comet/rulings.md）：决策 2/11 的「daemon 轮询刷新注册表」改为 ProxyBackend 实时 `GET /devices` 查询；决策 5 的「经 daemon 调代理签发配对码」改为 CLI 直连代理服务。行为与验收契约不变。

## 深化决策（Design 阶段确认，2026-09-29）

### 决策 8：proxy crate = lib + 薄 bin

`mobile-debug-proxy-server/` 内 `src/lib.rs` 承载全部逻辑，模块划分：`store`（JSON 持久化）、`auth`（Bearer 中间件）、`ws`（/ws/device 握手与消息循环）、`http`（REST 路由）、`relay`（per-device 连接句柄 + pending id→oneshot 映射）；`src/main.rs` 仅解析参数并启动。lib 形态使 CLI 与 proxy 的集成测试可进程内起服务（绑随机端口），不起二进制进程。

### 决策 9：命令中继 id 与串行化

请求 id 由服务侧全局 `AtomicU64` 分配（`cmd-<n>`）。每设备一个 mpsc 串行工作协程：HTTP 命令入队 → 协程逐条经 WS 下发并挂起等待 result → 按 id 唤醒 HTTP 等待方。与 daemon 每设备串行语义一致，天然避免同设备并发命令交错。

### 决策 10：多 owner 模型从简

数据文件模型支持多 owner 记录；首次启动自动生成一个 owner token；追加 owner 仅通过重复 `--owner-token` 启动参数或手工编辑数据文件。v1 不提供 owner 管理 API（Non-Goal：Web 管理台）。

### 决策 11：daemon uplink 用 ureq 同步 HTTP

`Backend` trait 方法在 daemon 的 `spawn_blocking` 内执行；项目记忆明确 blocking 线程内不可用 `block_in_place`/`block_on`（panic），同步 HTTP client 是唯一安全选项。选 `ureq`（纯 Rust + rustls，依赖树小）而非 reqwest blocking（依赖过重）。仅需 JSON GET/POST + 超时，ureq 能力足够。

- uplink 轮询：`GET /devices` 每 5s（可配）刷新注册表；轮询失败时 proxy 设备标记 offline（记录保留），恢复后自动回在线。
- 错误映射：401 → 结构化代理认证错误；404 → 设备离线；超时 → TIMEOUT。

### 决策 12：App 连接目标以地址形式区分

App 地址输入升级为两种形式：`host:port`（LAN 直连 daemon，现状）与完整 `ws://|wss://host:port`（代理服务器）；UI 显示当前目标类型；切换目标先断开原连接。扫码配对 URI（`agent-mobile://pair?host=...&port=...&code=...`）格式不变，天然兼容两种目标。

### 决策 13：测试策略

- proxy lib 单元测试：store 加载/保存/首启生成；auth 通过/拒绝；relay 配对三路径、离线标记与重连恢复、重置吊销、owner 隔离、30s 超时、迟到 result 丢弃。
- proxy 集成测试：进程内起 axum 服务（随机端口），tokio-tungstenite 模拟 App，HTTP 客户端全链路验证绑定→配对→tap 往返。
- CLI 集成测试：复用 tests/ `#[path]` 模块树（按项目记忆 tests-path 模式补声明），进程内起 proxy 服务验证 devices 合并枚举与命令路由。
- App：MuMu 模拟器手动验收（连接代理、扫码、切换目标）。
