plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}
android {
    namespace = "org.sigil.compose"
    compileSdk = 36
    defaultConfig {
        applicationId = "org.sigil.compose"
        minSdk = 26
        targetSdk = 35
        versionCode = 4
        versionName = "0.1.0-alpha.3"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        resValue("string", "app_name", "Sigil")
        ndk { abiFilters += "arm64-v8a" }
    }
    buildTypes {
        debug { applicationIdSuffix = ".dev"; resValue("string", "app_name", "Sigil Development") }
        create("acceptance") { initWith(getByName("debug")); applicationIdSuffix = ".acceptance"; matchingFallbacks += "debug"; resValue("string", "app_name", "Sigil Acceptance") }
        release { isMinifyEnabled = true; proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt")) }
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    buildFeatures { compose = true }
    val acceptance = providers.gradleProperty("sigilAcceptance").isPresent
    testBuildType = if (acceptance) "acceptance" else "debug"
    sourceSets["main"].jniLibs.srcDir(if (acceptance) "build/rust-acceptance" else "build/rust")
    sourceSets["acceptance"].java.srcDir("src/debug/java")
    sourceSets["acceptance"].manifest.srcFile("src/debug/AndroidManifest.xml")
}
kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
dependencies {
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.browser:browser:1.9.0")
    implementation("androidx.compose.material3:material3:1.4.0")
    implementation("androidx.graphics:graphics-path:1.1.0")
    implementation("androidx.camera:camera-camera2:1.5.3")
    implementation("androidx.camera:camera-lifecycle:1.5.3")
    implementation("androidx.camera:camera-view:1.5.3")
    implementation("org.maplibre.gl:android-sdk:13.4.1")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.unifiedpush.android:connector:3.3.5")
    implementation(project(":shared"))
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.7.0")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4:1.9.4")
    androidTestImplementation("androidx.compose.foundation:foundation:1.9.4")
    debugImplementation("androidx.compose.ui:ui-test-manifest:1.9.4")
    add("acceptanceImplementation", "androidx.compose.ui:ui-test-manifest:1.9.4")
}
