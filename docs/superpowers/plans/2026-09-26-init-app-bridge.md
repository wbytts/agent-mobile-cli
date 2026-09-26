---
change: init-app-bridge
design-doc: docs/openspec/changes/init-app-bridge/design.md
base-ref: 8749d16f0ee0b4ad5d55b93d014e5c0f5d39b1c5
---

# 实施计划：init-app-bridge

<!-- comet-task-authority: docs/openspec/changes/init-app-bridge/tasks.md -->

执行策略：autonomous + tdd + review standard。设计决策见 design.md（1-15），行为契约见 specs/（bridge-protocol/agent-app/app-backend）。Kotlin/UI 实现由子代理交付代码与编译验证，MuMu/真机交互验收由主会话执行。

环境事实：Android SDK/NDK 27.1 + Java 17 就绪；tauri CLI 需 `cargo install tauri-cli --locked`；Rust Android targets 需 `rustup target add aarch64-linux-android`；MuMu 127.0.0.1:5555、荣耀真机 192.168.2.5:5555。

## 组 1：Tauri 工程与多页 UI（独立区域 app/，与组 2 并行）

- 任务 1.1 `567a37b2-c2d7-4a66-afb3-70511fe05d2f` <!-- comet-task-ref:567a37b2-c2d7-4a66-afb3-70511fe05d2f -->：Tauri 2 Android 工程骨架（app/ 目录、Gradle/NDK/Rust targets 配置、版本锁定记录），debug APK 构建成功。
  - 验收：`cd app && cargo tauri android build --debug` 产出 APK；主会话 adb install 到 MuMu 并可启动。
- 任务 1.2 `2e4b6717-d3cf-4794-8296-f4441b6b7291` <!-- comet-task-ref:2e4b6717-d3cf-4794-8296-f4441b6b7291 -->：多页诊断 UI（连接页/能力自检页/日志页），纯前端渲染与状态切换。
  - 验收：三页渲染正常；主会话 MuMu 截图核对。

## 组 2：daemon WS 协议与配对（CLI 仓库 src/，与组 1 并行）

- 任务 2.1 `02e6b5e2-7068-4e61-a243-996aae31e593` <!-- comet-task-ref:02e6b5e2-7068-4e61-a243-996aae31e593 -->：daemon WS 协议完整实现（hello 注册/heartbeat/command 分发/result 回传 + 设备注册表），协议消息模块供 App path 引用。
  - RED：模拟 WS client 集成测试失败；GREEN：握手注册后设备出现在枚举。
- 任务 2.2 `ce233ce2-7f38-4e65-b248-b6edfe45d979` <!-- comet-task-ref:ce233ce2-7f38-4e65-b248-b6edfe45d979 -->：协议 schema 文档（docs/bridge-protocol.md，随代码同步）。
  - 验收：文档与实现逐字段一致（审查核对）。
- 任务 2.3 `7cb6921e-aba4-4f43-b531-469b150c5e6b` <!-- comet-task-ref:7cb6921e-aba4-4f43-b531-469b150c5e6b -->：配对认证（一次性配对码生成、token 签发/校验、`agent-mobile-cli pair` 显示配对码/候选 IP/终端二维码、`--reset`）。
  - RED：错误配对码被拒/token 重连/reset 失效三个测试失败；GREEN：全过。

## 组 3：Kotlin 原生能力（依赖组 1 骨架）

- 任务 3.1 `545bd128-daa9-419b-b1f5-dcf22cc03965` <!-- comet-task-ref:545bd128-daa9-419b-b1f5-dcf22cc03965 -->：AccessibilityService + Tauri 插件绑定（tap/swipe/uiTree）。
  - 验收：主会话 MuMu 开启无障碍后经插件点击设置项生效。
- 任务 3.2 `40e47a10-0bd3-4527-a23c-052347446c7f` <!-- comet-task-ref:40e47a10-0bd3-4527-a23c-052347446c7f -->：takeScreenshot 截图能力经插件回传非空 PNG。
- 任务 3.3 `ba80be11-91c1-434f-b3ed-1a43a25e9e46` <!-- comet-task-ref:ba80be11-91c1-434f-b3ed-1a43a25e9e46 -->：QuickJS 沙盒（Rust core rquickjs）绑定 `mobile.*` API 桥到 Kotlin。
  - RED：cargo 单测（脚本调 mobile.tap 产生插件调用、沙盒无法访问文件/网络）失败；GREEN：通过。
