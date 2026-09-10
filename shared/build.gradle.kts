plugins {
    id("org.jetbrains.kotlin.multiplatform")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.compose")
    id("com.android.library")
}
kotlin {
    androidTarget { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
    jvm("desktop")
    @OptIn(org.jetbrains.kotlin.gradle.ExperimentalWasmDsl::class)
    wasmJs { browser { commonWebpackConfig { sourceMaps = false } }; binaries.executable() }
    jvmToolchain(21)
    targets.withType<org.jetbrains.kotlin.gradle.targets.jvm.KotlinJvmTarget>().configureEach {
        compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
    }
    sourceSets {
        commonMain.dependencies {
            implementation(compose.components.resources)
            implementation(compose.material3)
            implementation(compose.foundation)
            implementation(compose.animation)
        }
        val jvmSharedMain by creating { dependsOn(commonMain.get()) }
        androidMain.get().dependsOn(jvmSharedMain)
        androidMain.dependencies { implementation("com.airbnb.android:lottie:6.7.1") }
        val desktopMain by getting {
            dependsOn(jvmSharedMain)
            dependencies { implementation(compose.desktop.currentOs) }
        }
        wasmJsMain.dependencies {
            implementation("org.jetbrains.kotlinx:kotlinx-browser:0.3")
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
        }
        wasmJsTest.dependencies { implementation(kotlin("test")) }
        val desktopTest by getting {
            dependencies {
                implementation(kotlin("test"))
                implementation(compose.desktop.uiTestJUnit4)
            }
        }
    }
}
tasks.withType<Test>().configureEach {
    systemProperty("java.library.path", "${rootProject.projectDir}/target/release")
}
val trimUnusedBrowserImport by tasks.registering {
    dependsOn("wasmJsProductionExecutableCompileSync")
    doLast {
        val generated = rootProject.layout.buildDirectory.dir("wasm/packages/Sigil-shared/kotlin").get().asFile
        val compiled = layout.buildDirectory.dir("compileSync/wasmJs/main/productionExecutable").get().asFile
        compiled.resolve("kotlin").listFiles()!!.filter { it.extension == "mjs" }.forEach {
            it.copyTo(generated.resolve(it.name), overwrite = true)
        }
        compiled.resolve("optimized/Sigil-shared.wasm")
            .copyTo(generated.resolve("Sigil-shared.wasm"), overwrite = true)
        val glue = generated.resolve("Sigil-shared.uninstantiated.mjs")
        val entry = generated.resolve("Sigil-shared.mjs")
        val text = glue.readText()
        val declaration = text.lineSequence().singleOrNull { it.contains("imports['@js-joda/core']") }
        if (declaration != null) {
            val variable = Regex("const (\\w+)").find(declaration)!!.groupValues[1]
            check(Regex("\\b$variable\\b").findAll(text).count() == 1) {
                "Browser date library is used; glue-only requirement needs a new decision"
            }
            glue.writeText(text.lineSequence().filterNot { it == declaration }.joinToString("\n"))
            entry.writeText(entry.readLines().filterNot { it.contains("@js-joda/core") }.joinToString("\n"))
        }
        check(!glue.readText().contains("@js-joda/core") && !entry.readText().contains("@js-joda/core")) {
            "Unexpected browser date-library import; audit generated glue before proceeding"
        }
        copy {
            from(rootProject.file("target/web")) { include("sigil_core.js", "sigil_core_bg.wasm") }
            into(generated)
        }
    }
}
tasks.named("wasmJsBrowserProductionWebpack") { dependsOn(trimUnusedBrowserImport) }
android {
    namespace = "org.sigil.shared"
    compileSdk = 36
    defaultConfig { minSdk = 26 }
    sourceSets["main"].assets.srcDir(rootProject.file("licenses"))
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
compose.desktop {
    application {
        mainClass = "org.sigil.MainKt"
        jvmArgs += "-Djava.library.path=${rootProject.projectDir}/target/release"
    }
}
