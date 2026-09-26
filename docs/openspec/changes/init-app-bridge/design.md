# Design

## Context

依赖 change `init-adb-core` 交付的 daemon、桥接 WS 监听端口与后端抽象层。动机与范围见 proposal.md。测试环境：MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a）。用户已确认：代理模式 = 脚本沙盒 + 设备操作 API；平台 Android 优先、协议预留多后端。

## Goals / Non-Goals

**Goals:**

- 定稿桥接 WS 协议并两端实现（daemon、App）
- Tauri 2 Android App 在 MuMu 上可安装、可连接、可执行命令与脚本
- 桥接设备接入 CLI 统一设备模型，控制命令契约与 ADB 后端一致

**Non-Goals:**

- iOS 端、root/注入式能力、脚本市场、App 加固
- ADB 后端已有命令的任何修改
- 无线 adb 与桥接的自动切换/择优策略

## Decisions

1. **App 采用 Tauri 2 + Kotlin 原生插件**：UI 与连接管理用 Tauri 2（WebView 前端 + Rust 核心），设备能力封装为 Tauri 插件调用 Kotlin 实现。备选纯 Kotlin 原生 App 放弃：用户明确建议 Tauri 2，且 UI/配置界面开发效率高。
2. **脚本引擎选 QuickJS**（JNI 集成）：包体积小、启动快、ES2020 支持足够。备选 V8 体积过大，Rhino 过旧。沙盒默认无文件/网络权限，仅注入 `mobile.*` API 绑定。
3. **桥接协议为 JSON 文本帧**：`hello`（设备信息+能力集）、`heartbeat`、`command`（id/method/params）、`result`（id/ok/result|error）、`script`/`script-result`。理由：与 CLI HTTP API 风格一致、调试友好；二进制图像数据（截图）以 base64 内联于 result。备选 protobuf 放弃：首版调试图优先级高于带宽。
4. **截图走 AccessibilityService.takeScreenshot**（API 30+，MuMu Android 12 满足）；失败时不自动回落 MediaProjection（需要用户授权弹窗），直接返回错误，留待后续版本。
5. **UI 树序列化复用 adb 模式同一简化模型**：AccessibilityService `rootInActiveWindow` 递归序列化为与 uiautomator dump 相同的节点结构（文本/类型/坐标区域），CLI 侧复用同一 `@eN` 引用分配逻辑，保证两后端输出结构一致。
6. **能力集协商**：`hello` 消息携带能力列表（tap/swipe/uiTree/screenshot/script），CLI 路由前按能力集校验，缺失即返回不支持错误（对应 app-backend 能力降级要求）。
7. **App 内连接保活**：前台服务（foreground service）维持 WS 连接与心跳，避免系统回收导致频繁掉线。

## Risks / Trade-offs

- Tauri 2 mobile 构建链（Gradle + NDK + Rust targets）复杂度高 → 任务组 1 先行打通「构建出可安装 APK」再叠加能力；固定 toolchain 版本并记录
- 无障碍服务被系统或厂商回收导致命令失败 → App 侧检测服务状态并在 UI 提示；心跳附带权限状态，daemon 可据此标记设备降级
- MuMu arm64 对 QuickJS 原生库兼容性 → 先编译验证 arm64-v8a so 加载，再集成
- base64 截图消息体积大 → 首版接受；后续可升级二进制帧
- 前台服务带来常驻通知 → Android 平台要求，不可去除，UI 中说明用途

## Migration Plan

无历史包袱；依赖 `init-adb-core` 归档后产生的 specs 作为基线。
