import java.util.zip.GZIPOutputStream

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
            implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
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
            from(rootProject.file("target/web")) { include("sigil_core.js", "sigil_core_bg.wasm", "sigil_browser.js", "sigil_browser_bg.wasm", "sigil_browser_events.js", "sigil_browser_events_bg.wasm", "sigil_materials.js", "sigil_materials_bg.wasm") }
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

tasks.named("wasmJsBrowserDistribution") {
    doLast {
        val destination = layout.buildDirectory.dir("dist/wasmJs/productionExecutable").get().asFile
        copy {
            from(rootProject.file("target/web")) { include("sigil_browser.js", "sigil_browser_bg.wasm", "sigil_browser_events.js", "sigil_browser_events_bg.wasm", "sigil_materials.js", "sigil_materials_bg.wasm") }
            into(destination)
        }
        destination.resolve("sigil-material-worker.mjs").writeText("import init, { material_worker_receive } from './sigil_materials.js'; const ready = init(); self.onmessage = async ({data}) => { await ready; material_worker_receive(data); };\n")
        destination.resolve("sigil-notifications.mjs").writeText("import init, { notification_push, notification_open } from './sigil_browser_events.js'; const ready = init(); self.addEventListener('push', event => event.waitUntil(ready.then(() => notification_push(event)))); self.addEventListener('notificationclick', event => event.waitUntil(ready.then(() => notification_open(event))));\n")
        destination.resolve("sigil-worker.mjs").writeText("import init, { worker_start } from './sigil_browser.js'; self.onmessage = async ({data}) => { self.onmessage = null; await init({module_or_path:data}); await worker_start(); };\n")
        destination.resolve("sigil-callback.mjs").writeText("import init, { complete_browser_auth } from './sigil_browser.js'; await init(); await complete_browser_auth();\n")
        copy { from(rootProject.file("licenses")); into(destination.resolve("licenses")) }
        destination.walkTopDown().filter { it.isFile && it.extension in setOf("wasm", "js", "mjs", "json", "ttf", "svg") }.toList().forEach { asset ->
            GZIPOutputStream(asset.resolveSibling(asset.name + ".gz").outputStream()).use { output -> asset.inputStream().use { it.copyTo(output) } }
        }
    }
}
