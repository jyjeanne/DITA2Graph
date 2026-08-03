import com.github.jyjeanne.DitaOtDownloadTask
import com.github.jyjeanne.DitaOtInstallPluginTask
import com.github.jyjeanne.DitaOtValidateTask
import com.github.jyjeanne.DitaLinkCheckTask
import com.github.jyjeanne.DitaOtTask

// Real, runnable version of docs/plugin-specification.md §8.2's example,
// pointed at this repo's own sample-docs/ and plugin/org.dita.dita2graph
// -- the integration harness for Phase 0 finding 4
// (docs/dev/phase-0-findings.md): does a live DITA-OT run actually
// resolve sample-docs/ the way §2.2/§3.2 assume?
//
// Property-setter calls below were corrected against dita-ot-gradle's
// actual Kotlin source (fetched from GitHub, not guessed) after the
// spec's §8.2 example failed to compile as originally written -- see
// docs/dev/phase-0-findings.md finding 5 for what was wrong and why.

plugins {
    id("io.github.jyjeanne.dita-ot-gradle") version "2.8.6"
}

val ditaOtVersion = "4.4"
val sampleDocs = layout.projectDirectory.dir("../sample-docs")
val pluginDir = layout.projectDirectory.dir("../plugin/org.dita.dita2graph")

val downloadDitaOt = tasks.register<DitaOtDownloadTask>("downloadDitaOt") {
    version(ditaOtVersion)
}

val installDita2Graph = tasks.register<DitaOtInstallPluginTask>("installDita2Graph") {
    dependsOn(downloadDitaOt)
    ditaOtDir(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    plugins(pluginDir.asFile.absolutePath)
    force.set(true)
}

val validateDocs = tasks.register<DitaOtValidateTask>("validateDocs") {
    dependsOn(downloadDitaOt)
    ditaOtDir(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocs.file("user-guide.ditamap"))
}

// DitaLinkCheckTask has no ditaOtDir/DITA-OT dependency at all -- it's a
// pure Kotlin XML link scanner, confirmed from its source.
val checkLinks = tasks.register<DitaLinkCheckTask>("checkLinks") {
    input(sampleDocs.file("user-guide.ditamap"))
}

val buildKnowledgeGraph = tasks.register<DitaOtTask>("buildKnowledgeGraph") {
    dependsOn(installDita2Graph, validateDocs, checkLinks)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocs.file("user-guide.ditamap"))
    output(layout.buildDirectory.dir("dita2graph").get().asFile.path)
    transtype("dita2graph")
    progressStyle("DETAILED")
}
