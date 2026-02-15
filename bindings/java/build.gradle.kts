plugins {
    `java-library`
}

group = "com.jsonschema.llm"
version = "0.1.0-SNAPSHOT"

repositories {
    mavenCentral()
}

dependencies {
    implementation("com.fasterxml.jackson.core:jackson-databind:2.16.1")
    testImplementation(platform("org.junit:junit-bom:5.10.2"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

// PanamaBinding requires java.lang.foreign.* (Java 22+).
// On JDK <22 we exclude it from compilation; at runtime JsonSchemaLlm
// catches the ClassNotFoundException and falls back to JNI.
val hasPanama = JavaVersion.current() >= JavaVersion.VERSION_22
if (!hasPanama) {
    sourceSets.main {
        java.exclude("**/PanamaBinding.java")
    }
}

tasks.test {
    useJUnitPlatform()
    // Enable native access for Panama FFM
    jvmArgs("--enable-native-access=ALL-UNNAMED")
    // Panama tests require JDK 22+
    enabled = hasPanama
}

val testJni = tasks.register<Test>("testJni") {
    useJUnitPlatform()
    // Only add --enable-native-access on JDK 22+ (JNI doesn't need it, JDK 17 doesn't support it)
    if (hasPanama) {
        jvmArgs("--enable-native-access=ALL-UNNAMED")
    }
    systemProperty("com.jsonschema.llm.forceJni", "true")
    testClassesDirs = sourceSets["test"].output.classesDirs
    classpath = sourceSets["test"].runtimeClasspath
}

tasks.check {
    dependsOn(testJni)
}

val osName = System.getProperty("os.name").lowercase()
val osArch = System.getProperty("os.arch").lowercase()

val targetOs = when {
    osName.contains("mac") -> "darwin"
    osName.contains("win") -> "windows"
    osName.contains("nux") -> "linux"
    else -> throw GradleException("Unsupported OS: $osName")
}

val targetArch = when {
    osArch == "amd64" || osArch == "x86_64" -> "x86_64"
    osArch == "aarch64" || osArch == "arm64" -> "aarch64"
    else -> throw GradleException("Unsupported Arch: $osArch")
}

val libName = when (targetOs) {
    "windows" -> "jsonschema_llm_java.dll"
    "darwin" -> "libjsonschema_llm_java.dylib"
    else -> "libjsonschema_llm_java.so"
}

val cargoBuild = tasks.register<Exec>("cargoBuild") {
    workingDir(rootProject.projectDir.parentFile.parentFile)
    commandLine("cargo", "build", "--release", "-p", "jsonschema-llm-java")
}

val copyNativeLib = tasks.register<Copy>("copyNativeLib") {
    dependsOn(cargoBuild)
    from(rootProject.projectDir.parentFile.parentFile.resolve("target/release/$libName"))
    into(layout.projectDirectory.dir("src/main/resources/native/$targetOs-$targetArch"))
}

tasks.processResources {
    dependsOn(copyNativeLib)
}
