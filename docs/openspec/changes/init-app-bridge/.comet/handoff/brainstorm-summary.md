# Brainstorm Summary

- Change: init-app-bridge
- Date: 2026-09-26

## 确认的技术方案

**分层架构**（在 Open design.md 草稿决策 1-11 上深化 12-18）：

```
CLI（pair 命令/devices 合并/app-bridge 后端）── HTTP ──▶ daemon ── WS :18777 ──▶ Tauri 2 App
                                                                              ├─ Web UI 三页
                                                                              ├─ Rust core：WS/协议/QuickJS 沙盒
                                                                              └─ Kotlin 插件：AccessibilityService/前台服务/扫码
```

12. **协议层全在 App 的 Rust core**：WS 客户端（tokio-tungstenite）、协议消息（serde）、QuickJS 沙盒（rquickjs bundled 交叉编译）；Kotlin 只做 AccessibilityService 能力桥、前台服务保活、扫码（ML Kit/zxing）。`mobile.*` 脚本调用 → Rust 插件命令 → Kotlin 执行 → 结果回传。
13. **协议类型单一来源**：CLI 侧新增协议模块（serde 消息类型），App Rust core 以 path 依赖引用同一文件/模块，两端不各写一份。
14. **daemon WS 完整实现**：预留握手占位升级为 hello（配对码或 token）→ 配对/注册 → 能力集协商 → command/script 路由 → result；设备注册表内存驻留；心跳 30s，断线标离线。
15. **app-bridge 后端**：实现 init-adb-core 的 Backend trait，桥接设备 id 形如 `bridge:<name>`；NOT_SUPPORTED 能力集：stop/logcat/shell（用户对齐决策），snapshot/tap/swipe/input/key/screenshot/apps/launch 可用。
16. **二维码**：qrcode crate 终端 unicode 输出；局域网 IP 枚举默认路由网卡优先、全候选列出。
17. **token 存储**：daemon 侧配置目录；App 侧 SharedPreferences（私有模式）。
18. **截图回传**：base64 PNG 内联 result（WS 帧默认 16MB 足够 1440×2560）。

## 关键取舍与风险

- Tauri 2 Android 构建链复杂（NDK/Gradle/Rust targets）→ 任务 1.1 先打通空壳 debug APK 再叠能力；固定 toolchain 版本
- rquickjs bundled 交叉编译 arm64-v8a 可行性 → 组 1 内早期编译 smoke 验证
- AccessibilityService 被系统回收 → 心跳携带权限状态，daemon 标记设备降级
- MuMu 模拟器扫码不便 → 手动配对为主验收路径，扫码在荣耀真机（NTH-AN00）验证
- 前台服务常驻通知（Android 平台强制，UI 中说明用途）
- 局域网明文 WS：配对码+token 为最低信任门槛，TLS 留后续

## 测试策略

- Rust core：cargo 单测（协议编解码、沙盒 `mobile.*` 绑定、配对状态机）
- daemon WS：Rust 集成测试（模拟 WS client 完成 hello/pair/command/result/断线重连）
- app-bridge 后端：单测（能力降级、枚举合并、桥接路由输出与 ADB 结构一致）
- Kotlin/UI：MuMu 手动验收（6.1 清单）+ 真机扫码验证；不新增 instrumented test 框架
- 端到端：6.1 全链路（安装→授权→手动+扫码配对→devices→snapshot→mobile.tap→screenshot→重连→pair --reset）

## Spec Patch

无新增——配对认证（5 场景）、扫码配对、多页诊断界面已在 Open 产物对账时回写 delta specs。
