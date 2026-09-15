import java.util.zip.GZIPOutputStream
import java.security.MessageDigest

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
fun webRustTask(name: String, directory: String, roots: List<String>, modules: List<String>, packages: List<String> = emptyList(), fonts: Boolean = false) = tasks.register(name) {
    inputs.files(roots.map { root -> rootProject.fileTree(root) {
        include("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain*", ".cargo/**", "src/**", "assets/**")
        exclude("**/target/**", "**/build/**", "**/.git/**")
    } })
    if (fonts) inputs.dir(rootProject.file("shared/src/commonMain/composeResources/font"))
    val cargo = listOf("cargo", "build", "--locked", "--release", "--target", "wasm32-unknown-unknown", "--lib") + packages.flatMap { listOf("-p", it) }
    inputs.property("cargoCommand", cargo)
    listOf("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS").forEach { key -> inputs.property(key, providers.environmentVariable(key).orElse("")) }
    inputs.property("rustVersion", providers.exec { workingDir(rootProject.file(directory)); commandLine("rustc", "--version") }.standardOutput.asText)
    inputs.property("bindingsVersion", providers.exec { commandLine("wasm-bindgen", "--version") }.standardOutput.asText)
    outputs.files(modules.flatMap { listOf(rootProject.file("target/web/$it.js"), rootProject.file("target/web/${it}_bg.wasm")) })
    doLast {
        fun run(command: List<String>) {
            val execution = providers.exec {
                workingDir(rootProject.file(directory))
                environment("CFLAGS_wasm32_unknown_unknown", "-std=gnu2x")
                environment("CARGO_TARGET_DIR", rootProject.file("$directory/target").absolutePath)
                commandLine(command)
                isIgnoreExitValue = true
            }
            val result = execution.result.get()
            execution.standardOutput.asText.get().takeIf { it.isNotBlank() }?.let { logger.lifecycle(it.trimEnd()) }
            execution.standardError.asText.get().takeIf { it.isNotBlank() }?.let { logger.lifecycle(it.trimEnd()) }
            result.assertNormalExitValue()
        }
        run(cargo)
        modules.forEach { module -> run(listOf("wasm-bindgen", "target/wasm32-unknown-unknown/release/$module.wasm", "--target", "web", "--out-dir", rootProject.file("target/web").absolutePath)) }
    }
}
val buildWebClient = webRustTask("buildWebClient", ".", listOf(".", "core", "crypto", "protocol", "client", "android", "browser", "browser-events", "text", "media", "maps", "calls", "server", "vendor/ece-native"), listOf("sigil_core", "sigil_browser", "sigil_browser_events"), listOf("sigil-core", "sigil-browser", "sigil-browser-events"))
val buildWebMaterials = webRustTask("buildWebMaterials", "materials", listOf("materials"), listOf("sigil_materials"), fonts = true)
val buildWebMaps = webRustTask("buildWebMaps", "browser-maps", listOf("browser-maps"), listOf("sigil_browser_maps"), fonts = true)
val buildWebAudio = webRustTask("buildWebAudio", "browser-audio", listOf("browser-audio"), listOf("sigil_browser_audio"))
val buildWebNativeModules = tasks.register("buildWebNativeModules") { dependsOn(buildWebClient, buildWebMaterials, buildWebMaps, buildWebAudio) }
val trimUnusedBrowserImport by tasks.registering {
    dependsOn(buildWebNativeModules)
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
            from(rootProject.file("target/web")) { include("sigil_core.js", "sigil_core_bg.wasm", "sigil_browser.js", "sigil_browser_bg.wasm", "sigil_browser_events.js", "sigil_browser_events_bg.wasm", "sigil_browser_audio.js", "sigil_browser_audio_bg.wasm", "sigil_materials.js", "sigil_materials_bg.wasm", "sigil_browser_maps.js", "sigil_browser_maps_bg.wasm") }
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
            from(rootProject.file("target/web")) { include("sigil_browser.js", "sigil_browser_bg.wasm", "sigil_browser_events.js", "sigil_browser_events_bg.wasm", "sigil_browser_audio.js", "sigil_browser_audio_bg.wasm", "sigil_materials.js", "sigil_materials_bg.wasm", "sigil_browser_maps.js", "sigil_browser_maps_bg.wasm") }
            into(destination)
        }
        fun digest(bytes:ByteArray)=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it)}
        val audioWasm=destination.resolve("sigil_browser_audio_bg.wasm")
        val audioWasmName="audio-${digest(audioWasm.readBytes())}.wasm"
        audioWasm.copyTo(destination.resolve(audioWasmName),overwrite=true)
        audioWasm.delete()
        val audioModule=destination.resolve("sigil_browser_audio.js")
        val audioSource=audioModule.readText().replace("sigil_browser_audio_bg.wasm",audioWasmName)
        val audioModuleName="audio-${digest(audioSource.toByteArray())}.mjs"
        destination.resolve(audioModuleName).writeText(audioSource)
        audioModule.writeText("export * from './$audioModuleName'; export { default } from './$audioModuleName';\n")
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

val browserTestNativeModules by tasks.registering(Copy::class) {
    dependsOn("wasmJsTestTestDevelopmentExecutableCompileSync", buildWebNativeModules)
    from(rootProject.file("target/web")) { include("sigil_*.js", "sigil_*_bg.wasm") }
    into(rootProject.layout.buildDirectory.dir("wasm/packages/Sigil-shared-test/kotlin"))
}
tasks.named("wasmJsBrowserTest") { dependsOn(browserTestNativeModules) }
