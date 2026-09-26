---
comet_change: init-adb-core
role: technical-design
canonical_spec: openspec
---

# Design

## Context

新仓库，无任何既有源码；动机与范围见 proposal.md。主机环境：macOS arm64，Rust 1.93 与 Node 22 就绪；adb 位于 `~/Library/Android/sdk/platform-tools`。测试设备：MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a）。架构参照 agent-browser-cli：CLI 短进程 + 常驻 daemon（HTTP API + 桥接 WS 双端口）+ npm wrapper 分发。本 change 只交付 ADB 后端；WS 桥接端口仅做监听与握手占位，消息协议随 change `init-app-bridge` 定稿。

## Goals / Non-Goals

**Goals:**

- 单二进制 CLI 内含 daemon，后续命令毫秒级复用连接
- backend 抽象使后续 App 桥接作为新后端类型接入，控制命令契约不变
- ADB 后端覆盖 specs 中全部感知与控制命令，在 MuMu 模拟器上可验收

**Non-Goals:**

- App 桥接协议的消息格式与设备端实现（属 `init-app-bridge`）
- 内置/自动下载 adb 二进制（依赖主机 platform-tools）
- adb 无线配对（`adb pair`）流程自动化、USB 授权引导
- 投屏/视频流

## 架构

```text
agent-mobile-cli (CLI 短进程, clap 解析参数)
   |  HTTP POST /cmd  (127.0.0.1:18775)
   v
daemon (同一二进制 daemon 子命令, 后台驻留, 无状态转发)
   |-- HTTP API: GET /health, POST /cmd {argv}
   |-- 桥接 WS 监听 (127.0.0.1:18777, 本期握手占位)
   |-- 启动锁 ~/.agent-mobile-cli/daemon.lock (pid + port + started_at)
   |-- 后端注册表 { "adb": AdbBackend }  (预留 "app-bridge")
   |-- 快照引用缓存 (内存, @eN -> 坐标, 每次 snapshot 刷新)
   v
adb 子进程 (主机 platform-tools)  ->  adb server (5037)  ->  MuMu 127.0.0.1:5555
```

模块划分（单 crate 多模块）：`main`（CLI 入口）、`cli`（命令树与参数）、`config`（用户级配置）、`daemon`（lock/http/ws）、`backend`（统一接口 + 路由）、`adb`（子进程封装）、`ui`（uiautomator 解析 + 简化树 + @eN 引用）、`output`（JSON + 错误模型）。

