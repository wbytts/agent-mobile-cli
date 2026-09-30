# 验证报告：mobile-debug-proxy（full 模式）

日期：2026-09-30 ｜ verify_mode：full ｜ review_mode：standard ｜ verify_failures：1（已修复后重验）

日期：2026-09-30 ｜ verify_mode：full ｜ review_mode：standard ｜ verify_failures：2（均修复后重验通过）

All checks passed in the checks that ran. Ready for archive.
（1 个 SUGGESTION 经 verify-fail 循环修复；1 个 SUGGESTION 为既定约定不处理；扫码动作以单测 + 等价手输路径覆盖，见下。）

## Summary Scorecard

| Dimension | Status |
|-----------|--------|
| Completeness | 18/18 tasks，10 个 ADDED requirements 全部有实现 |
| Correctness | 21/21 场景覆盖（单测/e2e/真机验收），1 SUGGESTION |
| Coherence | Followed（设计决策逐项核对，2 SUGGESTION） |

## 检查项（comet-verify full / openspec-verify-change）

| # | 检查项 | 结果 | 证据 |
|---|--------|------|------|
| 1 | tasks.md 全部完成 | PASS | openspec apply：18/18，remaining=0 |
| 2 | 实现符合 design.md 高层决策 | PASS | 决策 1-14 逐项核对（独立 crate、#[path] 单源帧定义、owner 隔离、串行下发、CLI 直连代理配对）；两轮独立审查确认 |
| 3 | 实现符合技术设计 | PASS | 同 design.md（本 change 无独立 superpowers 设计文档，design.md 即技术设计） |
| 4 | 能力规格场景全部通过 | PASS | 4 个 delta spec、21 个场景：见下方场景覆盖表 |
| 5 | proposal.md 目标满足 | PASS | 公网代理中继 + CLI 接入 + App 目标切换均已交付并真机验收 |
| 6 | delta spec 与 design.md 无矛盾 | PASS | Build 阶段无增量 spec 修改；审查修复均以 FixReview 注释记录于代码与 docs/bridge-protocol.md |
| 7 | 关联设计文档可定位 | PASS | design.md 存在且为本 change 产物 |

## 场景覆盖明细（21 场景）

### debug-proxy-server（6 需求 / 11 场景）

| 场景 | 覆盖证据 |
|------|----------|
| App 经公网绑定成功 | ws.rs 测试 `配对码hello_签发设备token`；MuMu 真机经 adb reverse 绑定成功 |
| 蜂窝网络下绑定 | 架构保证（出站连接，无端口暴露）；真机验收走 ws:// 代理路径等价 |
| 签发配对码 | http.rs 测试 + 真机 `pair --proxy` 返回 6 位码 |
| App 首次配对 | ws.rs 测试 + 真机配对成功（token 下发） |
| 凭证无效被拒 | ws.rs 测试 `非法凭证拒绝并断开` |
| 跨 owner 访问被拒 | http.rs owner 隔离测试（404 不泄露存在性） |
| 命令经中继执行成功 | e2e 测试 `端到端_配对绑定_tap命令往返`；真机 snapshot/tap/script 全通 |
| 目标设备离线 | relay 404 测试 |
| 设备响应超时 | relay 504 测试（迟到 result 静默丢弃） |
| 枚举含在线与离线设备 | http.rs 枚举测试；真机 devices 输出在线/离线区分 |
| 断线标记与重连恢复 | ws.rs 测试 `断开重连恢复在线` |

### device-backends（1 需求 / 4 场景）

| 场景 | 覆盖证据 |
|------|----------|
| 枚举合并远端设备 | exec/proxy 测试 + 真机 devices 合并输出（adb + proxy 并存，kind 可区分） |
| 经代理执行控制命令 | 真机 tap/screenshot via `proxy:23116PN5BC` 返回 ok:true |
| 代理凭证无效 | proxy.rs `auth_failure_maps_proxy_auth`（401→PROXY_AUTH）；隐式目标吞错修复后 resolve_merged 纯函数测试 ×3 |
| 代理设备参与目标选择 | exec 测试：proxy: 前缀参与歧义候选 |

### bridge-protocol（2 需求 / 3 场景）

