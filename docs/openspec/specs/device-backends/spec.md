# device-backends Specification

## Purpose
定义统一的设备模型与后端路由层，使设备控制命令不感知具体连接方式（ADB、桥接代理等），并首先提供可用的 ADB 直连后端。

## Requirements

### Requirement: 设备发现与枚举

CLI SHALL 枚举当前可达的 Android 设备，输出设备标识、型号、连接方式（USB/网络/桥接）与在线状态。

#### Scenario: 列出模拟器设备

- **WHEN** MuMu 模拟器已通过 adb 连接（`127.0.0.1:5555`）且用户执行设备枚举
- **THEN** 输出中包含该设备，状态为在线，连接方式为网络

### Requirement: adb 环境检测

当主机上 adb 不可用或无法连接 adb server 时，CLI SHALL 返回明确的错误信息与安装/修复指引，不得静默成功或异常崩溃。

#### Scenario: adb 缺失

- **WHEN** 主机 PATH 与常见安装位置均不存在 adb，用户执行设备枚举
- **THEN** 命令失败并输出指引性错误信息

### Requirement: 多设备目标选择

存在多个在线设备时，CLI SHALL 支持通过命令参数指定目标设备；未指定且在线设备多于一台时 SHALL 返回列出候选设备的歧义错误。

#### Scenario: 多设备未指定目标

- **WHEN** 两台设备同时在线且用户执行控制命令时未指定目标设备
- **THEN** 命令失败并输出候选设备列表

### Requirement: 网络设备主动连接

CLI SHALL 支持通过 `host:port` 主动连接网络 Android 设备。

#### Scenario: 连接网络设备

- **WHEN** 用户执行连接命令指定 `127.0.0.1:5555`
- **THEN** 该设备随后出现在设备枚举结果中

### Requirement: 后端路由抽象

设备控制命令 SHALL 经由统一的后端接口路由到目标设备所属后端执行；新增后端类型时控制命令的行为契约保持不变。

#### Scenario: 同一命令跨后端

- **WHEN** 用户对 ADB 后端设备执行点击命令
- **THEN** 命令按目标设备的后端类型路由执行，命令参数与输出结构与后端类型无关
