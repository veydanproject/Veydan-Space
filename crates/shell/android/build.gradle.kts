// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The Android library of the shell (docs/platform-spec.md 13.4): what every
// product's MainActivity does, in one place. A product includes it from its
// gen/android/settings.gradle and app/build.gradle.kts (`:veydan-shell`).
// The library builds into the gen/android/build/ of the product that
// includes it, so two products never share an output.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

layout.buildDirectory.set(rootProject.layout.buildDirectory.dir("veydan-shell"))

android {
    namespace = "net.veydan.shell"
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
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("androidx.webkit:webkit:1.14.0")
}