- 任务 3.4 `2a32ec5a-deb6-44e6-b589-b8cc6c589273` <!-- comet-task-ref:2a32ec5a-deb6-44e6-b589-b8cc6c589273 -->：无障碍权限状态检测与开启引导。
  - 验收：主会话 MuMu 验证未授权引导、授权后状态更新。

## 组 4：App WS 客户端与配对流程（依赖组 2 协议 + 组 1 骨架 + 组 3 插件接口）

- 任务 4.1 `a4554601-2865-4c3c-b2f5-c9403c9b061b` <!-- comet-task-ref:a4554601-2865-4c3c-b2f5-c9403c9b061b -->：前台服务保活 WS 客户端（连接/注册/心跳/自动重连）。
  - 验收：主会话杀 App 重开，daemon 侧设备先离线后恢复。
- 任务 4.2 `8a12453e-410d-40ae-81f1-9728dec0ade8` <!-- comet-task-ref:8a12453e-410d-40ae-81f1-9728dec0ade8 -->：命令处理循环（command/script → 插件/沙盒 → result 回传）。
  - RED：Rust core 单测（成功/失败两类回传）；GREEN：通过。
- 任务 4.3 `22a11751-6d93-4ed3-b560-514e1cc5f995` <!-- comet-task-ref:22a11751-6d93-4ed3-b560-514e1cc5f995 -->：配对与扫码流程（配对码输入、token 保存与自动重连、扫码解析 `agent-mobile://pair` URI）。
  - 验收：主会话 MuMu 手动配对 + 真机扫码配对均成功，杀 App 重连免配对。

## 组 5：CLI app-bridge 后端（依赖组 2 注册表）

- 任务 5.1 `ec4dc356-aefc-4499-8ce7-230b7a487b3d` <!-- comet-task-ref:ec4dc356-aefc-4499-8ce7-230b7a487b3d -->：Backend trait 实现（snapshot/tap/swipe/input/key/screenshot/apps/launch 经 WS 路由），输出结构与 ADB 后端一致。
- 任务 5.2 `d7b70f80-52cc-4ebf-8672-538918c5f211` <!-- comet-task-ref:d7b70f80-52cc-4ebf-8672-538918c5f211 -->：能力降级（stop/logcat/shell → NOT_SUPPORTED 结构化错误）。
- 任务 5.3 `41c1207f-f863-4d14-aea2-7f6f6dd9b8cf` <!-- comet-task-ref:41c1207f-f863-4d14-aea2-7f6f6dd9b8cf -->：devices 合并枚举（adb + bridge 类型标识）。
- 任务 5.4 `ea47e93b-5534-41a6-9fbf-e4c5f5bb0766` <!-- comet-task-ref:ea47e93b-5534-41a6-9fbf-e4c5f5bb0766 -->：CLI script 命令（文件/stdin 读 JS 经桥接下发，输出脚本返回值或异常）。
  - 三个任务验收：单测 + 主会话双后端在线冒烟。

## 组 6：端到端验收（主会话）

- 任务 6.1 `09ded594-1ab2-47ba-ac9c-bad71244d5ac` <!-- comet-task-ref:09ded594-1ab2-47ba-ac9c-bad71244d5ac -->：MuMu 全链路（安装→授权→手动配对→devices→snapshot→`mobile.tap` 脚本→screenshot→断线重连→`pair --reset` 失效）+ 真机扫码配对，逐项对照 specs 场景。

## 派发波次

- Wave 1（并行）：组 1（AppImpl：app/ 全部）、组 2（DaemonWsImpl：src/daemon/ + src/cli.rs pair 命令 + src/exec.rs）
- Wave 2（并行）：组 3（KotlinImpl：app Kotlin 插件 + 沙盒绑定）、组 5（BridgeBackendImpl：src/backend/）
- Wave 3：组 4（AppWsImpl：app Rust core + UI 接线）
- Wave 4：主会话组 6 验收；期间穿插 release 构建与安装
