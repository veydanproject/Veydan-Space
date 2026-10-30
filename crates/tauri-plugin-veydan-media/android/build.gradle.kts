// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "net.veydan.media"
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

dependencies {
    // FileProvider of the camera's shot, ContextCompat, ActivityCompat.
    implementation("androidx.core:core-ktx:1.13.1")
    // The contracts of the system's Photo Picker (PickVisualMedia); the
    // version the app brings.
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation(project(":tauri-android"))
    // The decisions of MediaRules.kt, on the JVM (src/test).
    testImplementation("junit:junit:4.13.2")
}
