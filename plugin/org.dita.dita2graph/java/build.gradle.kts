// Builds lib/dita2graph-core.jar (docs/plugin-specification.md §2.1) --
// the Java side of the DITA-OT plugin, kept in its own small Gradle
// project so it builds independently of gradle-build/ (which only
// *consumes* the jar this produces) and of the Rust workspace at the
// repo root.
//
// Compiled at --release 21, not the spec's Java 25 floor (§1.1): this
// sandbox only has JDK 21 available and Gradle's toolchain
// auto-provisioning can't reach api.foojay.io to fetch a JDK 25 (same
// constraint already documented in gradle-build/README.md). Bump this
// once a real JDK 25 is available -- there's nothing in the source that
// depends on 21 specifically.

plugins {
    java
}

repositories {
    mavenCentral()
}

dependencies {
    // Provided by DITA-OT's own Ant runtime at execution time (loaded
    // via plugin.xml's dita.conductor.lib.import feature putting this
    // jar on Ant's classpath, where org.apache.tools.ant.* is already
    // present) -- compileOnly, not bundled into the jar. Version matches
    // DITA-OT 4.4's own bundled Ant exactly (lib/ant-apache-resolver-1.10.15.jar).
    compileOnly("org.apache.ant:ant:1.10.15")

    testImplementation(platform("org.junit:junit-bom:5.11.0"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

tasks.withType<JavaCompile> {
    options.release.set(21)
}

tasks.test {
    useJUnitPlatform()
}

tasks.jar {
    archiveFileName.set("dita2graph-core.jar")
    destinationDirectory.set(layout.projectDirectory.dir("../lib"))
}
