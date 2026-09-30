# Spec Delta

## ADDED Requirements

### Requirement: 公网代理连接目标

App SHALL 支持配置公网代理服务器地址作为本机 daemon 之外的第二种连接目标；连接代理服务器时 SHALL 复用同一注册握手、配对码/token 认证与断线自动重连流程；连接页 SHALL 可区分当前连接目标是本机 daemon 还是代理服务器。

#### Scenario: 配置代理地址连接成功

- **WHEN** 用户在 App 中输入代理服务器地址与配对码并发起连接
- **THEN** App 显示已连接状态，代理服务侧出现对应设备注册，连接页标明当前目标为代理服务器

#### Scenario: 代理配对 URI 扫码

- **WHEN** 用户扫描指向代理服务器的配对 URI 二维码
- **THEN** App 解析 URI 自动填入代理地址与配对码并发起连接

#### Scenario: 切换连接目标

- **WHEN** 已连接本机 daemon 的 App 改为配置代理服务器地址并连接
- **THEN** App 断开原连接并以代理服务器为新目标完成注册，不残留双连接
