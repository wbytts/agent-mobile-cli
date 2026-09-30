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
