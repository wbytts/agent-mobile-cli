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

（构建过程中补充）
