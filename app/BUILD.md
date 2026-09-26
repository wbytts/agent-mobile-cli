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
# rquickjs（QuickJS 沙盒）在 Android 目标构建期用 bindgen 生成绑定，需要 NDK sysroot
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--target=aarch64-linux-android24 --sysroot=$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/sysroot"
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

- `src/`：纯前端多页诊断 UI（vanilla JS tab 路由，无框架）；能力自检页已接 Rust 命令，连接/日志页待任务组 4
- `src-tauri/`：Tauri Rust core（`[workspace]` 空表隔离根仓库 workspace，根 crate 不受 app/ 影响）。
  `src/sandbox.rs` 为 QuickJS 脚本沙盒（rquickjs，host 可单测）；`src/mobile.rs` 为 Kotlin 桥实现
- `android-plugin/`：Kotlin 原生能力（Tauri 插件 `BridgePlugin` + `BridgeAccessibilityService`），
  独立 Android 库模块；因 `src-tauri/gen/` 被 .gitignore 排除，Kotlin 源码必须放这里，
  经 gen 工程以 `project(":android-plugin")` 并入构建（见踩坑记录 3）
- `src-tauri/icons/icon-src.png`：源图标，`cargo tauri icon icons/icon-src.png` 重新生成全套

## 无障碍能力说明

`android-plugin` 的 manifest 声明了无障碍服务（构建期 manifest merger 自动并入主 manifest）：
手势分发（tap/swipe）、窗口内容读取（uiTree，输出与 uiautomator dump 同构的 XML）、
截图（API 30+ takeScreenshot）、文本注入（input）与全局动作（key）。
真机/模拟器验收前需开启服务：

```sh
$ADB -s 127.0.0.1:5555 shell settings put secure enabled_accessibility_services \
  com.agentmobile.bridge/com.agentmobile.bridge.BridgeAccessibilityService
$ADB -s 127.0.0.1:5555 shell settings put secure accessibility_enabled 1
```

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
3. **Kotlin 源码不能放 gen/ 内**：`src-tauri/gen/` 被 .gitignore 排除，插件源码置于
   `app/android-plugin/`（独立 Android 库模块）。gen 工程侧有两处手工接线（重跑
   `cargo tauri android init` 后需重新应用）：
   - `gen/android/settings.gradle` 追加：
     `include ':android-plugin'` 与
     `project(':android-plugin').projectDir = new File(rootDir, '../../../android-plugin')`
   - `gen/android/app/build.gradle.kts` 的 dependencies 追加：
     `implementation(project(":android-plugin"))`
   服务声明与资源随库模块 manifest merger 自动并入，无需改 gen 的 AndroidManifest。
4. **rquickjs 版本与 Android 绑定**：rquickjs 0.14.0 的 `bindgen` feature 依赖未发布的
   rquickjs-macro 0.14.0（crates.io 打包缺陷）无法启用；且 0.12/0.14 均未预置
   `aarch64-linux-android` 绑定。故锁 0.12，并仅在
   `[target.'cfg(target_os = "android")'.dependencies]` 启用 `bindgen`（host 用预置绑定，
   Android 构建期生成，需要本机 libclang）。

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

## 验证记录（2026-09-26，任务组 3：Kotlin 原生能力 + QuickJS 沙盒）

- host：`cargo test --lib` 10/10 通过（沙盒：mobile.* 八函数桥接、返回值/异常回传、
  require/std/os 隔离）；`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 干净
- APK 重新构建成功（含 android-plugin 库模块与 rquickjs bindgen 交叉编译）
- MuMu 实机冒烟（开启无障碍服务后，经自检页按钮逐项验证，截图 /tmp/bridge-smoke*.png）：
  - 无障碍权限行显示真实状态「已开启」；「前往无障碍设置」跳转
    `AccessibilitySettingsActivity`（dumpsys mCurrentFocus 确认）
  - tap 成功（(720,1900) 点击）；swipe 成功（(540,1600)→(540,600) 300ms）
  - uiTree 成功（XML 10057 字符，uiautomator dump 同构）；screenshot 成功（PNG 约 145 KB）
  - key 成功（notifications 真实下拉通知栏，截图确认）；input 正确展示失败原因
    （自检页无可编辑控件，ACTION_SET_TEXT 被拒，属预期）
  - 冒烟期间 logcat 无 AndroidRuntime 崩溃
