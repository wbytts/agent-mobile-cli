---
change: init-adb-core
design-doc: docs/openspec/changes/init-adb-core/design.md
base-ref: 20b4422426ea4223f76ffc0b2d62b7d42cd196e1
archived-with: 2026-09-26-init-adb-core
---

# 实施计划：init-adb-core

<!-- comet-task-authority: docs/openspec/changes/init-adb-core/tasks.md -->

执行策略：autonomous + tdd + review standard。设计决策见 design.md，行为契约见 specs/，此处只排实施顺序、依赖与验证。

依赖基线：clap 4（derive）、tokio、axum（HTTP API）、tokio-tungstenite（WS 监听）、serde/serde_json、quick-xml（uiautomator）、thiserror、which（adb 探测）。

## 组 1：项目骨架与配置（无外部依赖）

- 任务 1.1 `aac0aabc-9089-41b4-b30a-8d2e78b2c96c` <!-- comet-task-ref:aac0aabc-9089-41b4-b30a-8d2e78b2c96c -->：Cargo 单 crate + clap 命令树（devices/connect/snapshot/tap/swipe/input/key/screenshot/apps/launch/stop/logcat/shell/daemon-status/daemon-restart/daemon-stop/daemon 内部子命令）。
  - RED：`cargo run -- --version` 不存在命令树断言测试失败；GREEN：clap 解析单测通过。
  - 验收：`cargo run -- --version` 与 `--help` 正常输出。
- 任务 1.2 `155f973a-2b09-4aa6-8c29-9fd60aa5e8ce` <!-- comet-task-ref:155f973a-2b09-4aa6-8c29-9fd60aa5e8ce -->：`config` 模块（`~/.agent-mobile-cli/config.json`，http_port 18775 / bridge_port 18777 / adb_path / default_device，缺失自动生成）。
  - RED：Config::load_or_default 单测失败；GREEN：通过。
  - 验收：`cargo test config` 通过；删除临时 HOME 下配置文件后重建。
- 任务 1.3 `b8f7acdc-c280-490a-ab6d-f12cb9d85907` <!-- comet-task-ref:b8f7acdc-c280-490a-ab6d-f12cb9d85907 -->：.gitignore（target/.codegraph）、GitHub Actions（fmt+clippy+test）。
  - 验收：`cargo fmt --check && cargo clippy -- -D warnings && cargo test` 全绿。

## 组 2：常驻 daemon（依赖组 1）

- 任务 2.1 `627002aa-d98c-493d-8278-b05562c0666d` <!-- comet-task-ref:627002aa-d98c-493d-8278-b05562c0666d -->：`daemon::lock`（锁文件 + pid 存活检测 + 过期回收）。
  - RED：双进程抢锁集成测试失败；GREEN：仅一方持有。
  - 验收：`cargo test lock` 含并发用例通过。
- 任务 2.2 `2cf57575-7d35-4f1d-9566-35e698f40d7a` <!-- comet-task-ref:2cf57575-7d35-4f1d-9566-35e698f40d7a -->：`daemon::http`（axum：GET /health、POST /cmd 占位回显）、CLI 自动确保 daemon（健康检查失败→清理锁→spawn `daemon` 子命令后台化）。
  - RED：/health 客户端测试失败；GREEN：通过。
  - 验收：启动后 `curl 127.0.0.1:18775/health` 返回 ok。
- 任务 2.3 `5faf1b78-9cba-40e8-9dfe-392d2de93f0e` <!-- comet-task-ref:5faf1b78-9cba-40e8-9dfe-392d2de93f0e -->：`daemon::ws`（tokio-tungstenite 监听握手占位，hello 后保持连接）。
  - RED：WS 握手测试失败；GREEN：握手成功并收到占位响应。
  - 验收：`cargo test ws` 通过。
- 任务 2.4 `772e0432-8c9a-4e00-9dad-024b87ff7f9c` <!-- comet-task-ref:772e0432-8c9a-4e00-9dad-024b87ff7f9c -->：daemon-status/restart/stop 命令（经锁文件与 /health 实现）。
  - 验收：status 输出端口与运行状态；stop 后端口释放。

## 组 3：ADB 后端与路由抽象（依赖组 1，与组 2 并行可做，集成在组 4）

- 任务 3.1 `ba46e25e-fea6-4c1c-9dbd-8bce5a2f2a8a` <!-- comet-task-ref:ba46e25e-fea6-4c1c-9dbd-8bce5a2f2a8a -->：`adb::locate`（探测链：配置→ANDROID_HOME/ANDROID_SDK_ROOT→PATH→常见路径）与 ADB_NOT_FOUND 指引错误。
  - RED：探测优先级单测（临时目录桩）失败；GREEN：通过。
- 任务 3.2 `5d2b80ab-3e3b-496a-a868-725b8373dcc6` <!-- comet-task-ref:5d2b80ab-3e3b-496a-a868-725b8373dcc6 -->：`adb::devices`（解析 `adb devices -l`：serial/state/model/transport）+ `devices` 命令。
  - RED：样例文本解析单测失败；GREEN：通过。
  - 验收（集成）：`devices` 列出 127.0.0.1:5555 在线。
- 任务 3.3 `ede50616-6063-4c3d-9503-d5c2870b415b` <!-- comet-task-ref:ede50616-6063-4c3d-9503-d5c2870b415b -->：`connect host:port` 命令。
  - 验收（集成）：disconnect 后 connect 恢复可见。
