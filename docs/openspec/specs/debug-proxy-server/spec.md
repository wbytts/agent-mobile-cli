# debug-proxy-server Specification

## Purpose
提供部署在公网可达主机上的代理中继服务，使调试 App 与 CLI 不在同一局域网时仍能建立桥接通路：App 出站绑定、CLI 经 HTTP API 间接调试设备。

## Requirements

### Requirement: 设备出站绑定

代理服务 SHALL 监听 WebSocket 端点接受设备端 App 的出站连接；App 完成注册握手（上报设备名称、Android 版本与能力集）后，服务 SHALL 将该设备纳入其所属 owner 的设备注册表并标记在线。

#### Scenario: App 经公网绑定成功

- **WHEN** App 向代理服务的 WS 端点发起连接并完成注册握手
- **THEN** 服务侧设备注册表中出现该设备且状态为在线，同 owner 的 CLI 可枚举到

#### Scenario: 蜂窝网络下绑定

- **WHEN** App 处于蜂窝网络（无法直连任何 CLI 主机）并连接代理服务
- **THEN** 绑定流程与局域网场景一致，无需设备侧任何端口暴露

### Requirement: 配对与凭证认证

代理服务 SHALL 支持 owner 凭证（长期 token）标识调用方身份；owner 可经认证 API 签发一次性配对码；App 首次注册 SHALL 提交配对码，验证通过后服务下发设备 token，后续连接凭设备 token 认证；配对码错误、失效或凭证缺失时 SHALL 拒绝注册。

#### Scenario: 签发配对码

- **WHEN** CLI 携带有效 owner token 请求签发配对码
- **THEN** 服务返回一次性配对码，该码在首次配对成功或重置后失效

#### Scenario: App 首次配对

- **WHEN** App 提交正确配对码完成注册
- **THEN** 服务下发设备 token，设备进入该 owner 的注册表

#### Scenario: 凭证无效被拒

- **WHEN** App 提交错误配对码或无效设备 token
- **THEN** 服务拒绝注册并断开连接，设备不进入任何注册表

### Requirement: 多 owner 隔离

代理服务 SHALL 按 owner 隔离设备注册表与配对码；一个 owner 的凭证 SHALL NOT 枚举、下发命令或重置另一 owner 的设备。

#### Scenario: 跨 owner 访问被拒

- **WHEN** owner A 的凭证请求操作 owner B 名下的设备
- **THEN** 服务返回认证/未找到错误，不泄露设备存在性以外的信息

### Requirement: 命令中继

代理服务 SHALL 提供认证 HTTP API 接收 CLI 的命令与脚本请求，经 WS 转发至目标设备，并将设备回传的结果或错误按请求标识关联后返回给 CLI；设备离线或等待超时 SHALL 返回明确的结构化错误。

#### Scenario: 命令经中继执行成功

- **WHEN** CLI 对在线设备调用命令 API
- **THEN** 服务将命令经 WS 下发设备，设备结果回传后 API 返回成功结果

#### Scenario: 目标设备离线

- **WHEN** CLI 对离线设备调用命令 API
- **THEN** API 返回设备离线的结构化错误，不产生悬挂等待

#### Scenario: 设备响应超时

- **WHEN** 命令下发后设备在超时窗口内未回传结果
- **THEN** API 返回超时错误，迟到的结果静默丢弃

### Requirement: 设备枚举 API

代理服务 SHALL 提供认证 HTTP API 返回 owner 名下全部已注册设备及其在线状态、能力集与最近活跃时间。

#### Scenario: 枚举含在线与离线设备

- **WHEN** 一台设备在线、另一台已注册但断线，CLI 调用枚举 API
- **THEN** 返回两台设备记录，在线状态可区分

### Requirement: 心跳与离线标记

代理服务 SHALL 维持设备连接的心跳；连接断开或心跳超时时 SHALL 将设备标记为离线（注册记录保留），设备重连并完成认证后恢复在线。

#### Scenario: 断线标记与重连恢复

- **WHEN** 设备连接断开，随后 App 重新连接并凭设备 token 认证
- **THEN** 服务先将设备标记离线，重连认证后恢复在线且命令中继恢复可用
