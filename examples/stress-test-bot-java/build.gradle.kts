plugins {
    application
}

group = "com.jsonschema.llm.stress"
version = "1.0.0"

repositories {
    mavenLocal()
    mavenCentral()
}

dependencies {
    // WASI wrapper (published to mavenLocal via bindings/java)
    implementation("com.jsonschema.llm:json-schema-llm-java:0.1.0-ALPHA")
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
}
