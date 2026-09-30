# Comet Design Handoff

- Change: mobile-debug-proxy
- Phase: design
- Mode: compact
- Context hash: c56f6573304c93f4a08e99d0ca39ef44097fb5db04ec7d1867bb1639e97ace8e

Generated-by: comet-handoff.sh
Task hash policy: task-content-v1. Read tasks.md for live completion; excerpts are design-time context.

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## docs/openspec/changes/mobile-debug-proxy/proposal.md

- Source: docs/openspec/changes/mobile-debug-proxy/proposal.md
- Lines: 1-32
- SHA256: 31837c88a6e40799d249a8ec70b86dd49dd148cb05be3f770495d96bbf499c1f

```md
# Proposal

## Why

现有桥接模式要求调试 App 与 CLI 所在主机处于同一局域网（或借助 adb reverse 等本机隧道）：daemon 的桥接 WS 监听在主机上，App 必须直连主机 IP。当手机与运行 Agent 工具的机器不在同一局域网（蜂窝网络、跨地域办公、云端 Agent）时，无法建立桥接，远程移动调试完全不可用。

## What Changes

- 在仓库根目录新增独立 Rust 二进制 `mobile-debug-proxy-server`（Axum 框架）：部署在公网可达主机上，作为 App 与 CLI 之间的公共代理/中继服务
- App 通过出站 WebSocket 主动绑定代理服务（穿透 NAT/蜂窝网络），沿用现有配对码 + token 认证模型完成设备注册
- CLI 通过调用代理服务的 HTTP API 间接调试设备：枚举远端设备、下发 command/script、获取结果，无需与设备同网
- 复用并扩展现有 bridge-protocol 帧协议用于中继（消息单一来源仍是 `src/bridge_proto.rs` 或其抽取的共享定义）
- 本地 LAN 直连模式保持不变，代理模式为新增的可选通路，不影响既有行为

## Capabilities

### New Capabilities

- `debug-proxy-server`: 公共代理服务——设备 WS 绑定与配对/token 认证、按 owner 隔离的设备注册表、command/script 中继与结果回传、面向 CLI 的 HTTP API（设备枚举、命令下发、配对码签发）

### Modified Capabilities

- `bridge-protocol`: 帧协议扩展公网中继语义——连接角色（设备侧/中继服务）、请求 id 归属、超时与离线标记在中继链路下的行为
- `agent-app`: App 支持配置并连接公网代理服务器地址（在 LAN daemon 之外新增第二种连接目标）
- `device-backends`: CLI 新增 proxy 设备来源，经代理服务 HTTP API 间接控制远端桥接设备

## Impact

- 新增：`mobile-debug-proxy-server/` 独立 crate（axum、tokio、tokio-tungstenite、serde）
- 修改：`src/bridge_proto.rs`（协议扩展，App Rust core 同文件引用需同步）、`src/backend/`（新增 proxy 后端）、`src/cli.rs`/`src/config.rs`（代理服务地址与凭证配置）、`app/`（App 侧服务器地址配置与连接逻辑）
- 文档：`docs/bridge-protocol.md` 同步协议变更；README/AI_INSTALL 增补远程调试部署说明
- 不影响：ADB 直连后端、本地 LAN 桥接、daemon HTTP 管理端点

```

## docs/openspec/changes/mobile-debug-proxy/design.md

- Source: docs/openspec/changes/mobile-debug-proxy/design.md
- Lines: 1-119
- SHA256: 651619a5cddbf0629a57af3275a55cf7c4c6e3eb65b6201eddb24cd6aa276a50

[TRUNCATED]

```md
---
comet_change: mobile-debug-proxy
role: technical-design
canonical_spec: openspec
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

```

Full source: docs/openspec/changes/mobile-debug-proxy/design.md

## docs/openspec/changes/mobile-debug-proxy/tasks.md