## Decisions
1. **单进程双端口 daemon，无状态转发**：daemon 同时监听 HTTP API 与桥接 WS 端口；每条命令到达后 fork adb 子进程执行（adb server 5037 本身常驻，连接天然复用），daemon 不内嵌 adb 协议长连接。备选「daemon 内嵌 adb 长连接协议」放弃：adb fork 约 30-80ms 可接受，内嵌属过度设计；备选「无 daemon」放弃：WS 桥接端口必须常驻，且与参考项目模型不一致。
2. **HTTP API 单入口转发**：`GET /health` + `POST /cmd {argv}`。clap 在 CLI 侧解析参数，daemon 只做执行器；新增命令不需要改协议。备选「每命令一个 REST 端点」放弃：API 面随命令数膨胀。
3. **ADB 通过子进程调用主机 adb 实现**，不引入 Rust 原生 ADB 协议客户端。全部调用显式 `-s <serial>`；默认超时 10s（截图/文件推拉 30s）。探测顺序：配置文件 `adb_path` → `ANDROID_HOME`/`ANDROID_SDK_ROOT` → PATH → 平台常见安装路径。
4. **UI 快照走 uiautomator dump**：设备端生成 UI XML → 拉取解析 → 简化树 → 可交互节点按先序分配 `@eN` 引用。简化规则：剔除零面积节点与无文本无描述且不可交互的容器（递归上提子节点）；`@eN` 引用只分配给 clickable/long-clickable/focusable/scrollable 为真的节点，带文本/描述的非交互叶子保留在树中可见但不获引用（实现后以测试为准的澄清，不改变 spec 的「可交互元素分配引用」契约）。引用表记录各 `@eN` 的坐标中心点，存于 daemon 内存，每次 snapshot 全量刷新，daemon 重启失效（文档明示「引用仅对最近一次快照有效」）。
5. **输入边界**：`input text` 仅支持 ASCII（adb input 限制，空格转义为 `%s`）；含非 ASCII 的输入返回 `NOT_SUPPORTED` 结构化错误，不引入 ADBKeyBoard 等 IME 依赖。`key` 接受 keyevent 名称或数字码。
6. **截图/日志/应用的命令选型**：截图 `adb exec-out screencap -p`（exec-out 避免 PTY 换行损坏二进制，需 platform-tools 28+，启动时检测版本）；logcat `adb logcat -d -t <lines>` dump 模式（非阻塞，支持 tag/级别过滤）；启动应用 `monkey -p <pkg> 1`（免查 launcher activity）；停止 `am force-stop`。
7. **输出与错误模型**：stdout 恒为 JSON——成功 `{"ok":true,"result":{...}}`，失败 `{"ok":false,"error":{"code","message","details"}}`；退出码 0 成功、1 执行错误（clap 用法错误为 2）。错误码枚举：`ADB_NOT_FOUND`、`DEVICE_OFFLINE`、`DEVICE_AMBIGUOUS`、`DEVICE_NOT_FOUND`、`NOT_SUPPORTED`、`TIMEOUT`、`ADB_ERROR`、`IO_ERROR`。
8. **启动锁**：锁文件 + 端口探测双重判定，锁内记录 pid 与启动时间，过期锁可回收；CLI 健康检查失败时自动清理并重启 daemon。
9. **配置与端口**：用户级 `~/.agent-mobile-cli/config.json`（`http_port` 默认 18775、`bridge_port` 默认 18777、`adb_path`、`default_device`），缺失自动生成默认值；端口默认值避开 agent-browser-cli 的 18765/18767。
10. **后端抽象层**：统一后端接口（设备枚举、快照、触控、截图、shell、日志、应用管理），设备记录携带后端类型标识，路由按设备 ID 分发；`adb` 为首个实现，`app-bridge` 预留类型位。
11. **分发**：GitHub Release 多平台预编译二进制，npm 包 postinstall 按 `process.platform`/`process.arch` 下载对应二进制（与参考项目模式一致）。

## 测试策略

- **单元测试（无设备依赖）**：adb 输出解析器（devices -l、uiautomator XML、pm list）、简化树与 @eN 分配算法、配置文件生成/读取、启动锁逻辑、错误模型序列化。
- **并发锁测试**：双进程同时抢锁，断言仅一个 daemon 绑定端口、另一个转为复用。
- **集成测试（默认 `#[ignore]`，`AGENT_MOBILE_TEST_DEVICE=127.0.0.1:5555` 启用）**：MuMu 全链路——devices/connect/snapshot/tap/input/screenshot/apps/logcat/shell，逐项对应 specs 场景。
- CI 跑单元与并发测试；集成测试本地手动触发。

## Risks / Trade-offs

- adb 子进程输出格式随 platform-tools 版本漂移 → 优先机器可读输出与显式 `-s`；验证环境固定 MuMu + 当前 platform-tools 版本
- uiautomator dump 在 WebView、游戏 Surface 等页面内容缺失 → spec 仅承诺「当前屏幕 UI 层级」，感知缺口由截图兜底
- MuMu 多实例 adb 端口不固定（5555/16384…） → 设备枚举覆盖网络设备，`connect host:port` 支持主动接入
- daemon 异常退出残留锁文件 → 锁内含 pid 存活检测与过期回收；健康检查失败自动清理重启
- 中文等非 ASCII 输入不支持 → spec 未承诺；返回明确 `NOT_SUPPORTED` 错误
- 引用表内存化，daemon 重启后 @eN 失效 → 换取实现简单，文档明示有效期
- daemon 内嵌会增加首版工作量 → 换来命令延迟与连接稳定性，与参考项目一致的收益，接受

## Migration Plan

新仓库首次落地，无迁移。版本自 `0.1.0` 起。
