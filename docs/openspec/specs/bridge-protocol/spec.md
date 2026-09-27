# bridge-protocol Specification

## Purpose
定义 CLI daemon 与设备端调试 App 之间的 WebSocket 桥接协议，使设备无需 adb 连接即可接入 CLI 的统一设备模型，接收命令与脚本并回传结果。

## Requirements

### Requirement: 设备连接与注册

设备端 App 连接 daemon 的桥接 WS 端口后 SHALL 完成注册握手，上报设备名称、Android 版本与能力集；注册成功后 daemon SHALL 将该设备纳入设备枚举，后端类型标记为桥接类型。

#### Scenario: 注册后设备可见

- **WHEN** App 与 daemon 建立 WS 连接并完成注册
- **THEN** CLI 设备枚举结果中包含该设备，且后端类型与 ADB 设备可区分

### Requirement: 心跳与断线重连

桥接连接 SHALL 维持心跳；连接断开时 daemon SHALL 将对应设备标记为离线，App 恢复连接后 SHALL 自动重新注册并恢复在线状态。

#### Scenario: App 进程被杀后恢复

- **WHEN** App 进程被结束导致连接断开，随后用户重新打开 App
- **THEN** daemon 先将设备标记为离线，App 重连注册后恢复在线

### Requirement: 命令分发与结果回传

daemon SHALL 支持通过桥接连接向指定设备下发命令消息（请求标识、动作、参数），App 执行后 SHALL 回传与该请求关联的结构化结果或错误。

#### Scenario: 命令成功回传

- **WHEN** daemon 向已注册设备下发点击命令
- **THEN** App 执行后回传成功结果，CLI 输出与 ADB 后端一致的 `ok: true` 结构

#### Scenario: 命令失败回传

- **WHEN** App 执行命令失败（如目标元素不可交互）
- **THEN** 回传错误信息，CLI 输出结构化错误并以非零退出码结束

### Requirement: 配对认证

daemon SHALL 在桥接监听启动时生成一次性配对码，并提供 CLI 命令（`agent-mobile-cli pair`）显示配对码、候选局域网地址与配对二维码（URI 含 host/port/配对码）；App 首次注册 SHALL 提交配对码，验证通过后 daemon 下发长期 token，后续连接凭 token 认证；配对码错误或缺失时 daemon SHALL 拒绝注册。

#### Scenario: 显示配对信息

- **WHEN** 用户执行 `agent-mobile-cli pair`
- **THEN** 输出当前配对码、候选局域网地址与配对二维码（URI `agent-mobile://pair?host=...&port=...&code=...`）

#### Scenario: 首次配对成功

- **WHEN** App 首次连接并提交正确配对码（手动输入或扫码 URI 解析）
- **THEN** daemon 验证通过并下发长期 token，App 保存 token 并完成注册

#### Scenario: 凭 token 重连免配对

- **WHEN** 已配对 App 使用保存的 token 重新连接
- **THEN** daemon 验证 token 后直接完成注册，无需再次输入配对码

#### Scenario: 配对码错误被拒

- **WHEN** App 提交错误或已失效的配对码
- **THEN** daemon 拒绝注册并返回认证错误，设备不出现在枚举中

#### Scenario: 重置配对

- **WHEN** 用户执行 `agent-mobile-cli pair --reset`
- **THEN** daemon 重新生成配对码，此前下发的全部 token 失效，已连接设备被要求重新配对

### Requirement: 脚本下发执行

协议 SHALL 支持脚本动作，携带 JS 源码由 App 在脚本沙盒中执行，并将脚本返回值或异常回传 daemon。

#### Scenario: 脚本返回执行结果

- **WHEN** daemon 下发一段调用设备操作 API 的脚本
- **THEN** App 在沙盒中执行并回传脚本返回值；脚本抛异常时回传错误信息