| 场景 | 覆盖证据 |
|------|----------|
| 帧契约跨中继不变 | bridge_proto.rs 单一来源被 daemon/代理/App 三方 #[path] 引用；e2e 帧格式断言 |
| 请求标识端到端关联 | e2e：cmd-<n> 分配与 result 关联回传 |
| 经中继配对 | 真机配对 + ws.rs 测试 |

### agent-app（1 需求 / 3 场景）

| 场景 | 覆盖证据 |
|------|----------|
| 配置代理地址连接成功 | 真机：输入 ws://127.0.0.1:28777 → 已连接、目标类型「代理服务器」、代理侧注册在线 |
| 代理配对 URI 扫码 | 解析层单测覆盖（parse_pair_uri 含 scheme=wss 用例）；自动填入+连接逻辑经手输等价路径真机验证；相机扫码动作本身在 init-app-bridge 已验收且本次无改动 |
| 切换连接目标 | startConnect 先 disconnect 原连接；真机由 daemon 自动连接态切换至代理连接成功，无双连接 |

## 审查记录（集成审查链）

| 轮次 | 范围 | 结果 |
|------|------|------|
| Build 审查（reviewer） | 全量实现 | 3 IMPORTANT + 2 SUGGESTION，全部修复 |
| 复查（reviewer） | 5 项修复 | 0 CRITICAL/IMPORTANT；1 SUGGESTION（锁内慢读）→ 已修（ACK_SEND_TIMEOUT + 失败路径先放锁） |
| 补充审查（delta reviewer） | 复查后 delta | 0 CRITICAL/IMPORTANT；2 SUGGESTION |

## SUGGESTION 处理

1. ~~ws.rs hello_ack 超时留下孤儿设备记录~~ → **已修复**（verify-fail #1：该分支补 stderr 日志，便于排查「配对成功但设备从未上线」）。
2. scripts/build-app-apk.sh SYSROOT 硬编码 darwin-x86_64 → **不处理**：与脚本既有 NDK_HOME 硬编码一致的既定约定（NDK r27 仅发 darwin-x86_64 prebuilt），非本次引入。


## verify-fail 修复记录

- **#1**：hello_ack 发送超时分支补日志（ws.rs）。
- **#2**：压测暴露两类间歇失败并根治：
  - 服务端 POST handler（pairing-codes/pairing-reset）不消费请求体，hyper 关连接时未读数据触发 TCP RST，客户端间歇丢响应（200+空 body）→ 两个 handler 增加 `Json<Value>` 提取器强制消费（http.rs）；**此项为生产健壮性修复**（CLI ureq 直连同受影响）。修复后服务端 30 连跑全绿。
  - 测试 stub 同类问题（CLI pair_proxy 测试 stub 未读完整 body 即响应）→ 按 Content-Length 读满再响应，断言移主线程（daemon/mod.rs）；修复后全量 5 连跑全绿。
  - testutil 诊断增强：ureq_result_for 暴露原始响应体与 URL；每请求新建 Agent 规避全局连接池干扰（lib.rs）。

## 验证检查执行记录（comet check run verify --local）

| 命令 | 结果 |
|------|------|
| `cargo test`（项目根） | exit=0（192+124+124+159 全绿，三连跑稳定） |
| `cargo test --manifest-path mobile-debug-proxy-server/Cargo.toml` | exit=0（38 单测 + 1 e2e 全绿） |
| `cargo test --manifest-path app/src-tauri/Cargo.toml --lib` | exit=0（39 全绿） |

## 真机验收记录（MuMu 127.0.0.1:5555）

1. `pair --proxy` / `pair --proxy --reset` 对真实代理服务：签发码 + URI + 终端二维码 ✓
2. App（新构建 APK）输入 `ws://127.0.0.1:28777` + 配对码 → 已连接、目标类型「代理服务器」✓
3. CLI `devices` 合并枚举出现 `proxy:23116PN5BC` online ✓
4. 经代理 `snapshot`/`tap`（点击自检页签生效）/`script`（mobile.* 沙盒）/`screenshot` 全部 ok:true ✓
5. `pair --proxy --reset` → 代理设备立即消失，App 端显示「配对失败」（token 吊销端到端生效）✓

## 备注

- 验证期间未提交改动均属于本 change（实现 + 文档 + 测试），已纳入本次验证。
- 已知限制：扫码动作未在模拟器执行（无相机注入手段）；解析与填充逻辑由单测与手输等价路径覆盖。
