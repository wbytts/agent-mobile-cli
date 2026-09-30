#!/bin/sh
# 构建 Agent Mobile Bridge debug APK（universal APK，真机/模拟器通用）
set -e
cd "$(dirname "$0")/../app"
export ANDROID_HOME="$HOME/Library/Android/sdk"
export NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
# rquickjs-sys bindgen 需要 NDK sysroot 才能找到 stdio.h 等头文件（按目标分别注入，
# 与 CI release.yml 的 BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android 一致）。
SYSROOT="$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/sysroot"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--target=aarch64-linux-android24 --sysroot=$SYSROOT"
export BINDGEN_EXTRA_CLANG_ARGS_armv7_linux_androideabi="--target=armv7a-linux-androideabi24 --sysroot=$SYSROOT"
export BINDGEN_EXTRA_CLANG_ARGS_i686_linux_android="--target=i686-linux-android24 --sysroot=$SYSROOT"
export BINDGEN_EXTRA_CLANG_ARGS_x86_64_linux_android="--target=x86_64-linux-android24 --sysroot=$SYSROOT"
cargo tauri android build --debug "$@"
echo "APK: app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
