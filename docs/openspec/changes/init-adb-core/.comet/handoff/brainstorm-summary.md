# Brainstorm Summary

- Change: init-adb-core
- Date: 2026-09-26

状态：已确认（2026-09-26 用户确认本设计）

## 确认的技术方案

**架构**：单 crate 多模块。CLI 短进程解析参数后经 HTTP（127.0.0.1:18775）转发到常驻 daemon 执行；daemon 监听桥接 WS（127.0.0.1:18777，本期握手占位）。daemon 本身无状态：每条命令到达后 fork adb 子进程执行（adb server 5037 本身常驻，天然复用），daemon 持有配置/启动锁/WS 监听/快照引用缓存。

```
CLI (clap 解析) --HTTP POST /cmd--> daemon --> fork adb --> adb server --> MuMu 127.0.0.1:5555
```

**模块**：main/cli（命令树）、config（~/.agent-mobile-cli/config.json）、daemon（lock/http/ws）、backend（trait + 路由 + 注册表，预留 app-bridge）、adb（子进程封装）、ui（uiautomator 解析 + 简化树 + @eN 引用）、output（JSON + 错误模型）。

**HTTP API**：`GET /health`、`POST /cmd {argv}` 单入口转发（新命令不改 API）。

**输出/错误模型**：stdout JSON `{ok,result|error{code,message,details}}`；退出码 0/1（用法错误 clap 2）。错误码：ADB_NOT_FOUND/DEVICE_OFFLINE/DEVICE_AMBIGUOUS/DEVICE_NOT_FOUND/NOT_SUPPORTED/TIMEOUT/ADB_ERROR/IO_ERROR。

**adb 封装**：全部调用显式 `-s <serial>`；默认超时 10s（截图/推拉 30s）；探测链 config → ANDROID_HOME/ANDROID_SDK_ROOT → PATH → 平台常见路径。

**snapshot 简化树**：`uiautomator dump` → pull → 解析 XML；过滤零面积/无内容不可交互容器；可交互判定（clickable/focusable/scrollable 或带文本叶子）；@eN 先序编号，引用表存 daemon 内存（每次 snapshot 刷新，daemon 重启失效）。

**输入边界（重要）**：`input text` 仅 ASCII（adb 限制，空格转 %s），非 ASCII 返回 NOT_SUPPORTED 明确错误，不引入 IME 依赖。

**其余命令**：screenshot 用 `adb exec-out screencap -p`（exec-out 防 PTY 损坏二进制）；logcat 用 `adb logcat -d -t N` dump 模式；launch 用 `monkey -p pkg 1`（免查 launcher activity）；stop 用 `am force-stop`。

**端口**：HTTP 18775 / WS 18777（避开 agent-browser-cli 的 18765/18767）。

## 关键取舍与风险

- daemon 无状态转发（备选：daemon 内嵌 adb 协议长连接）——adb fork 约 30-80ms 可接受，内嵌协议过度设计
- @eN 引用存 daemon 内存，重启失效 → 文档说明「引用仅对最近一次快照有效」
- 中文输入不支持 → spec 未承诺；返回明确错误
- exec-out 需 platform-tools 28+（2020 年后均满足）→ 版本检测报错
- daemon 崩溃 → CLI 健康检查失败自动清理锁并重启

## 测试策略

- 单元测试（无设备）：adb 输出解析、简化树算法、配置、锁、错误模型
- 集成测试（默认 #[ignore]，AGENT_MOBILE_TEST_DEVICE=127.0.0.1:5555 启用）：MuMu 全链路
- 并发锁测试：双进程抢锁断言单一绑定
- CI 跑单测；集成测试本地触发

## Spec Patch

无。
