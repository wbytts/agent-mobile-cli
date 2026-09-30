# agent-app Specification

## Purpose
提供安装在 Android 设备上的调试 App，作为 CLI 的设备端代理：维护与 daemon 的桥接连接，通过无障碍服务执行设备操作，并在脚本沙盒中运行下发的 JS 脚本。

## Requirements

### Requirement: daemon 连接管理

App SHALL 支持配置 daemon 地址（host:port）与配对码发起桥接连接，或扫码解析配对 URI 自动填入地址与配对码；连接后显示连接状态；连接断开后 SHALL 自动重连（已配对设备凭保存的 token 免配对）。

#### Scenario: 手动配置地址与配对码后连接成功

- **WHEN** 用户在 App 中输入主机、桥接端口与配对码并发起连接
- **THEN** App 显示已连接状态，daemon 侧出现对应设备注册

#### Scenario: 扫码快速配对

- **WHEN** 用户在 App 中扫码识别 `agent-mobile://pair?...` 二维码
- **THEN** App 解析 URI 自动填入地址与配对码并发起连接，无需手动输入

#### Scenario: 配对码错误提示

- **WHEN** App 提交的配对码错误或失效
- **THEN** App 显示认证失败原因并停留在连接页，不产生设备注册

### Requirement: 无障碍设备操作

App SHALL 通过无障碍服务执行点击、滑动与当前窗口 UI 树读取；无障碍权限未开启时 SHALL 提供跳转系统设置的引导并显示权限状态。

#### Scenario: 权限引导

- **WHEN** 无障碍权限未开启
- **THEN** App 显示权限状态与开启引导，可一键跳转系统无障碍设置页

#### Scenario: 开启权限后执行点击

- **WHEN** 无障碍权限已开启且收到点击命令
- **THEN** 设备上对应坐标的界面元素收到点击并产生响应

### Requirement: 脚本沙盒

App SHALL 内嵌 JS 引擎执行 daemon 下发的脚本，向脚本暴露 `mobile.*` 设备操作 API（点击、滑动、UI 树、截图等）；脚本不得访问沙盒外的设备资源，沙盒外能力仅限显式暴露的 API。

#### Scenario: 脚本调用设备 API

- **WHEN** 沙盒执行包含 `mobile.tap(x, y)` 的脚本
- **THEN** 设备对应位置收到点击，脚本继续执行并返回结果

### Requirement: 多页诊断界面

App SHALL 提供多页诊断界面：连接页（地址/配对码/扫码入口/连接状态）、能力自检页（无障碍权限状态与逐项能力自测）、日志页（最近命令与连接事件）。

#### Scenario: 三页切换与状态展示

- **WHEN** 用户在三个页面间切换
- **THEN** 各页正确渲染当前状态（连接状态、权限状态、日志列表）

#### Scenario: 能力自检

- **WHEN** 用户在能力自检页执行某项能力自测（如截图、UI 树读取）
- **THEN** App 在本地执行该项能力并展示成功或失败原因，无需经 daemon 下发命令

### Requirement: 屏幕截图回传

App SHALL 提供当前屏幕截图能力，并将图像数据经桥接连接回传 daemon。

#### Scenario: 截图命令返回图片

- **WHEN** daemon 向 App 下发截图命令
- **THEN** App 截取当前屏幕并回传 PNG 图像数据，CLI 保存为非空文件

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
