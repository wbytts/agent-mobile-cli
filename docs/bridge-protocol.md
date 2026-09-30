# 桥接协议（bridge-protocol）

CLI daemon 与设备端调试 App 之间的 WebSocket 桥接协议。消息类型单一来源为
`src/bridge_proto.rs`（App Rust core 以 path 引用同一文件，design.md 决策 11）；
本文档与该文件逐字段一致，修改协议时必须同步更新。

**公网中继**：mobile-debug-proxy-server（仓库根目录独立 crate）作为公网中继复用本协议，
App 侧不感知对端是 daemon 还是中继服务（mobile-debug-proxy design.md 决策 1/3，见文末
「公网中继模式」章节）。

- 传输：WebSocket 文本帧（JSON，UTF-8）；daemon 监听 `0.0.0.0:<bridge_port>`（默认 18777，
  LAN 可达以支持真机直连/扫码配对，design.md 决策 8；暴露面由一次性配对码 + token 认证兜底）。
  HTTP 管理端点（含 `/pair-info` 配对码）仍只绑定 `127.0.0.1:18775`。
- 帧上限：默认 16MB；截图等二进制以 base64 内联于 `result`（design.md 决策 15）。
- 请求关联：`command`/`script` 的 `id` 由 daemon 生成（`cmd-<n>`），App 回传 `result` 携带同一 `id`。

## 消息一览

| type        | 方向           | 用途                                 |
| ----------- | -------------- | ------------------------------------ |
| hello       | App → daemon   | 注册握手（配对码或 token 认证）      |
| hello_ack   | daemon → App   | 握手应答（首次配对下发 token）       |
| heartbeat   | App → daemon   | 心跳（刷新 last_seen）               |
| pong        | daemon → App   | 心跳应答                             |
| command     | daemon → App   | 设备操作命令                         |
| script      | daemon → App   | 脚本下发（QuickJS 沙盒执行）         |
| result      | App → daemon   | command/script 执行回传              |
| result_ack  | daemon → App   | result 接收确认                      |

## App → daemon

### hello

连接建立后首条消息必须是 `hello`（10 秒超时，否则断开）。`pairing_code` 与 `token`
二选一：首次配对提交配对码；已配对设备凭 token。

```json
{
  "type": "hello",
  "pairing_code": "483920",
  "device_name": "MuMu",
  "android_version": "12",
  "capabilities": ["tap", "swipe", "input", "key", "uiTree", "screenshot", "apps", "launch", "script"]
}
```

| 字段            | 类型     | 必填 | 说明                                                         |
| --------------- | -------- | ---- | ------------------------------------------------------------ |
| pairing_code    | string   | 否*  | 6 位一次性配对码（首次配对用；与 token 二选一）              |
| token           | string   | 否*  | 长期 token，64 字符 hex（已配对设备用；与 pairing_code 二选一） |
| device_name     | string   | 是   | 设备名称；桥接设备 id 为 `bridge:<device_name>`              |
| android_version | string   | 是   | Android 版本（如 `"12"`）                                    |
| capabilities    | string[] | 是   | 能力集，取值见下                                             |

`capabilities` 枚举（`Capability`，serde camelCase）：`tap` / `swipe` / `input` /
`key` / `uiTree` / `screenshot` / `apps` / `launch` / `script`。未知取值反序列化失败，
视为非法 hello。

### heartbeat

```json
{ "type": "heartbeat" }
```

无字段。daemon 刷新该连接的 `last_seen` 并回 `pong`。超过 30 秒无任何消息
（`HEARTBEAT_TIMEOUT`）设备在枚举中标记为 `offline`；连接断开同样标记离线，
设备记录保留，重连注册后恢复在线。

### result

`command` / `script` 的执行回传，`id` 与下发帧一致。

```json
{ "type": "result", "id": "cmd-3", "ok": true, "result": { "tapped": [100, 200] } }
```

```json
{ "type": "result", "id": "cmd-4", "ok": false, "error": "element not interactable" }
```

| 字段   | 类型   | 必填 | 说明                                         |
| ------ | ------ | ---- | -------------------------------------------- |
| id     | string | 是   | 与下发的 command/script `id` 一致            |
| ok     | bool   | 是   | 执行是否成功                                 |
| result | any    | 否   | 成功时的结构化结果（截图为 base64 PNG 内联） |
| error  | string | 否   | 失败时的错误描述                             |

daemon 收到后按 `id` 唤醒等待方并回 `result_ack`；未知 `id`（如超时后迟到）静默丢弃。