- 任务 3.4 `7cf82c41-e766-45ff-bb94-9841f81d5d1d` <!-- comet-task-ref:7cf82c41-e766-45ff-bb94-9841f81d5d1d -->：目标设备解析（--device → default_device → 单在线 → DEVICE_AMBIGUOUS 候选列表）。
  - RED：解析单测（0/1/N 设备三态）失败；GREEN：通过。
- 任务 3.5 `1b9f0a2d-fe58-4cce-bf3f-30e40613daa6` <!-- comet-task-ref:1b9f0a2d-fe58-4cce-bf3f-30e40613daa6 -->：`backend` trait（枚举/快照/触控/截图/shell/日志/应用）+ 设备记录（id/backend 类型）+ 路由；`adb` 实现注册，预留 `app-bridge` 类型位。
  - RED：路由单测失败；GREEN：adb 类型正确路由。

## 组 4：设备控制命令（依赖组 3，经组 2 daemon 执行）

- 任务 4.1 `8e1074c3-e5d7-412c-a3d6-4cf3db1f5407` <!-- comet-task-ref:8e1074c3-e5d7-412c-a3d6-4cf3db1f5407 -->：`ui` 模块（uiautomator XML 解析、简化树、@eN 先序分配）+ snapshot 命令 + daemon 内存引用表。
  - RED：样例 XML 简化单测失败；GREEN：通过。
  - 验收（集成）：MuMu 设置页 snapshot 输出含 @eN 与坐标。
- 任务 4.2 `d694d982-c500-4692-bbd5-c72b7796bb1a` <!-- comet-task-ref:d694d982-c500-4692-bbd5-c72b7796bb1a -->：tap/swipe（坐标直传；@eN 经查引用表取中心点）。
  - 验收（集成）：点击设置项界面跳转。
- 任务 4.3 `eea065a5-e566-43bb-bc82-1eb883a430f6` <!-- comet-task-ref:eea065a5-e566-43bb-bc82-1eb883a430f6 -->：input（ASCII 校验、空格 %s 转义、非 ASCII → NOT_SUPPORTED）+ key。
  - RED：ASCII 校验单测失败；GREEN：通过。
  - 验收（集成）：输入框出现目标文本。
- 任务 4.4 `38910be9-474a-4096-92f4-f6754534e220` <!-- comet-task-ref:38910be9-474a-4096-92f4-f6754534e220 -->：screenshot（`adb exec-out screencap -p`，platform-tools 版本检测）。
  - 验收（集成）：PNG 非空且与屏幕一致。
- 任务 4.5 `c39d64c0-29ae-45b8-8fa0-42c28d1cc46d` <!-- comet-task-ref:c39d64c0-29ae-45b8-8fa0-42c28d1cc46d -->：apps（`pm list packages` 可过滤）、launch（monkey）、stop（am force-stop）。
  - RED：pm 输出解析单测失败；GREEN：通过。
  - 验收（集成）：launch 设置后前台切换。
- 任务 4.6 `abde685c-fc82-42b8-92ba-cb12ac9622ad` <!-- comet-task-ref:abde685c-fc82-42b8-92ba-cb12ac9622ad -->：logcat（`-d -t N` + tag/级别）。
  - 验收（集成）：输出行数 ≤ 指定值。
- 任务 4.7 `703fbaef-5f87-4714-8e74-5cea36654033` <!-- comet-task-ref:703fbaef-5f87-4714-8e74-5cea36654033 -->：shell 透传（stdout/stderr/exit code 三段返回）。
  - 验收（集成）：getprop 返回型号与退出码 0。
- 任务 4.8 `5be39b82-14a3-4d5d-99d4-61b2aeceb3b8` <!-- comet-task-ref:5be39b82-14a3-4d5d-99d4-61b2aeceb3b8 -->：离线/超时统一错误（子进程超时 TIMEOUT、device offline 探测 DEVICE_OFFLINE）。
  - 验收（集成）：断开后命令结构化报错且不挂起。

## 组 5：分发与文档（依赖组 4 命令面稳定）

- 任务 5.1 `93f996b8-1581-4ea5-8dd5-b72e9f52f4c6` <!-- comet-task-ref:93f996b8-1581-4ea5-8dd5-b72e9f52f4c6 -->：npm/（package.json + bin 转发 + postinstall 按平台定位二进制，支持本地回退到 cargo 构建产物）。
  - 验收：`npm pack` 后本地安装，`agent-mobile-cli --version` 可执行。
- 任务 5.2 `92ae8a59-b613-45e5-a3b3-6ef4fc8bff5d` <!-- comet-task-ref:92ae8a59-b613-45e5-a3b3-6ef4fc8bff5d -->：skills/agent-mobile-cli/SKILL.md（全部命令、选项、SOP）。
  - 验收：逐条命令与 clap --help 对齐。
- 任务 5.3 `e1cf9c74-821c-4df8-bd26-3d743e05b014` <!-- comet-task-ref:e1cf9c74-821c-4df8-bd26-3d743e05b014 -->：README（安装、快速自检、命令速览）。
  - 验收：快速自检命令实跑通过。

## 组 6：端到端验收（依赖全部）

- 任务 6.1 `383a8dbf-e7bb-4e09-9d20-e34c7e9b4a83` <!-- comet-task-ref:383a8dbf-e7bb-4e09-9d20-e34c7e9b4a83 -->：MuMu 全链路：devices → connect → snapshot → tap → input → screenshot → logcat → shell，逐项对照 specs 场景。
  - 验收：全部场景通过并记录输出。

## 风险与回退

- 集成测试依赖 MuMu 在线：AGENT_MOBILE_TEST_DEVICE 未设置时自动跳过，不阻塞单测。
- 任务组间提交边界：每组完成后提交一次，便于分段审查与回退。
