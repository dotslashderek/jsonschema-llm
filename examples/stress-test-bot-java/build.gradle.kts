plugins {
    application
}

group = "com.jsonschema.llm.stress"
version = "1.0.0"

repositories {
    mavenCentral()
}

dependencies {
    implementation("com.jsonschema.llm:jsonschema-llm-java")
    implementation("com.fasterxml.jackson.core:jackson-databind:2.16.1")
    implementation("com.networknt:json-schema-validator:1.4.0")
    implementation("com.openai:openai-java:4.20.0")
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

application {
    mainClass.set("com.jsonschema.llm.stress.StressTestBot")
    // Enable native access for Panama FFM
    applicationDefaultJvmArgs = listOf("--enable-native-access=ALL-UNNAMED")
}

// Ensure native lib is built before running
val cargoBuild = tasks.register<Exec>("cargoBuild") {
    workingDir(rootProject.projectDir.resolve("../../"))
    commandLine("cargo", "build", "--release", "-p", "jsonschema-llm-java")
}

tasks.named("compileJava") {
    dependsOn(cargoBuild)
}