- Source: docs/openspec/changes/mobile-debug-proxy/tasks.md
- Lines: 1-37
- SHA256: 9e5b2f48a569c3051a83b237bf023f62da1225f491e957ee37e9c58386cced7a

```md
# Tasks

## 1. 代理服务骨架与持久化

- [ ] 1.1 创建 `mobile-debug-proxy-server/` 独立 crate（axum 0.7、tokio、tokio-tungstenite、serde、parking_lot、rand、base64），`#[path]` 引用 `../src/bridge_proto.rs`；`cargo build` 通过 <!-- comet-task:fe3d215a-6401-489f-9996-1a37c3669f29 -->
- [ ] 1.2 实现 JSON 数据文件持久化（owners/设备记录/token/配对码状态，parking_lot 保护；首次启动生成 64 hex owner token 打印 stdout 并写盘，支持 `--owner-token` 覆盖）；单元测试覆盖加载/保存/首次生成 <!-- comet-task:8dcbe22f-b0d9-4913-94ae-7549c2de7125 -->
- [ ] 1.3 实现 `GET /healthz` 与 owner token Bearer 认证中间件（无效凭证返回 401 结构化错误）；单元测试覆盖认证通过/拒绝 <!-- comet-task:6a9b97ac-4318-49ef-83d1-2a8bd4f8a1f6 -->

## 2. 设备 WS 绑定与配对

- [ ] 2.1 实现 `WS /ws/device`：hello 握手（配对码或设备 token 认证，10s 首帧超时）、hello_ack、设备注册表按 owner 隔离写入；单元测试覆盖配对码/token/拒绝三路径 <!-- comet-task:b4c53474-4baa-4ed9-aec9-051e798da618 -->
- [ ] 2.2 实现 heartbeat/pong 与离线标记（30s 无消息标离线、断开保留记录、重连凭 token 恢复在线）；单元测试覆盖断线标记与重连恢复 <!-- comet-task:2b6433fe-9de7-4065-8763-f23b20620cf7 -->
- [ ] 2.3 实现 `POST /pairing-codes`（签发 6 位一次性配对码）与 `POST /pairing-reset`（重置配对码、吊销 owner 全部设备 token、断开已连接设备）；单元测试覆盖签发/一次性失效/重置吊销 <!-- comet-task:47454a26-f808-498f-b6a2-bf2f95168ea3 -->

## 3. 命令中继与设备 API

- [ ] 3.1 实现 command/script 中继：`POST /devices/:name/commands`、`POST /devices/:name/scripts` 挂起等待 WS result（id 由服务按连接分配 `cmd-<n>`，30s 超时，设备离线立即报错，迟到 result 静默丢弃）；单元测试覆盖成功/离线/超时三路径 <!-- comet-task:7152029c-2c01-4a1d-81d1-3b1326751365 -->
- [ ] 3.2 实现 `GET /devices` 枚举（在线状态、能力集、last_seen）与跨 owner 访问隔离（返回未找到，不泄露）；单元测试覆盖枚举与隔离 <!-- comet-task:ce43ad3a-a4f7-40cb-a053-c7fd7ebe146a -->
- [ ] 3.3 端到端集成测试：内存 WS 客户端模拟 App 绑定 → 配对 → HTTP 下发 tap → 回传 result → 断言 API 返回成功结构 <!-- comet-task:e83e8538-77d4-4e91-a437-373a8a295ab0 -->

## 4. CLI daemon uplink 接入

