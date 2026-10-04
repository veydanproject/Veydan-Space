// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import groovy.json.JsonSlurper

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

// Firebase reads its settings from string resources. The usual way to make
// them is the google-services plugin applied to the app module; here they
// are made from the same file, in this library, so that the app module
// stays as it is and a build without this plugin has no trace of Firebase.
//
// No file: the library builds, and the app reports that pushes are not set up.
fun firebaseSettings(): Map<String, String> {
    val file = rootProject.file("app/google-services.json")
    if (!file.exists()) {
        logger.lifecycle("veydan-push: no app/google-services.json, pushes will be unavailable")
        return emptyMap()
    }
    val appId = (rootProject.findProject(":app")?.extensions?.findByName("android")
        as? com.android.build.api.dsl.ApplicationExtension)?.defaultConfig?.applicationId

    @Suppress("UNCHECKED_CAST")
    val json = JsonSlurper().parse(file) as Map<String, Any?>
    val project = json["project_info"] as? Map<String, Any?> ?: emptyMap()
    val clients = json["client"] as? List<Map<String, Any?>> ?: emptyList()
    fun packageOf(client: Map<String, Any?>): String? {
        val info = client["client_info"] as? Map<String, Any?>
        val android = info?.get("android_client_info") as? Map<String, Any?>
        return android?.get("package_name") as? String
    }
    // The client of this very product, or an error: the file of another
    // product (Space's copied into Chat) would set Firebase up with the
    // wrong app, so a single client of another package is not a fallback.
    val client = clients.firstOrNull { appId != null && packageOf(it) == appId }
        ?: throw GradleException(
            "veydan-push: ${file.path} has no client for package $appId " +
                "(it names ${clients.mapNotNull(::packageOf)}); add an Android app " +
                "with this package to the Firebase project and download its file"
        )
    val info = client["client_info"] as? Map<String, Any?> ?: emptyMap()
    val keys = client["api_key"] as? List<Map<String, Any?>> ?: emptyList()

    val out = mutableMapOf<String, String>()
    fun put(name: String, value: Any?) {
        val text = value as? String
        if (!text.isNullOrEmpty()) out[name] = text
    }
    put("google_app_id", info["mobilesdk_app_id"])
    put("gcm_defaultSenderId", project["project_number"])
    put("project_id", project["project_id"])
    put("google_storage_bucket", project["storage_bucket"])
    put("google_api_key", keys.firstOrNull()?.get("current_key"))
    if (!out.containsKey("google_app_id") || !out.containsKey("google_api_key")) {
        throw GradleException("veydan-push: app/google-services.json lacks the app id or the api key")
    }
    return out
}

// The library of the product the handler of a push loads without the app
// (Core.kt): `[lib] name` of the product's Cargo.toml, the one place the
// name exists (docs/platform-spec.md 13.4). The root project is the
// product's gen/android, so the crate is two levels up. A build of this
// library alone (its own settings.gradle) names the library with
// -PveydanCoreLibrary=<name>.
fun coreLibraryName(): String {
    (findProperty("veydanCoreLibrary") as? String)?.let { return it }
    val cargo = rootProject.file("../../Cargo.toml")
    if (!cargo.exists()) {
        throw GradleException("veydan-push: ${cargo.path} not found; the library name comes from the product's [lib] name")
    }
    var section = ""
    var packageName: String? = null
    var libName: String? = null
    for (raw in cargo.readLines()) {
        val line = raw.substringBefore('#').trim()
        if (line.startsWith("[")) {
            section = line
            continue
        }
        val m = Regex("""^name\s*=\s*"([^"]+)"""").find(line) ?: continue
        when (section) {
            "[package]" -> packageName = m.groupValues[1]
            "[lib]" -> libName = m.groupValues[1]
        }
    }
    return libName
        ?: packageName?.replace('-', '_')
        ?: throw GradleException("veydan-push: ${cargo.path} has neither [lib] name nor [package] name")
}

android {
    namespace = "net.veydan.push"
    compileSdk = 36

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
        resValue("string", "veydan_core_library", coreLibraryName())
        for ((name, value) in firebaseSettings()) {
            resValue("string", name, value)
        }
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
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    // The same version lifecycle-process brings along.
    implementation("androidx.startup:startup-runtime:1.1.1")
    // The last line of releases built with Kotlin 2.0; the project compiles
    // with Kotlin 1.9, which reads metadata up to 2.0 and no further.
    implementation("com.google.firebase:firebase-messaging:24.1.2")
    // Named for the check "are Google services on this phone".
    implementation("com.google.android.gms:play-services-base:18.5.0")
    implementation(project(":tauri-android"))
}
