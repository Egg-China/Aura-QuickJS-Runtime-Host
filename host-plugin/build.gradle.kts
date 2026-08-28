import org.gradle.api.tasks.bundling.AbstractArchiveTask
import org.gradle.api.tasks.bundling.Zip

plugins { java }

repositories { mavenCentral() }

val auraJar = System.getenv("AURA_JAR")?.let(::file)
    ?: file("../.ci/aura/Aura-Launcher-27.1.dev-6d37f20-next.jar")
require(auraJar.isFile) { "Set AURA_JAR to the exact Aura Launcher Next Shadow JAR" }

dependencies {
    compileOnly(files(auraJar))
    testImplementation(files(auraJar))
    testImplementation(platform("org.junit:junit-bom:5.11.4"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

tasks.withType<JavaCompile>().configureEach { options.release.set(17) }
tasks.withType<Test>().configureEach { useJUnitPlatform() }
tasks.withType<AbstractArchiveTask>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}
tasks.jar { archiveBaseName.set("aura-quickjs-runtime-host-plugin") }

val processHost = providers.environmentVariable("AURA_QUICKJS_PROCESS_HOST")
val nativePlatform = providers.environmentVariable("AURA_QUICKJS_PLATFORM")

tasks.register<Zip>("packageNpl") {
    dependsOn(tasks.jar)
    archiveFileName.set("dev.hmclce.runtime.quickjs-host-v0.1.0-beta.1.npl")
    destinationDirectory.set(layout.buildDirectory.dir("npl"))
    from("plugin.json")
    into("libs") { from(tasks.jar) }
    into(nativePlatform.map { "native/$it" }) { from(processHost) }
    doFirst {
        val platform = nativePlatform.orNull ?: error("Set AURA_QUICKJS_PLATFORM")
        val process = processHost.orNull?.let(::file) ?: error("Set AURA_QUICKJS_PROCESS_HOST")
        require(platform in setOf(
            "windows-x64", "windows-arm64", "linux-x64", "linux-arm64", "macos-x64", "macos-arm64"
        )) { "Unsupported QuickJS Host platform: $platform" }
        val expected = if (platform.startsWith("windows-")) "aura-quickjs-host.exe" else "aura-quickjs-host"
        require(process.isFile) { "QuickJS process Host does not exist: $process" }
        require(process.name == expected) { "QuickJS process Host for $platform must be named $expected" }
    }
}
