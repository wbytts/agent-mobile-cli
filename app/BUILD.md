# Agent Mobile Bridge 构建说明

Tauri 2 Android 工程，应用名 `Agent Mobile Bridge`，标识符 `com.agentmobile.bridge`。

## 锁定工具链版本

| 工具 | 版本 |
| --- | --- |
| tauri-cli | 2.11.5（`cargo install tauri-cli --locked --version '^2'`） |
| rustc / cargo | 1.93.1 |
| Rust target | `aarch64-linux-android`（`rustup target add aarch64-linux-android`） |
| Android NDK | 27.1.12297006（`~/Library/Android/sdk/ndk/27.1.12297006`） |
| Android SDK | `~/Library/Android/sdk`（platforms + build-tools） |
| JDK | OpenJDK 17.0.18 |
| 验证设备 | MuMu 模拟器 `127.0.0.1:5555`（Android 12, arm64-v8a） |

## 环境变量

构建 Android 前需要：

```sh
export ANDROID_HOME="$HOME/Library/Android/sdk"
export NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
```

## 构建命令

```sh
cd app
cargo tauri android init        # 首次，生成 gen/android Gradle 工程
cargo tauri android build --debug   # 产出 aarch64 debug APK
```

APK 输出路径：`src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`。

## 安装与启动（MuMu）

```sh
ADB=~/Library/Android/sdk/platform-tools/adb
$ADB -s 127.0.0.1:5555 install -r <apk>
$ADB -s 127.0.0.1:5555 shell monkey -p com.agentmobile.bridge 1
```

## 工程结构

- `src/`：纯前端多页诊断 UI（vanilla JS tab 路由，无框架），状态为 mock，待接 Rust 命令
- `src-tauri/`：Tauri Rust core（`[workspace]` 空表隔离根仓库 workspace，根 crate 不受 app/ 影响）
- `src-tauri/icons/icon-src.png`：源图标，`cargo tauri icon icons/icon-src.png` 重新生成全套

## 踩坑记录

1. **Gradle wrapper 下载超时**：首次 `android build` 卡在 gradle-wrapper 从
   `services.gradle.org` 下载 `gradle-8.14.3-bin.zip`（read timeout）。
   解法：`src-tauri/gen/android/gradle/wrapper/gradle-wrapper.properties` 的
   `distributionUrl` 改为腾讯镜像
   `https://mirrors.cloud.tencent.com/gradle/gradle-8.14.3-bin.zip`，并删除半截缓存
   `rm -rf ~/.gradle/wrapper/dists/gradle-8.14.3-bin`（残留 partial 会让 wrapper 卡死重下）。
   注意 `gen/` 由 `cargo tauri android init` 生成，重跑 init 后需重新修改。
2. **首次 `android init` 超时中断**：init 会顺带安装 armv7/i686/x86_64 三个额外
   Rust target，下载较慢；被 300s 超时打断后重跑即可（幂等续装）。

## 验证记录（2026-09-26）

- `cargo tauri android build --debug --target aarch64` 成功，APK 115MB：
  `app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`
- MuMu（127.0.0.1:5555）`install -r` 成功，`monkey -p com.agentmobile.bridge 1` 启动，
  logcat 显示 `Displayed com.agentmobile.bridge/.MainActivity: +412ms`，WebView 95 加载正常，
  全程无 FATAL；进程在三页切换后保持存活
- 三页截图证据（/tmp）：`app-page-connect.png`（连接页）、`app-page-caps.png`（能力自检页）、
  `app-page-logs.png`（日志页，首条为 tap 自测「待接线」交互产生）
- workspace 隔离验证：根仓库 `cargo metadata` 仅含 `agent-mobile-cli`；
  根仓库 `cargo clippy --all-targets -- -D warnings` 与 `cargo test`（54 passed）不受 app/ 影响