## daemon → App

### hello_ack

```json
{ "type": "hello_ack", "ok": true, "token": "9f2c…64hex" }
```

```json
{ "type": "hello_ack", "ok": false, "error": "配对码错误或已失效" }
```

| 字段  | 类型   | 必填 | 说明                                                       |
| ----- | ------ | ---- | ---------------------------------------------------------- |
| ok    | bool   | 是   | 认证是否通过                                               |
| token | string | 否   | 配对码验证通过时新签发的长期 token（token 路径不重复下发） |
| error | string | 否   | 拒绝原因（配对码错误/失效、token 无效、缺少凭证等）        |

`ok=false` 时 daemon 发送该帧后即断开连接，设备不进入枚举。

### command

```json
{ "type": "command", "id": "cmd-3", "method": "tap", "params": { "x": 100, "y": 200 } }
```

| 字段   | 类型   | 必填 | 说明                                                              |
| ------ | ------ | ---- | ----------------------------------------------------------------- |
| id     | string | 是   | 请求标识（`cmd-<n>`），result 按此关联                            |
| method | string | 是   | 动作名：`tap`/`swipe`/`input`/`key`/`uiTree`/`screenshot`/`apps`/`launch` |
| params | object | 是   | 动作参数（无参数动作传 `{}`）                                     |

### command method 参数契约

各 `method` 的 `params` 与成功时 `result.result` 结构（CLI 侧 app-bridge 后端按此契约
序列化/解析，与 ADB 后端命令语义对齐；`ok=false` 时 `error` 为人读描述）：

| method      | params                                                      | result.result                                |
| ----------- | ----------------------------------------------------------- | -------------------------------------------- |
| `tap`       | `{ "x": i32, "y": i32 }`（屏幕坐标）                        | 任意（忽略）                                 |
| `swipe`     | `{ "x1", "y1", "x2", "y2", "duration_ms" }`（i32/u32）      | 任意（忽略）                                 |
| `input`     | `{ "text": string }`（JSON 传输，任意 Unicode，无转义体系） | 任意（忽略）                                 |
| `key`       | `{ "key": string }`（如 `"KEYCODE_HOME"`）                  | 任意（忽略）                                 |
| `uiTree`    | `{}`                                                        | `{ "xml": string }`（与 uiautomator dump 同构的 UI 树 XML） |
| `screenshot`| `{}`                                                        | `{ "png_base64": string }`（base64 PNG，决策 15） |
| `apps`      | `{ "filter": string\|null, "all": bool }`                   | `{ "packages": [string] }`                   |
| `launch`    | `{ "package": string }`                                     | 任意（忽略）                                 |

说明：

- `uiTree` 的 `full` 语义由 CLI 侧处理（full=原始 XML 直出，否则 CLI 复用同一
  `@eN` 简化分配逻辑，design.md 决策 5）；App 始终返回完整 XML。
- `apps` 的 `filter` 与 `all`（含系统应用）由设备侧执行。ADB 后端为
  `pm list packages [-3]`（第三方/全部包名）；桥接后端按 launcher 入口枚举
  （非 all 时有启动入口的应用，filter 匹配包名与应用标签子串）——两后端
  集合语义不同属预期（移动端无 pm 等价物），调用方不应假设一致。
- CLI `agent-mobile-cli script <file.js|->` 命令对应 `script` 帧（daemon 侧要求
  hello 上报 `script` 能力，缺失即 NOT_SUPPORTED 不下发）。
- 所有 command/script 下发前 daemon 按 hello `capabilities` 校验（决策 6）：
  能力缺失返回 NOT_SUPPORTED 结构化错误（含能力名与设备名），不下发帧。

### script

```json
{ "type": "script", "id": "cmd-5", "source": "mobile.tap(100, 200)" }
```

| 字段   | 类型   | 必填 | 说明                                        |
| ------ | ------ | ---- | ------------------------------------------- |
| id     | string | 是   | 请求标识，result 按此关联                   |
| source | string | 是   | JS 源码（QuickJS 沙盒执行，注入 mobile.* API） |

### result_ack

```json
{ "type": "result_ack", "id": "cmd-3" }
```

| 字段 | 类型   | 必填 | 说明                |
| ---- | ------ | ---- | ------------------- |
| id   | string | 是   | 已接收的 result `id` |

### pong

```json
{ "type": "pong" }
```

无字段。heartbeat 的应答。

## 配对认证流程（design.md 决策 8/14）

