# Spec Delta

## Purpose

为 Agent 提供稳定的命令行入口与常驻 daemon 服务，避免每次命令重复初始化设备连接，并为设备端桥接代理预留接入端口；同时提供跨平台的 npm 安装分发。

## ADDED Requirements

### Requirement: 结构化命令输出

所有 CLI 命令 SHALL 以 JSON 输出结构化结果（成功时 `ok: true` 与结果负载，失败时 `ok: false` 与错误描述），失败时以非零退出码结束。

#### Scenario: 命令成功

- **WHEN** 用户执行任意读取类命令且执行成功
- **THEN** stdout 输出 `ok: true` 的 JSON，进程退出码为 0

#### Scenario: 命令失败

- **WHEN** 命令因目标设备离线而失败
- **THEN** stdout 输出 `ok: false` 与错误原因的 JSON，进程退出码非零

### Requirement: 常驻 daemon 会话复用

首个需要设备连接的命令 SHALL 自动启动常驻 daemon；后续命令 SHALL 通过 daemon 的 HTTP API 复用已建立的连接，不重复初始化。

#### Scenario: 连续命令复用连接

- **WHEN** 用户连续执行两条需要设备连接的命令
- **THEN** 第二条命令复用 daemon 已有连接，不重新执行设备连接初始化

### Requirement: daemon 启动锁

并发启动 daemon 时 SHALL 通过启动锁保证同一时刻只有一个 daemon 实例绑定服务端口。

#### Scenario: 并发首跑

- **WHEN** 两个 CLI 进程同时执行各自的首条命令
- **THEN** 只有一个 daemon 成功绑定端口，另一个进程转为复用该 daemon

### Requirement: 用户级配置文件

CLI SHALL 支持用户级配置文件自定义服务端口等参数；配置文件缺失时 SHALL 自动生成默认值。

#### Scenario: 配置文件缺失

- **WHEN** 用户删除配置文件后执行任意命令
- **THEN** CLI 自动重新生成包含默认端口配置的文件并按默认值运行

### Requirement: 桥接 WebSocket 监听

daemon SHALL 在独立于 HTTP API 的端口上监听 WebSocket 连接，供设备端桥接代理接入；该端口可在配置文件中修改。

#### Scenario: daemon 启动后端口监听

- **WHEN** daemon 完成启动
- **THEN** 配置的桥接 WS 端口处于监听状态并接受 WebSocket 握手

### Requirement: npm 安装分发

项目 SHALL 提供 npm 包，安装时按当前操作系统与架构定位对应的 CLI 预编译二进制。

#### Scenario: 全局安装后可执行

- **WHEN** 用户通过 npm 全局安装该包
- **THEN** `agent-mobile-cli --version` 可直接执行并输出版本号
