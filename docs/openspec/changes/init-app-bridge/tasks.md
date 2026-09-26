# Tasks

## 1. Tauri 工程与构建链

- [x] 1.1 创建 Tauri 2 Android 工程骨架（含 Gradle/NDK/Rust targets 配置），验证 debug APK 构建成功并在 MuMu 安装启动 <!-- comet-task:567a37b2-c2d7-4a66-afb3-70511fe05d2f -->
- [x] 1.2 实现 App 多页诊断 UI（连接页：daemon 地址/配对码输入/扫码入口/连接状态；能力自检页：无障碍权限状态与逐项能力自测；日志页：命令与连接事件），验证三页渲染与状态切换 <!-- comet-task:2e4b6717-d3cf-4794-8296-f4441b6b7291 -->

## 2. 桥接协议

- [x] 2.1 在 daemon 侧实现 WS 协议（hello 注册/heartbeat/command 分发/result 回传），验证模拟 WS 客户端握手注册后设备出现在枚举中 <!-- comet-task:02e6b5e2-7068-4e61-a243-996aae31e593 -->
- [x] 2.2 编写协议消息 schema 文档（随代码同步维护），验证文档中每种消息与实现字段一致 <!-- comet-task:ce233ce2-7f38-4e65-b248-b6edfe45d979 -->
- [x] 2.3 实现 daemon 侧配对认证（启动生成一次性配对码、验证配对并下发/校验长期 token、`agent-mobile-cli pair` 显示配对码/候选 IP/终端配对二维码与 `--reset` 重置），验证错误配对码被拒、配对后 token 重连成功、重置后旧 token 失效 <!-- comet-task:7cb6921e-aba4-4f43-b531-469b150c5e6b -->

## 3. Kotlin 原生能力

- [x] 3.1 实现 AccessibilityService 与 Tauri 插件绑定（点击/滑动/UI 树读取），验证 MuMu 上经插件点击设置项生效 <!-- comet-task:545bd128-daa9-419b-b1f5-dcf22cc03965 -->
- [x] 3.2 实现截图能力（takeScreenshot）并经插件回传，验证返回非空 PNG 数据 <!-- comet-task:40e47a10-0bd3-4527-a23c-052347446c7f -->
- [x] 3.3 集成 QuickJS 沙盒插件并绑定 `mobile.*` API（tap/swipe/uiTree/screenshot），验证脚本执行返回结果且无法访问沙盒外资源 <!-- comet-task:ba80be11-91c1-434f-b3ed-1a43a25e9e46 -->
- [x] 3.4 实现无障碍权限状态检测与开启引导（跳转系统设置），验证未授权时引导展示、授权后状态更新 <!-- comet-task:2a32ec5a-deb6-44e6-b589-b8cc6c589273 -->

## 4. App 内 WS 客户端

- [ ] 4.1 实现前台服务保活的 WS 客户端（连接/注册/心跳/自动重连），验证杀掉 App 重开后 daemon 侧设备先离线后恢复在线 <!-- comet-task:a4554601-2865-4c3c-b2f5-c9403c9b061b -->
- [ ] 4.2 实现命令处理循环（接收 command/script → 分发到插件/沙盒 → 回传 result），验证命令成功与失败两类回传 <!-- comet-task:8a12453e-410d-40ae-81f1-9728dec0ade8 -->
- [ ] 4.3 实现 App 配对与扫码流程（配对码输入提交、token 本地保存与自动凭 token 重连、相机扫码解析 `agent-mobile://pair` URI 自动填入），验证手动与扫码两条路径均配对成功且杀 App 重连免配对 <!-- comet-task:22a11751-6d93-4ed3-b560-514e1cc5f995 -->

## 5. CLI 桥接后端

- [x] 5.1 实现 `app-bridge` 后端的统一接口（snapshot/tap/swipe/input/screenshot 经 WS 路由），验证对桥接设备执行命令输出结构与 ADB 后端一致 <!-- comet-task:ec4dc356-aefc-4499-8ce7-230b7a487b3d -->
- [x] 5.2 实现能力集校验与降级错误，验证对桥接设备执行 shell 返回「不支持」结构化错误 <!-- comet-task:d7b70f80-52cc-4ebf-8672-538918c5f211 -->
- [x] 5.3 实现 devices 合并枚举（adb + 桥接设备类型标识），验证两类设备同时在线时均可列出并区分 <!-- comet-task:41c1207f-f863-4d14-aea2-7f6f6dd9b8cf -->
- [x] 5.4 实现 CLI script 命令（`agent-mobile-cli script <file.js|-> --device <id>`：文件或 stdin 读 JS 源码，经桥接下发 script 消息，输出脚本返回值或异常），验证对桥接设备执行 `mobile.tap` 脚本输出与协议一致 <!-- comet-task:ea47e93b-5534-41a6-9fbf-e4c5f5bb0766 -->

## 6. 端到端验收

- [ ] 6.1 在 MuMu 执行全链路验收：安装 App → 无障碍授权 → 手动与扫码配对 → devices 可见 → snapshot → 脚本 `mobile.tap` → screenshot → 断线重连恢复 → `pair --reset` 后旧 token 失效，逐项对照 specs 场景通过 <!-- comet-task:09ded594-1ab2-47ba-ac9c-bad71244d5ac -->
