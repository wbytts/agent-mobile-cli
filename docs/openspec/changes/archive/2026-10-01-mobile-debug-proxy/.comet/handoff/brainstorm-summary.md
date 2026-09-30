# Brainstorm Summary

- Change: mobile-debug-proxy
- Date: 2026-09-29

## 确认的技术方案

（用户已于 2026-09-29 设计确认点正式确认以下方案）

1. **proxy crate 结构**：`mobile-debug-proxy-server/` = lib + 薄 bin。lib 内模块：`store`（JSON 持久化）、`auth`（Bearer 中间件）、`ws`（/ws/device 握手与消息循环）、`http`（REST 路由）、`relay`（per-device 连接句柄 + pending id→oneshot 映射）。lib/bin 分离使 CLI 集成测试可进程内起服务，避免起二进制进程。
2. **命令中继 id**：服务侧全局 AtomicU64 分配 `cmd-<n>`；每设备命令经 per-device 串行队列（mpsc 工作协程）下发，与 daemon 串行语义一致。
3. **多 owner 模型**：数据文件支持多 owner 记录；首启自动生成一个 owner token；追加 owner 仅通过重复 `--owner-token` 启动参数或手工编辑数据文件。v1 不做 owner 管理 API。
4. **daemon uplink 传输**：纯 HTTP——每 5s（可配）`GET /devices` 轮询刷新注册表；命令时 `POST` 同步等待。新增 `ureq` 依赖（同步、纯 Rust、rustls）：Backend trait 在 `spawn_blocking` 内执行，项目记忆明确此处不可用 tokio 阻塞原语，同步 HTTP client 是唯一安全选项；reqwest blocking 依赖树过重，弃。
5. **uplink 故障语义**：轮询失败时 proxy 来源设备标记 offline（记录保留），恢复后自动回在线；401 → 结构化代理认证错误；404/410 → 设备离线；超时 → TIMEOUT。
6. **App 连接目标**：地址输入升级为支持 `host:port`（LAN 直连）与完整 `ws://|wss://host:port`（代理）两种形式；UI 显示当前目标类型；扫码 URI 格式不变天然兼容。

## 关键取舍与风险

- ureq 新增依赖 vs reqwest blocking：选依赖树小的 ureq；风险是 API 能力有限，本场景只需 JSON POST/GET + timeout，足够。
- 轮询枚举有秒级延迟；命令路径实时，不受影响。
- owner token 数据文件明文存储：与 daemon tokens.json 现状一致，文档强调文件权限与 TLS。
- 公网暴露面：一键 reset 吊销；文档强制反向代理 TLS。

## 测试策略

- proxy lib：单元测试覆盖 store/auth/relay 各路径（配对三路径、离线标记、重置吊销、owner 隔离、超时、迟到 result 丢弃）。
- proxy 集成测试：进程内起 axum 服务（随机端口），tokio-tungstenite 模拟 App，HTTP 客户端全链路验证 tap 往返。
- CLI 集成测试：复用 tests/ #[path] 模块树模式（按项目记忆补声明），进程内起 proxy 服务验证 devices 合并枚举与命令路由。
- App：MuMu 模拟器手动验收（连接代理、扫码、切换目标）。

## Spec Patch

无——Open 阶段 delta spec 验收场景已覆盖核心成功与边界场景，brainstorming 未发现缺口。
