# Spec Delta

## Purpose

提供安装在 Android 设备上的调试 App，作为 CLI 的设备端代理：维护与 daemon 的桥接连接，通过无障碍服务执行设备操作，并在脚本沙盒中运行下发的 JS 脚本。

## ADDED Requirements

### Requirement: daemon 连接管理

App SHALL 支持配置 daemon 地址（host:port），发起桥接连接并显示连接状态；连接断开后 SHALL 自动重连。

#### Scenario: 配置地址后连接成功

- **WHEN** 用户在 App 中输入主机与桥接端口并发起连接
- **THEN** App 显示已连接状态，daemon 侧出现对应设备注册

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

### Requirement: 屏幕截图回传

App SHALL 提供当前屏幕截图能力，并将图像数据经桥接连接回传 daemon。

#### Scenario: 截图命令返回图片

- **WHEN** daemon 向 App 下发截图命令
- **THEN** App 截取当前屏幕并回传 PNG 图像数据，CLI 保存为非空文件
