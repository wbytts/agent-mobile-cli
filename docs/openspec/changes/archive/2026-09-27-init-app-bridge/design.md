---
comet_change: init-app-bridge
role: technical-design
canonical_spec: openspec
archived-with: 2026-09-27-init-app-bridge
status: final
---

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
8. **配对认证模型（双通道）**：桥接监听启动时 daemon 生成 6 位一次性配对码（首次使用或 daemon 重启后失效），`agent-mobile-cli pair` 显示配对码与候选局域网 IP 并在终端输出配对二维码（unicode block），二维码内容 URI `agent-mobile://pair?host=<ip>&port=<port>&code=<code>`；App 可手动输入地址+配对码，或扫码（相机权限 + ML Kit/zxing）解析 URI 自动填入并发起配对。配对码验证通过后 daemon 下发长期 token（32 字节随机 hex，存 daemon 配置目录与 App 本地），后续 hello 凭 token 认证；`agent-mobile-cli pair --reset` 重新生成配对码并使全部已发 token 失效。理由：局域网明文 WS 的最低信任门槛，一次性配对码收窄泄露面，token 可撤销；扫码消除真机手动输入成本（MuMu 模拟器走手动通道）。备选无认证放弃：同网段任意主机可控制设备；备选 mDNS 自动发现放弃：多一套发现协议与平台权限，首版手动/扫码已够用。
9. **App UI 为多页诊断型**：连接页（daemon 地址/配对码输入/连接状态）、能力自检页（无障碍权限状态 + 逐项能力自测按钮）、日志页（最近命令与连接事件）。用户已确认（2026-09-26 晚对账）；替代此前单页基础 UI 的设想。
10. **协议层全在 App 的 Rust core**：WS 客户端（tokio-tungstenite）、协议消息（serde）、QuickJS 沙盒（rquickjs bundled）均在 Rust core；Kotlin 仅承担 AccessibilityService 能力桥、前台服务保活、扫码（ML Kit/zxing）。`mobile.*` 脚本调用经 Tauri 插件命令桥到 Kotlin 执行后回传。理由：协议/沙盒逻辑与 CLI 同栈（tokio/serde）可单测可复用，Kotlin 面最小化。
11. **协议类型单一来源**：CLI 侧协议模块（serde 消息类型）被 App Rust core 以 path 依赖直接引用，两端不各自维护消息定义。
12. **app-bridge 后端能力集**：桥接设备 id 形如 `bridge:<name>`；可用：snapshot/tap/swipe/input/key/screenshot/apps/launch；NOT_SUPPORTED：stop/logcat/shell（用户确认全命令对齐、受限降级），返回结构化能力不支持错误，不静默路由到其他后端。
13. **配对二维码与地址枚举**：qrcode crate 终端 unicode 输出配对 URI；局域网候选 IP 枚举默认路由网卡优先、全部候选列出（多网卡主机）。
14. **token 双侧存储**：daemon 侧配置目录（随 config.json 同目录 tokens 存储）；App 侧 SharedPreferences（私有模式）。`pair --reset` 重新生成配对码并清空全部已发 token。
15. **截图消息内联**：base64 PNG 内联于 result（WS 帧上限默认 16MB，1440×2560 PNG 约 1-3MB 足够）；二进制帧升级留后续。

## Risks / Trade-offs

- Tauri 2 mobile 构建链（Gradle + NDK + Rust targets）复杂度高 → 任务组 1 先行打通「构建出可安装 APK」再叠加能力；固定 toolchain 版本并记录
- 无障碍服务被系统或厂商回收导致命令失败 → App 侧检测服务状态并在 UI 提示；心跳附带权限状态，daemon 可据此标记设备降级
- MuMu arm64 对 QuickJS 原生库兼容性 → 先编译验证 arm64-v8a so 加载，再集成
- base64 截图消息体积大 → 首版接受；后续可升级二进制帧
- 前台服务带来常驻通知 → Android 平台要求，不可去除，UI 中说明用途

## Test Strategy

- Rust core（App 侧）：cargo 单测覆盖协议编解码、QuickJS 沙盒 `mobile.*` 绑定、配对状态机（配对码验证/token 签发与校验/重置失效）
- daemon WS：Rust 集成测试以模拟 WS client 完成 hello（配对码与 token 两种路径）/command/result/script/断线重连全流程
- app-bridge 后端：单测覆盖能力降级错误、devices 合并枚举、桥接路由输出结构与 ADB 后端一致
- Kotlin/UI：MuMu 手动验收（6.1 清单）+ 荣耀真机扫码验证；首版不引入 instrumented test 框架
- 端到端：安装 → 无障碍授权 → 手动与扫码配对 → devices 可见 → snapshot → `mobile.tap` 脚本 → screenshot → 断线重连 → `pair --reset` 旧 token 失效

## Migration Plan

无历史包袱；依赖 `init-adb-core` 归档后产生的 specs 作为基线。
