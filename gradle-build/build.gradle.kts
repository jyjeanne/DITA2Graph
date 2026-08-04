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
val sampleDocsInvalid = layout.projectDirectory.dir("../sample-docs-invalid")
val sampleDocsNested = layout.projectDirectory.dir("../sample-docs-nested")
val sampleDocsMapref = layout.projectDirectory.dir("../sample-docs-mapref")
val pluginDir = layout.projectDirectory.dir("../plugin/org.dita.dita2graph")

val downloadDitaOt = tasks.register<DitaOtDownloadTask>("downloadDitaOt") {
    version(ditaOtVersion)
}

// DitaOtInstallPluginTask's "local" install path needs a ZIP file, not a
// bare directory -- same as `dita install` itself (confirmed directly:
// "Failed to expand .../org.dita.dita2graph to .../plugin", the same
// error java.util.zip gives trying to read a directory as a zip stream).
// dita-ot-gradle's own "absolute path" wording for local installs means
// a path to a .zip, not a plugin directory (docs/dev/phase-0-findings.md
// finding 7).
val zipPlugin = tasks.register<Zip>("zipDita2GraphPlugin") {
    from(pluginDir)
    archiveFileName.set("org.dita.dita2graph.zip")
    destinationDirectory.set(layout.buildDirectory.dir("plugin-zip"))
    into("org.dita.dita2graph")
}

val installDita2Graph = tasks.register<DitaOtInstallPluginTask>("installDita2Graph") {
    dependsOn(downloadDitaOt, zipPlugin)
    ditaOtDir(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    plugins(zipPlugin.get().archiveFile.get().asFile.absolutePath)
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

// Deliberately NOT a dependency of validateDocs/buildKnowledgeGraph --
// this task is expected to FAIL. It's the Phase 4 exit criterion
// (docs/plugin-specification.md §12) made concrete: proof that DITA-OT's
// own validation rejects broken source before dita2graph ever runs, not
// just a claim. See ../sample-docs-invalid/README.md for which kind of
// "broken" this actually is (an unresolvable href, not an unresolvable
// keyref -- the latter is only informational and does not fail).
val validateBrokenDoc = tasks.register<DitaOtValidateTask>("validateBrokenDoc") {
    dependsOn(downloadDitaOt)
    ditaOtDir(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocsInvalid.file("broken.ditamap"))
}

val buildKnowledgeGraph = tasks.register<DitaOtTask>("buildKnowledgeGraph") {
    dependsOn(installDita2Graph, validateDocs, checkLinks)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocs.file("user-guide.ditamap"))
    output(layout.buildDirectory.dir("dita2graph").get().asFile.path)
    transtype("dita2graph")
    progressStyle("DETAILED")
}

// The public/internal DITAVAL split (docs/plugin-specification.md §6.1):
// topics/internal-notes.dita's topicref carries audience="internal" in
// user-guide.ditamap, so filtering it out at preprocessing time (not as
// an MCP-server-side check, per §6.1's "a topic that was never extracted
// cannot leak") means it never reaches dita2graph's extraction step and
// no okf/topics/internal-notes.md is written -- confirmed directly by
// running both profiles and diffing the resulting bundles (§12 Phase 5).
val buildKnowledgeGraphPublic = tasks.register<DitaOtTask>("buildKnowledgeGraphPublic") {
    dependsOn(installDita2Graph, validateDocs, checkLinks)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocs.file("user-guide.ditamap"))
    filter(sampleDocs.file("public.ditaval"))
    output(layout.buildDirectory.dir("dita2graph-public").get().asFile.path)
    transtype("dita2graph")
    progressStyle("DETAILED")
}

val buildKnowledgeGraphInternal = tasks.register<DitaOtTask>("buildKnowledgeGraphInternal") {
    dependsOn(installDita2Graph, validateDocs, checkLinks)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocs.file("user-guide.ditamap"))
    filter(sampleDocs.file("internal.ditaval"))
    output(layout.buildDirectory.dir("dita2graph-internal").get().asFile.path)
    transtype("dita2graph")
    progressStyle("DETAILED")
}

// Nested topicref/topichead/topicgroup map-structure guarantee (§3.3,
// §12 Phase 1 status) and the related-links exclusion it depends on --
// see ../sample-docs-nested/README.md for what a genuinely nested map
// makes DITA-OT auto-generate that a flat map like sample-docs/ never
// does, and why that would silently corrupt the graph without the fix.
val buildKnowledgeGraphNested = tasks.register<DitaOtTask>("buildKnowledgeGraphNested") {
    dependsOn(installDita2Graph)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocsNested.file("user-guide.ditamap"))
    output(layout.buildDirectory.dir("dita2graph-nested").get().asFile.path)
    transtype("dita2graph")
    progressStyle("DETAILED")
}

// mapref/anchorref map composition (§3.3, finding 14) -- see
// ../sample-docs-mapref/README.md for what's actually confirmed to work
// (both, via DITA-OT's own mapref preprocessing step) versus navref
// (not supported at all, confirmed separately, no fixture needed since
// nothing happens for it to regress).
val buildKnowledgeGraphMapref = tasks.register<DitaOtTask>("buildKnowledgeGraphMapref") {
    dependsOn(installDita2Graph)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-$ditaOtVersion"))
    input(sampleDocsMapref.file("user-guide.ditamap"))
    output(layout.buildDirectory.dir("dita2graph-mapref").get().asFile.path)
    transtype("dita2graph")
    progressStyle("DETAILED")
}
