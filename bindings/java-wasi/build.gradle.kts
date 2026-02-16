plugins {
    `java-library`
}

group = "com.jsonschema.llm"
version = "0.1.0"

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

tasks.test {
    useJUnitPlatform()
    environment("JSL_WASM_PATH",
        file("../../target/wasm32-wasip1/release/jsonschema_llm_wasi.wasm").absolutePath)
}