- [ ] 4.1 `src/config.rs` 增加代理配置（地址、owner token），`src/backend/mod.rs` 增加 `BackendKind::Proxy` 与 `proxy:<name>` 设备 id 解析；单元测试覆盖配置读写与 id 解析 <!-- comet-task:86e08129-9d54-4b56-b564-bd3589249c35 -->
- [ ] 4.2 daemon 实现 proxy uplink：周期调用 `GET /devices` 刷新远端设备进注册表（BackendKind::proxy），命令经 `POST /devices/:name/commands|scripts` 同步执行并复用统一输出/错误契约；集成测试用本地起代理服务验证 devices 合并枚举与 tap 路由 <!-- comet-task:72e7a208-7fc1-4201-8542-de631748c1e9 -->
- [ ] 4.3 `agent-mobile-cli pair --proxy`：经 daemon 调代理签发配对码，输出 `agent-mobile://pair?...` URI 与终端二维码；`--proxy --reset` 对应配对重置；冒烟验证输出含 URI 与二维码 <!-- comet-task:5416fda1-424e-4667-b74c-1afdd8999786 -->
- [ ] 4.4 `tests/` 的 `#[path]` 模块树按记忆 tests-path 模式补声明，全量 `cargo test` 通过 <!-- comet-task:fc3c3917-21dd-4620-9ba3-251d5749bc59 -->

## 5. App 连接目标扩展

- [ ] 5.1 App 连接页支持代理服务器地址配置与当前目标类型展示（本机 daemon/代理服务器），切换目标先断开原连接；真机/模拟器手动验证连接代理后状态正确 <!-- comet-task:77356175-7bb1-4e5a-9d06-913b42b584f9 -->
- [ ] 5.2 扫码解析代理配对 URI 直连代理（复用现有 `agent-mobile://pair` 解析）；验证扫码后自动填入代理地址与配对码并完成注册 <!-- comet-task:f53ffadd-4bee-46c3-81a4-b5269b81278f -->

## 6. 文档与发布

- [ ] 6.1 更新 `docs/bridge-protocol.md` 增补公网中继角色与配对模型章节（与 `src/bridge_proto.rs` 逐字段一致） <!-- comet-task:e9a493b2-98c1-45db-a406-b0204dfdd4ce -->
- [ ] 6.2 更新 README/AI_INSTALL：代理服务部署（含反向代理 TLS 示例）、CLI 配置、`pair --proxy` 流程说明 <!-- comet-task:ac442aed-c78d-44b3-bc92-1dbeeead9833 -->
- [ ] 6.3 全链路验收：本机起代理服务 → App（模拟器）绑定 → CLI 经代理执行 snapshot/tap/script，结果与 LAN 模式一致 <!-- comet-task:9306fe6d-f66c-4a90-ac14-44a015987fec -->

```

## docs/openspec/changes/mobile-debug-proxy/.openspec.yaml

- Source: docs/openspec/changes/mobile-debug-proxy/.openspec.yaml
- Lines: 1-2
- SHA256: 5edc88a042fdda2550686e54689bdd6779b13445bcaf782a1af9426634b9b4fb

```md
schema: spec-driven
created: 2026-09-29

```

## docs/openspec/changes/mobile-debug-proxy/specs/agent-app/spec.md

- Source: docs/openspec/changes/mobile-debug-proxy/specs/agent-app/spec.md
- Lines: 1-22
- SHA256: 22b799d4bf9fbcec83696e37b3877cd575fb9dbae446a63519d78dced032ad84

```md
# Spec Delta

## ADDED Requirements

### Requirement: 公网代理连接目标

App SHALL 支持配置公网代理服务器地址作为本机 daemon 之外的第二种连接目标；连接代理服务器时 SHALL 复用同一注册握手、配对码/token 认证与断线自动重连流程；连接页 SHALL 可区分当前连接目标是本机 daemon 还是代理服务器。

#### Scenario: 配置代理地址连接成功

- **WHEN** 用户在 App 中输入代理服务器地址与配对码并发起连接
- **THEN** App 显示已连接状态，代理服务侧出现对应设备注册，连接页标明当前目标为代理服务器

#### Scenario: 代理配对 URI 扫码

- **WHEN** 用户扫描指向代理服务器的配对 URI 二维码
- **THEN** App 解析 URI 自动填入代理地址与配对码并发起连接

#### Scenario: 切换连接目标

