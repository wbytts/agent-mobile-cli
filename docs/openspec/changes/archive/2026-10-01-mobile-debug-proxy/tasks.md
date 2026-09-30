# Tasks

## 1. 代理服务骨架与持久化

- [x] 1.1 创建 `mobile-debug-proxy-server/` 独立 crate（axum 0.7、tokio、tokio-tungstenite、serde、parking_lot、rand、base64），`#[path]` 引用 `../src/bridge_proto.rs`；`cargo build` 通过 <!-- comet-task:fe3d215a-6401-489f-9996-1a37c3669f29 -->
- [x] 1.2 实现 JSON 数据文件持久化（owners/设备记录/token/配对码状态，parking_lot 保护；首次启动生成 64 hex owner token 打印 stdout 并写盘，支持 `--owner-token` 覆盖）；单元测试覆盖加载/保存/首次生成 <!-- comet-task:8dcbe22f-b0d9-4913-94ae-7549c2de7125 -->
- [x] 1.3 实现 `GET /healthz` 与 owner token Bearer 认证中间件（无效凭证返回 401 结构化错误）；单元测试覆盖认证通过/拒绝 <!-- comet-task:6a9b97ac-4318-49ef-83d1-2a8bd4f8a1f6 -->

## 2. 设备 WS 绑定与配对

- [x] 2.1 实现 `WS /ws/device`：hello 握手（配对码或设备 token 认证，10s 首帧超时）、hello_ack、设备注册表按 owner 隔离写入；单元测试覆盖配对码/token/拒绝三路径 <!-- comet-task:b4c53474-4baa-4ed9-aec9-051e798da618 -->
- [x] 2.2 实现 heartbeat/pong 与离线标记（30s 无消息标离线、断开保留记录、重连凭 token 恢复在线）；单元测试覆盖断线标记与重连恢复 <!-- comet-task:2b6433fe-9de7-4065-8763-f23b20620cf7 -->
- [x] 2.3 实现 `POST /pairing-codes`（签发 6 位一次性配对码）与 `POST /pairing-reset`（重置配对码、吊销 owner 全部设备 token、断开已连接设备）；单元测试覆盖签发/一次性失效/重置吊销 <!-- comet-task:47454a26-f808-498f-b6a2-bf2f95168ea3 -->

## 3. 命令中继与设备 API

- [x] 3.1 实现 command/script 中继：`POST /devices/:name/commands`、`POST /devices/:name/scripts` 挂起等待 WS result（id 由服务按连接分配 `cmd-<n>`，30s 超时，设备离线立即报错，迟到 result 静默丢弃）；单元测试覆盖成功/离线/超时三路径 <!-- comet-task:7152029c-2c01-4a1d-81d1-3b1326751365 -->
- [x] 3.2 实现 `GET /devices` 枚举（在线状态、能力集、last_seen）与跨 owner 访问隔离（返回未找到，不泄露）；单元测试覆盖枚举与隔离 <!-- comet-task:ce43ad3a-a4f7-40cb-a053-c7fd7ebe146a -->
- [x] 3.3 端到端集成测试：内存 WS 客户端模拟 App 绑定 → 配对 → HTTP 下发 tap → 回传 result → 断言 API 返回成功结构 <!-- comet-task:e83e8538-77d4-4e91-a437-373a8a295ab0 -->

## 4. CLI daemon uplink 接入

- [x] 4.1 `src/config.rs` 增加代理配置（地址、owner token），`src/backend/mod.rs` 增加 `BackendKind::Proxy` 与 `proxy:<name>` 设备 id 解析；单元测试覆盖配置读写与 id 解析 <!-- comet-task:86e08129-9d54-4b56-b564-bd3589249c35 -->
- [x] 4.2 daemon 实现 proxy uplink：周期调用 `GET /devices` 刷新远端设备进注册表（BackendKind::proxy），命令经 `POST /devices/:name/commands|scripts` 同步执行并复用统一输出/错误契约；集成测试用本地起代理服务验证 devices 合并枚举与 tap 路由 <!-- comet-task:72e7a208-7fc1-4201-8542-de631748c1e9 -->
- [x] 4.3 `agent-mobile-cli pair --proxy`：经 daemon 调代理签发配对码，输出 `agent-mobile://pair?...` URI 与终端二维码；`--proxy --reset` 对应配对重置；冒烟验证输出含 URI 与二维码 <!-- comet-task:5416fda1-424e-4667-b74c-1afdd8999786 -->
- [x] 4.4 `tests/` 的 `#[path]` 模块树按记忆 tests-path 模式补声明，全量 `cargo test` 通过 <!-- comet-task:fc3c3917-21dd-4620-9ba3-251d5749bc59 -->

## 5. App 连接目标扩展

- [x] 5.1 App 连接页支持代理服务器地址配置与当前目标类型展示（本机 daemon/代理服务器），切换目标先断开原连接；真机/模拟器手动验证连接代理后状态正确 <!-- comet-task:77356175-7bb1-4e5a-9d06-913b42b584f9 -->
- [x] 5.2 扫码解析代理配对 URI 直连代理（复用现有 `agent-mobile://pair` 解析）；验证扫码后自动填入代理地址与配对码并完成注册 <!-- comet-task:f53ffadd-4bee-46c3-81a4-b5269b81278f -->

## 6. 文档与发布

- [x] 6.1 更新 `docs/bridge-protocol.md` 增补公网中继角色与配对模型章节（与 `src/bridge_proto.rs` 逐字段一致） <!-- comet-task:e9a493b2-98c1-45db-a406-b0204dfdd4ce -->
- [x] 6.2 更新 README/AI_INSTALL：代理服务部署（含反向代理 TLS 示例）、CLI 配置、`pair --proxy` 流程说明 <!-- comet-task:ac442aed-c78d-44b3-bc92-1dbeeead9833 -->
- [x] 6.3 全链路验收：本机起代理服务 → App（模拟器）绑定 → CLI 经代理执行 snapshot/tap/script，结果与 LAN 模式一致 <!-- comet-task:9306fe6d-f66c-4a90-ac14-44a015987fec -->
