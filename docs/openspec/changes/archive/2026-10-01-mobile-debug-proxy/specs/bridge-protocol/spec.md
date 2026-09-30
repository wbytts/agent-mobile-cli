# Spec Delta

## ADDED Requirements

### Requirement: 公网中继传输

协议帧 SHALL 可经公网中继服务转发：设备侧连接角色由「App 连本机 daemon」扩展为「App 连中继服务」，hello/heartbeat/command/script/result 各帧的字段契约保持不变；中继场景下 command/script 的请求标识 SHALL 由命令调用方一侧分配并在同一设备连接上唯一，result 按该标识关联回传。

#### Scenario: 帧契约跨中继不变

- **WHEN** App 经中继服务接入并收到 tap 命令帧
- **THEN** 帧结构与直连 daemon 场景完全一致，App 无需区分对端是 daemon 还是中继服务

#### Scenario: 请求标识端到端关联

- **WHEN** 调用方经中继服务下发携带请求标识的命令
- **THEN** 设备回传的 result 携带同一标识，调用方据此关联唤醒等待方

### Requirement: 中继配对模型

中继场景 SHALL 复用一次性配对码 + 长期 token 的认证模型：配对码由中继服务侧签发与校验，App 的 hello 帧字段不变，不感知配对码签发方是 daemon 还是中继服务。

#### Scenario: 经中继配对

- **WHEN** App 向中继服务提交有效配对码完成 hello
- **THEN** App 收到 hello_ack 与长期 token，后续连接凭 token 认证，流程与直连 daemon 一致