- **WHEN** 已连接本机 daemon 的 App 改为配置代理服务器地址并连接
- **THEN** App 断开原连接并以代理服务器为新目标完成注册，不残留双连接

```

## docs/openspec/changes/mobile-debug-proxy/specs/bridge-protocol/spec.md

- Source: docs/openspec/changes/mobile-debug-proxy/specs/bridge-protocol/spec.md
- Lines: 1-26
- SHA256: 23de2dd062d3c0cc45ec58bc2cae1f96bce53fb8715c02e3dda0ff6c22b97d0c

```md
# Spec Delta

## ADDED Requirements

### Requirement: 公网中继传输

协议帧 SHALL 可经公网中继服务转发：设备侧连接角色由「App 连本机 daemon」扩展为「App 连中继服务」，hello/heartbeat/command/script/result 各帧的字段契约保持不变；中继场景下 command/script 的请求标识 SHALL 由命令调用方一侧分配并在同一设备连接上唯一，result 按该标识关联回传。

#### Scenario: 帧契约跨中继不变

- **WHEN** App 经中继服务接入并收到 tap 命令帧
- **THEN** 帧结构与直连 daemon 场景完全一致，App 无需区分对端是 daemon 还是中继服务

#### Scenario: 请求标识端到端关联

- **WHEN** 调用方经中继服务下发携带请求标识的命令
- **THEN** 设备回传的 result 携带同一标识，调用方据此关联唤醒等待方

### Requirement: 中继配对模型

中继场景 SHALL 复用一次性配对码 + 长期 token 的认证模型：配对码由中继服务侧签发与校验，App 的 hello 帧字段不变，不感知配对码签发方是 daemon 还是中继服务。

#### Scenario: 经中继配对

- **WHEN** App 向中继服务提交有效配对码完成 hello
- **THEN** App 收到 hello_ack 与长期 token，后续连接凭 token 认证，流程与直连 daemon 一致

```

## docs/openspec/changes/mobile-debug-proxy/specs/debug-proxy-server/spec.md

- Source: docs/openspec/changes/mobile-debug-proxy/specs/debug-proxy-server/spec.md
- Lines: 1-86
- SHA256: 2621ccc260a9a6e7bc19f30225e180d09e3cb3f860614761fd23be93ad051960

[TRUNCATED]

```md
# Spec Delta

## Purpose

提供部署在公网可达主机上的代理中继服务，使调试 App 与 CLI 不在同一局域网时仍能建立桥接通路：App 出站绑定、CLI 经 HTTP API 间接调试设备。

## ADDED Requirements

### Requirement: 设备出站绑定

代理服务 SHALL 监听 WebSocket 端点接受设备端 App 的出站连接；App 完成注册握手（上报设备名称、Android 版本与能力集）后，服务 SHALL 将该设备纳入其所属 owner 的设备注册表并标记在线。

#### Scenario: App 经公网绑定成功

- **WHEN** App 向代理服务的 WS 端点发起连接并完成注册握手
- **THEN** 服务侧设备注册表中出现该设备且状态为在线，同 owner 的 CLI 可枚举到

#### Scenario: 蜂窝网络下绑定

- **WHEN** App 处于蜂窝网络（无法直连任何 CLI 主机）并连接代理服务
- **THEN** 绑定流程与局域网场景一致，无需设备侧任何端口暴露

### Requirement: 配对与凭证认证

代理服务 SHALL 支持 owner 凭证（长期 token）标识调用方身份；owner 可经认证 API 签发一次性配对码；App 首次注册 SHALL 提交配对码，验证通过后服务下发设备 token，后续连接凭设备 token 认证；配对码错误、失效或凭证缺失时 SHALL 拒绝注册。

#### Scenario: 签发配对码

- **WHEN** CLI 携带有效 owner token 请求签发配对码
- **THEN** 服务返回一次性配对码，该码在首次配对成功或重置后失效

