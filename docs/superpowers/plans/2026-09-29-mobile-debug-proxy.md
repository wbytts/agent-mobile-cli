---
change: mobile-debug-proxy
design-doc: docs/openspec/changes/mobile-debug-proxy/design.md
base-ref: 699378fe5f8318909d32c660fe1a5aebc4e32d10
archived-with: 2026-10-01-mobile-debug-proxy
---

# 实施计划：mobile-debug-proxy

<!-- comet-task-authority: docs/openspec/changes/mobile-debug-proxy/tasks.md -->

依据 design.md 决策 1-13 实施。TDD 模式：每个实现任务先 RED（确认失败原因是待实现行为）后 GREEN。验收以各任务的测试/命令为准。

## 组 1：代理服务骨架与持久化

- <!-- comet-task-ref:fe3d215a-6401-489f-9996-1a37c3669f29 --> 1.1 crate 骨架：独立 `mobile-debug-proxy-server/`（lib+bin），依赖 axum 0.7/tokio/tokio-tungstenite 0.24/serde/parking_lot/rand；`#[path]` 引用 `../src/bridge_proto.rs`。验收：`cargo build --manifest-path mobile-debug-proxy-server/Cargo.toml` 通过。约束：bridge_proto 若引用 crate:: 内其他模块需一并处理（按其当前 import 实际定）。
- <!-- comet-task-ref:8dcbe22f-b0d9-4913-94ae-7549c2de7125 --> 1.2 store 模块：JSON 数据文件（owners/devices/tokens/pairing），parking_lot，首启生成 owner token 并打印，--owner-token 覆盖。RED：加载/保存/首次生成单测先失败。
- <!-- comet-task-ref:6a9b97ac-4318-49ef-83d1-2a8bd4f8a1f6 --> 1.3 auth 模块 + /healthz：Bearer 中间件校验 owner token，无效返回 401 结构化 JSON。RED：认证通过/拒绝单测。

## 组 2：设备 WS 绑定与配对

- <!-- comet-task-ref:b4c53474-4baa-4ed9-aec9-051e798da618 --> 2.1 ws 模块：/ws/device，hello 首帧 10s 超时，配对码/token 双路径认证，hello_ack 下发设备 token，注册表按 owner 隔离。RED：三路径（配对码/token/拒绝）测试。
- <!-- comet-task-ref:2b6433fe-9de7-4065-8763-f23b20620cf7 --> 2.2 心跳与离线：heartbeat/pong，30s 超时标离线，断开保留记录，重连凭 token 恢复。RED：离线标记与重连恢复测试。
- <!-- comet-task-ref:47454a26-f808-498f-b6a2-bf2f95168ea3 --> 2.3 pairing API：POST /pairing-codes、POST /pairing-reset（吊销 owner 全部设备 token + 断开连接）。RED：签发/一次性失效/重置吊销测试。

## 组 3：命令中继与设备 API

- <!-- comet-task-ref:7152029c-2c01-4a1d-81d1-3b1326751365 --> 3.1 relay 模块 + POST /devices/:name/commands|scripts：AtomicU64 id，per-device mpsc 串行协程，30s 超时，离线立即报错，迟到 result 丢弃。RED：成功/离线/超时三路径。
- <!-- comet-task-ref:ce43ad3a-a4f7-40cb-a053-c7fd7ebe146a --> 3.2 GET /devices 枚举 + owner 隔离（404 语义）。RED：枚举与隔离测试。
- <!-- comet-task-ref:e83e8538-77d4-4e91-a437-373a8a295ab0 --> 3.3 端到端集成：进程内起服务（随机端口），tokio-tungstenite 模拟 App，配对→tap→result 全链路。

## 组 4：CLI daemon uplink

- <!-- comet-task-ref:86e08129-9d54-4b56-b564-bd3589249c35 --> 4.1 config.rs 增 [proxy] 段（url/token），BackendKind::Proxy，`proxy:<name>` id 解析。RED：配置读写与 id 解析单测。
- <!-- comet-task-ref:72e7a208-7fc1-4201-8542-de631748c1e9 --> 4.2 daemon uplink：ureq 同步 HTTP（决策 11），5s 轮询 GET /devices 注入注册表，命令 POST 同步执行，错误映射（401/404/超时）。集成：进程内起 proxy lib 服务验证合并枚举与 tap 路由。注意 spawn_blocking 约束（记忆 daemon-spawn-blocking-block-in-place）。
- <!-- comet-task-ref:5416fda1-424e-4667-b74c-1afdd8999786 --> 4.3 pair --proxy：经 daemon 调代理签发配对码，输出 agent-mobile://pair URI + 终端二维码；--proxy --reset。冒烟验证输出。
- <!-- comet-task-ref:fc3c3917-21dd-4620-9ba3-251d5749bc59 --> 4.4 tests/ #[path] 模块树补声明（记忆 tests-path），`cargo test` 全绿。

## 组 5：App 连接目标

- <!-- comet-task-ref:77356175-7bb1-4e5a-9d06-913b42b584f9 --> 5.1 App 连接页：地址输入支持 host:port 与 ws(s)://host:port 两种形式，UI 显示目标类型，切换先断原连接。MuMu 手动验证。
- <!-- comet-task-ref:f53ffadd-4bee-46c3-81a4-b5269b81278f --> 5.2 扫码解析代理 URI 直连代理（复用现有解析）。MuMu 验证。

## 组 6：文档与验收

- <!-- comet-task-ref:e9a493b2-98c1-45db-a406-b0204dfdd4ce --> 6.1 docs/bridge-protocol.md 增补公网中继章节（与 bridge_proto.rs 逐字段一致）。
- <!-- comet-task-ref:ac442aed-c78d-44b3-bc92-1dbeeead9833 --> 6.2 README/AI_INSTALL：部署、反向代理 TLS 示例、CLI 配置、pair --proxy。
- <!-- comet-task-ref:9306fe6d-f66c-4a90-ac14-44a015987fec --> 6.3 全链路验收：本机起代理 → 模拟器 App 绑定 → CLI 经代理 snapshot/tap/script，对照 LAN 模式结果。

## 审查安排（review_mode: standard）

组 2/3（WS/relay 并发与安全敏感）与组 4（daemon uplink，触及现有核心路径）完成后委派独立 reviewer 审查；组 1/5/6 低风险随验收。CRITICAL/IMPORTANT 必须解决。
