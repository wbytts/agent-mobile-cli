# Tasks

## 1. 项目骨架与配置

- [x] 1.1 创建 Cargo workspace 与 CLI 骨架（clap 命令树），验证 `cargo run -- --version` 与 `--help` 正常输出 <!-- comet-task:aac0aabc-9089-41b4-b30a-8d2e78b2c96c -->
- [x] 1.2 实现用户级配置文件（`~/.agent-mobile-cli/config.json`，缺失自动生成默认值），单元测试覆盖生成与读取 <!-- comet-task:155f973a-2b09-4aa6-8c29-9fd60aa5e8ce -->
- [x] 1.3 完善 .gitignore 与基础 CI（fmt/clippy/test），验证 `cargo test` 全绿 <!-- comet-task:b8f7acdc-c280-490a-ab6d-f12cb9d85907 -->

## 2. 常驻 daemon

- [x] 2.1 实现启动锁（锁文件 + pid 存活检测 + 过期回收），并发启动测试验证只有一个 daemon 绑定端口 <!-- comet-task:627002aa-d98c-493d-8278-b05562c0666d -->
- [x] 2.2 实现 HTTP API 服务（健康检查 + 命令路由入口），验证 curl 健康检查返回 ok <!-- comet-task:2cf57575-7d35-4f1d-9566-35e698f40d7a -->
- [x] 2.3 实现桥接 WS 端口监听与握手占位，验证 WebSocket 客户端握手成功 <!-- comet-task:5faf1b78-9cba-40e8-9dfe-392d2de93f0e -->
- [x] 2.4 实现 daemon 生命周期命令（status/restart/stop），验证 status 输出运行状态与端口 <!-- comet-task:772e0432-8c9a-4e00-9dad-024b87ff7f9c -->

## 3. ADB 后端与路由抽象

- [x] 3.1 实现 adb 探测链（配置 → 环境变量 → PATH → 常见安装路径），单元测试覆盖优先级与缺失时的指引性错误 <!-- comet-task:ba46e25e-fea6-4c1c-9dbd-8bce5a2f2a8a -->
- [x] 3.2 实现设备枚举（解析 `adb devices -l`），验证 `devices` 列出 MuMu 模拟器 127.0.0.1:5555 且状态在线 <!-- comet-task:5d2b80ab-3e3b-496a-a868-725b8373dcc6 -->
- [x] 3.3 实现 `connect host:port`，验证连接后设备出现在枚举结果中 <!-- comet-task:ede50616-6063-4c3d-9503-d5c2870b415b -->
- [x] 3.4 实现 `--device` 目标选择与多设备歧义错误，验证两台在线设备未指定时输出候选列表 <!-- comet-task:7cf82c41-e766-45ff-bb94-9841f81d5d1d -->
- [x] 3.5 实现后端抽象层（统一后端接口 + 设备记录后端类型 + 命令路由，预留 app-bridge 类型），单元测试验证 adb 后端路由 <!-- comet-task:1b9f0a2d-fe58-4cce-bf3f-30e40613daa6 -->

## 4. 设备控制命令

- [x] 4.1 实现 `snapshot`（uiautomator dump 拉取解析 + 简化树 + `@eN` 引用表），验证 MuMu 系统设置页快照含带坐标区域的元素引用 <!-- comet-task:8e1074c3-e5d7-412c-a3d6-4cf3db1f5407 -->
- [x] 4.2 实现 `tap`/`swipe`（坐标与元素引用两种寻址），验证点击设置项后界面实际跳转 <!-- comet-task:d694d982-c500-4692-bbd5-c72b7796bb1a -->
- [x] 4.3 实现 `input` 文本输入与 `key` 按键事件，验证输入框出现目标文本 <!-- comet-task:eea065a5-e566-43bb-bc82-1eb883a430f6 -->
- [x] 4.4 实现 `screenshot`（PNG 保存，支持输出路径），验证文件非空且可打开、内容与屏幕一致 <!-- comet-task:38910be9-474a-4096-92f4-f6754534e220 -->
- [x] 4.5 实现 `apps` 列表（可过滤）与 `launch`/`stop`，验证启动系统设置后前台切换 <!-- comet-task:c39d64c0-29ae-45b8-8fa0-42c28d1cc46d -->
- [x] 4.6 实现 `logcat`（最近 N 行 + tag/级别过滤），验证输出行数不超过指定值 <!-- comet-task:abde685c-fc82-42b8-92ba-cb12ac9622ad -->
- [x] 4.7 实现 `shell` 透传（返回 stdout/stderr/退出码），验证 `getprop ro.product.model` 返回设备型号 <!-- comet-task:703fbaef-5f87-4714-8e74-5cea36654033 -->
- [x] 4.8 实现设备离线与操作超时的结构化错误，验证断开模拟器后命令以 device offline 错误结束且不挂起 <!-- comet-task:5be39b82-14a3-4d5d-99d4-61b2aeceb3b8 -->

## 5. 分发与 Agent 文档

- [ ] 5.1 实现 npm wrapper（package.json + postinstall 按平台定位二进制），验证本地打包安装后 `agent-mobile-cli --version` 可执行 <!-- comet-task:93f996b8-1581-4ea5-8dd5-b72e9f52f4c6 -->
- [ ] 5.2 编写 `skills/agent-mobile-cli/SKILL.md`（覆盖全部命令、选项与典型 SOP），验证文档中每条命令与已实现行为一致 <!-- comet-task:92ae8a59-b613-45e5-a3b3-6ef4fc8bff5d -->
- [ ] 5.3 编写 README（安装、快速自检、命令速览），验证快速自检段落命令实跑通过 <!-- comet-task:e1cf9c74-821c-4df8-bd26-3d743e05b014 -->

## 6. 端到端验收

- [ ] 6.1 在 MuMu 模拟器执行全链路验收：devices → connect → snapshot → tap → input → screenshot → logcat → shell，逐项对照 specs 场景通过 <!-- comet-task:383a8dbf-e7bb-4e09-9d20-e34c7e9b4a83 -->