1. daemon 启动桥接监听时生成 6 位一次性配对码（首次配对成功或 daemon 重启后失效）。
2. `agent-mobile-cli pair` 经本机管理端点 `GET /pair-info`（只绑定 127.0.0.1）取配对码、
   候选局域网 IP（默认路由网卡优先）与桥接端口，输出配对 URI
   `agent-mobile://pair?host=<ip>&port=<port>&code=<code>` 及终端 unicode 二维码。
3. App 首次连接提交配对码 → 验证通过签发长期 token（32 字节随机 hex，64 字符），
   追加记录到 daemon 配置目录 `tokens.json`，配对码同时失效。
4. 后续连接凭 token 认证直过；`agent-mobile-cli pair --reset`（`POST /pair-reset`）
   重新生成配对码、清空全部已签发 token，并断开全部已连接桥接设备（发 Close 帧、
   立即标离线），设备需用新配对码重新配对。

## 连接生命周期

```text
App                daemon
 |--- WS 握手 ------>|
 |--- hello -------->|  认证（配对码签发 token / token 直过 / 拒绝）
 |<-- hello_ack -----|  ok=false 即断开
 |                   |  注册到设备注册表（id = bridge:<device_name>）
 |--- heartbeat ---->|
 |<-- pong ----------|  刷新 last_seen
 |<-- command -------|  executor 路由（组 5 经 BridgeRegistry.command/script）
 |--- result ------->|  按 id 唤醒等待方
 |<-- result_ack ----|
 |--- 断开 --------->|  标记离线（记录保留，重连注册恢复在线）
```

## 公网中继模式（mobile-debug-proxy）

`mobile-debug-proxy-server` 把本协议延伸到公网：App 出站连接中继服务，CLI 经中继的
HTTP API 间接调试，解决 App 与 CLI 主机不在同一局域网的场景。帧定义不变（单一来源
仍是 `src/bridge_proto.rs`），差异仅在角色与入口：

- **设备侧连接**：App 连接中继的 `WS /ws/device`（或 `/ws` 别名，与 daemon 路径一致），
  hello/heartbeat/result 各帧字段契约不变；配对码由中继服务签发与校验，App 不感知。
- **请求标识**：中继场景 command/script 的 `id` 由中继服务按全局序号分配（`cmd-<n>`），
  在同一设备连接上唯一；result 按该 id 端到端关联回传给 CLI 调用方。
- **CLI 侧 HTTP API**（除 `/healthz` 外均需 `Authorization: Bearer <owner-token>`）：

| 端点 | 说明 |
| ---- | ---- |
| `GET /healthz` | 健康检查（无认证） |
| `GET /devices` | owner 名下设备枚举 `{devices:[{name,online,capabilities,last_seen}]}`（last_seen 为 RFC3339） |
| `POST /devices/:name/commands` | body `{method,params}`，中继 command 并同步等待 result |
| `POST /devices/:name/scripts` | body `{source}`，中继 script 并同步等待 result |
| `POST /pairing-codes` | 签发一次性配对码，返回 `{pairing_code}` |
| `POST /pairing-reset` | 重置配对码、吊销 owner 全部设备 token 并断开其已连接设备 |

- **响应契约**：中继完成（含设备执行失败）统一 200 + `{"ok", "result"|"error"}`；
  设备不存在/离线 404 `{"error"}`；等待超时 504 `{"error"}`；认证失败 401 `{"error"}`；
  跨 owner 访问按 404 处理（不泄露设备存在性）。
- **owner 隔离**：设备注册表与配对码按 owner token 隔离；owner token 首启自动生成
  （64 hex）或由 `--owner-token` 注入；配对码在全服务范围唯一（跨 owner 无碰撞）。
- **reset 原子性**：`pairing-reset` 与 WS hello 的认证+注册互斥（同一把锁），
  在途 hello 要么先完成注册（随后被断开），要么在 reset 后认证失败——
  不存在「reset 返回后仍有设备以已吊销 token 在线」的窗口。
- **设备名约束**：1-64 字符，不得含 `/ ? # %` 或控制字符（设备名进入 HTTP 路径段，
  CLI 侧对空格/中文等做百分编码后调用）。
- **部署**：服务监听明文 HTTP/WS（默认 `0.0.0.0:28777`），公网部署由反向代理终结 TLS
  （wss/https）；App 地址输入 `wss://host` 形式即经反代连接。代理配对 URI 在 TLS
  部署时携带 `&scheme=wss`，App 扫码后按 wss:// 连接。
