#!/bin/sh
# 构建 Agent Mobile Bridge debug APK（aarch64 真机/模拟器通用 universal APK）
set -e
cd "$(dirname "$0")/../app"
export ANDROID_HOME="$HOME/Library/Android/sdk"
export NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
cargo tauri android build --debug "$@"
echo "APK: app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
