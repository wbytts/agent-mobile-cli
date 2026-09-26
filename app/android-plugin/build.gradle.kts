plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.agentmobile.bridge"
    compileSdk = 36
    defaultConfig {
        minSdk = 24
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.appcompat:appcompat:1.7.1")
    // 扫码配对（zxing 相机扫码页，design.md 决策 8）
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}