#### Scenario: App 首次配对

- **WHEN** App 提交正确配对码完成注册
- **THEN** 服务下发设备 token，设备进入该 owner 的注册表

#### Scenario: 凭证无效被拒

- **WHEN** App 提交错误配对码或无效设备 token
- **THEN** 服务拒绝注册并断开连接，设备不进入任何注册表

### Requirement: 多 owner 隔离

代理服务 SHALL 按 owner 隔离设备注册表与配对码；一个 owner 的凭证 SHALL NOT 枚举、下发命令或重置另一 owner 的设备。

#### Scenario: 跨 owner 访问被拒

- **WHEN** owner A 的凭证请求操作 owner B 名下的设备
- **THEN** 服务返回认证/未找到错误，不泄露设备存在性以外的信息

### Requirement: 命令中继

代理服务 SHALL 提供认证 HTTP API 接收 CLI 的命令与脚本请求，经 WS 转发至目标设备，并将设备回传的结果或错误按请求标识关联后返回给 CLI；设备离线或等待超时 SHALL 返回明确的结构化错误。

#### Scenario: 命令经中继执行成功

- **WHEN** CLI 对在线设备调用命令 API
- **THEN** 服务将命令经 WS 下发设备，设备结果回传后 API 返回成功结果

#### Scenario: 目标设备离线

- **WHEN** CLI 对离线设备调用命令 API
- **THEN** API 返回设备离线的结构化错误，不产生悬挂等待

#### Scenario: 设备响应超时

- **WHEN** 命令下发后设备在超时窗口内未回传结果
- **THEN** API 返回超时错误，迟到的结果静默丢弃

### Requirement: 设备枚举 API

代理服务 SHALL 提供认证 HTTP API 返回 owner 名下全部已注册设备及其在线状态、能力集与最近活跃时间。

#### Scenario: 枚举含在线与离线设备

- **WHEN** 一台设备在线、另一台已注册但断线，CLI 调用枚举 API
- **THEN** 返回两台设备记录，在线状态可区分

### Requirement: 心跳与离线标记


```

Full source: docs/openspec/changes/mobile-debug-proxy/specs/debug-proxy-server/spec.md

## docs/openspec/changes/mobile-debug-proxy/specs/device-backends/spec.md

- Source: docs/openspec/changes/mobile-debug-proxy/specs/device-backends/spec.md
- Lines: 1-27
- SHA256: 98a7ae0aec66d34d2380314a4a57713a19f357f5152dde25f13b645f43ff99fe

```md
# Spec Delta

## ADDED Requirements

### Requirement: 代理设备接入

CLI SHALL 支持配置代理服务地址与 owner 凭证；配置后设备枚举 SHALL 合并展示经代理服务注册的远端设备（来源可区分），控制命令 SHALL 经统一后端接口路由到代理通路执行，命令参数与输出结构与其他后端一致；代理服务不可达或凭证无效时 SHALL 返回明确的结构化错误。

#### Scenario: 枚举合并远端设备

- **WHEN** 本地有一台 ADB 设备在线，代理服务上有一台远端设备在线，用户执行设备枚举
- **THEN** 输出同时包含两台设备，远端设备的来源与本地设备可区分

#### Scenario: 经代理执行控制命令

- **WHEN** 用户对代理来源的在线设备执行点击命令
- **THEN** 命令经代理服务中继到设备执行，CLI 输出与其他后端一致的 `ok: true` 结构

#### Scenario: 代理凭证无效

- **WHEN** 配置的 owner 凭证无效或已失效，用户执行设备枚举或控制命令
- **THEN** 命令失败并输出指明代理认证失败的结构化错误

#### Scenario: 代理设备参与目标选择

- **WHEN** 本地设备与代理设备同时在线且用户执行控制命令未指定目标
- **THEN** 返回列出全部候选设备（含代理设备）的歧义错误，与现有多设备契约一致

```
