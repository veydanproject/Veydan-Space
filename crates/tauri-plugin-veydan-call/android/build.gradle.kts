// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "net.veydan.call"
    compileSdk = 36

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

// The Java side of libwebrtc (livekit.org.webrtc.*): the audio device
// module and the codec factories the engine of calls (crates/messenger/rtc)
// reaches over JNI. It comes with the prebuilt archive that
// scripts/webrtc-toolchain.sh pins (VEYDAN_WEBRTC_DIR, exported by
// android-env.sh); the Rust side links the matching libwebrtc.a of the same
// tag. Without the jar the library builds, and the engine refuses to start
// on the phone instead of aborting (android.rs of the engine).
fun webrtcJar(): File? {
    val tag = "webrtc-89d790b"
    val dir = System.getenv("VEYDAN_WEBRTC_DIR")?.let { File(it) }
        ?: rootProject.file("../../../../data/toolchains/webrtc")
    val jar = File(dir, "$tag/android-arm64-release/libwebrtc.jar")
    if (!jar.isFile) {
        logger.warn("veydan-call: ${jar.path} is missing; calls will have no engine on the phone")
        return null
    }
    return jar
}

dependencies {
    // NotificationCompat.CallStyle, ServiceCompat.startForeground with types.
    implementation("androidx.core:core-ktx:1.13.1")
    implementation(project(":tauri-android"))
    webrtcJar()?.let { implementation(files(it)) }
    // The camera as frames (Camera.kt). The last line built with Kotlin
    // 1.9, which the project compiles with.
    implementation("androidx.camera:camera-core:1.3.4")
    implementation("androidx.camera:camera-camera2:1.3.4")
    implementation("androidx.camera:camera-lifecycle:1.3.4")
    // The lifecycle of the camera's own (the same version the app brings).
    implementation("androidx.lifecycle:lifecycle-runtime:2.10.0")
    // The decisions of CallRules.kt, on the JVM (src/test).
    testImplementation("junit:junit:4.13.2")
}
