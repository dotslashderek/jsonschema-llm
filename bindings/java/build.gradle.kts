plugins {
    `java-library`
    `maven-publish`
}

group = "com.jsonschema.llm"
version = "0.1.0-ALPHA"

repositories {
    mavenCentral()
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

dependencies {
    implementation("com.dylibso.chicory:runtime:0.0.12")
    implementation("com.dylibso.chicory:wasi:0.0.12")
    implementation("com.fasterxml.jackson.core:jackson-databind:2.16.1")

    testImplementation(platform("org.junit:junit-bom:5.10.1"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

// ---------------------------------------------------------------------------
// WASM binary embedding
// ---------------------------------------------------------------------------

val wasmSource = file("../../target/wasm32-wasip1/release/json_schema_llm_wasi.wasm")
val wasmGeneratedDir = layout.buildDirectory.dir("generated/resources")

val embedWasm by tasks.registering(Copy::class) {
    description = "Embeds the compiled WASM binary into the JAR resources."
    doFirst {
        if (!wasmSource.exists()) {
            throw GradleException(
                "Rust WASM binary not found at ${wasmSource.absolutePath}.\n" +
                "Build it first: cargo build --target wasm32-wasip1 --release -p json-schema-llm-wasi"
            )
        }
    }
    from(wasmSource)
    into(wasmGeneratedDir.map { it.dir("wasm") })
}

sourceSets.main {
    resources.srcDir(wasmGeneratedDir)
}

tasks.processResources {
    dependsOn(embedWasm)
}

// ---------------------------------------------------------------------------
// Maven publishing
// ---------------------------------------------------------------------------

publishing {
    publications {
        create<MavenPublication>("mavenJava") {
            from(components["java"])
        }
    }
}

// ---------------------------------------------------------------------------
// Test configuration
// ---------------------------------------------------------------------------

tasks.test {
    useJUnitPlatform()
    environment("JSL_WASM_PATH",
        file("../../target/wasm32-wasip1/release/json_schema_llm_wasi.wasm").absolutePath)
}
