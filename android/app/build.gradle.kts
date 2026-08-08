plugins {
    // AGP 9 ships built-in Kotlin; only compiler plugins are applied explicitly.
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// The app version IS the cargo workspace version — parsed from the root
// Cargo.toml so even a bare `gradlew assemble` stays in sync; a
// -PversionName (CI) still wins.
val workspaceVersion: String = run {
    val toml = rootProject.projectDir.resolve("../Cargo.toml").readText()
    val pkg = toml.substringAfter("[workspace.package]")
    Regex("""version\s*=\s*"([^"]+)"""").find(pkg)?.groupValues?.get(1) ?: "0.0.0"
}

android {
    namespace = "org.fgsec.halogen"
    compileSdk = 36

    defaultConfig {
        applicationId = "org.fgsec.halogen"
        minSdk = 30
        targetSdk = 36
        // Overridden by CI (-PversionCode/-PversionName).
        versionCode = (project.findProperty("versionCode") as String?)?.toInt() ?: 1
        versionName = (project.findProperty("versionName") as String?) ?: workspaceVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            // No minification: F-Droid-friendly reproducible output; the app is small.
            isMinifyEnabled = false
            // Deliberate (release signing is deferred): debug-signed release
            // APKs — installable everywhere, no keystore to manage yet.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures { compose = true }

    sourceSets {
        getByName("main") {
            // Generated code (just android-core): typeshare wire DTOs + UniFFI bindings.
            kotlin.srcDir("../generated/wire")
            kotlin.srcDir("../generated/uniffi")
        }
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.activity.compose)
    implementation(libs.navigation.compose)
    implementation(libs.lifecycle.process)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.media3.exoplayer)
    implementation(libs.media3.session)
    implementation(libs.media3.datasource.okhttp)
    implementation(libs.coil.compose)
    implementation(libs.coil.network.okhttp)
    implementation(libs.okhttp)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.datetime)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.security.crypto)
    // UniFFI bindings load the Rust .so through JNA (aar packaging).
    implementation("${libs.jna.get()}@aar")

    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.uiautomator)
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
}
