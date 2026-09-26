# Tasks

## 1. Tauri 工程与构建链

- [ ] 1.1 创建 Tauri 2 Android 工程骨架（含 Gradle/NDK/Rust targets 配置），验证 debug APK 构建成功并在 MuMu 安装启动
- [ ] 1.2 实现 App 基础 UI（daemon 地址输入、连接状态显示、日志视图），验证页面上输入与状态渲染正常

## 2. 桥接协议

- [ ] 2.1 在 daemon 侧实现 WS 协议（hello 注册/heartbeat/command 分发/result 回传），验证模拟 WS 客户端握手注册后设备出现在枚举中
- [ ] 2.2 编写协议消息 schema 文档（随代码同步维护），验证文档中每种消息与实现字段一致

## 3. Kotlin 原生能力

- [ ] 3.1 实现 AccessibilityService 与 Tauri 插件绑定（点击/滑动/UI 树读取），验证 MuMu 上经插件点击设置项生效
- [ ] 3.2 实现截图能力（takeScreenshot）并经插件回传，验证返回非空 PNG 数据
- [ ] 3.3 集成 QuickJS 沙盒插件并绑定 `mobile.*` API（tap/swipe/uiTree/screenshot），验证脚本执行返回结果且无法访问沙盒外资源
- [ ] 3.4 实现无障碍权限状态检测与开启引导（跳转系统设置），验证未授权时引导展示、授权后状态更新

## 4. App 内 WS 客户端

- [ ] 4.1 实现前台服务保活的 WS 客户端（连接/注册/心跳/自动重连），验证杀掉 App 重开后 daemon 侧设备先离线后恢复在线
- [ ] 4.2 实现命令处理循环（接收 command/script → 分发到插件/沙盒 → 回传 result），验证命令成功与失败两类回传

## 5. CLI 桥接后端

- [ ] 5.1 实现 `app-bridge` 后端的统一接口（snapshot/tap/swipe/input/screenshot 经 WS 路由），验证对桥接设备执行命令输出结构与 ADB 后端一致
- [ ] 5.2 实现能力集校验与降级错误，验证对桥接设备执行 shell 返回「不支持」结构化错误
- [ ] 5.3 实现 devices 合并枚举（adb + 桥接设备类型标识），验证两类设备同时在线时均可列出并区分

## 6. 端到端验收

- [ ] 6.1 在 MuMu 执行全链路验收：安装 App → 无障碍授权 → 配置连接 → devices 可见 → snapshot → 脚本 `mobile.tap` → screenshot → 断线重连恢复，逐项对照 specs 场景通过
