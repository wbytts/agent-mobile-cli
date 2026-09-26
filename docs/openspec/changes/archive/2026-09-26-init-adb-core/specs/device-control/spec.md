# Spec Delta

## Purpose

让 Agent 对已连接的 Android 设备进行屏幕感知（UI 快照、截图、日志）与交互操作（触控、输入、应用管理），输出面向 Agent 优化的简化结构。

## ADDED Requirements

### Requirement: 屏幕 UI 快照

CLI SHALL 获取目标设备当前屏幕的 UI 层级，输出简化后的结构：为可交互元素分配稳定引用（如 `@e1`），并包含元素文本、类型与屏幕坐标区域。

#### Scenario: 快照包含元素引用

- **WHEN** 设备停留在包含可点击控件的页面，用户执行快照命令
- **THEN** 输出为简化结构，其中可交互元素带有引用标识、文本与坐标区域

### Requirement: 触控与文本输入

CLI SHALL 支持按屏幕坐标或元素引用执行点击、滑动，向焦点控件输入文本，以及发送按键事件。

#### Scenario: 按元素引用点击

- **WHEN** 用户对快照中的元素引用执行点击
- **THEN** 设备上对应位置的元素收到点击，界面产生相应响应

#### Scenario: 文本输入

- **WHEN** 焦点位于输入框时用户执行文本输入命令
- **THEN** 输入框中出现该文本

### Requirement: 屏幕截图

CLI SHALL 截取目标设备当前屏幕并保存为 PNG 文件，支持指定输出路径。

#### Scenario: 截图保存成功

- **WHEN** 用户执行截图命令并指定输出路径
- **THEN** 该路径下生成非空 PNG 文件且内容与设备当前屏幕一致

### Requirement: 应用管理

CLI SHALL 支持列出设备已安装应用（可按名称过滤），以及按包名启动、停止应用。

#### Scenario: 启动系统设置

- **WHEN** 用户执行启动命令指定系统设置包名
- **THEN** 设备前台切换到系统设置页面

### Requirement: 日志读取

CLI SHALL 支持读取设备 logcat 最近 N 行输出，并可按 tag 或级别过滤。

#### Scenario: 读取最近日志

- **WHEN** 用户请求最近 50 行日志
- **THEN** 输出不超过 50 行的日志内容

### Requirement: shell 命令透传

CLI SHALL 支持在目标设备上执行 shell 命令，返回其 stdout、stderr 与退出码。

#### Scenario: 查询设备型号

- **WHEN** 用户透传执行 `getprop ro.product.model`
- **THEN** 输出中包含设备型号字符串与退出码 0

### Requirement: 操作错误处理

当目标设备在操作过程中离线或操作超时时，CLI SHALL 返回结构化错误，不得挂起或无输出退出。

#### Scenario: 操作中设备离线

- **WHEN** 控制命令执行期间设备断开连接
- **THEN** 命令以设备离线的结构化错误结束
