package org.dita.dita2graph.tasks;

import org.apache.tools.ant.BuildException;
import org.apache.tools.ant.Project;
import org.apache.tools.ant.Task;

import java.io.BufferedReader;
import java.io.File;
import java.io.FileWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.List;

/**
 * The {@code dita2graph:extract} Ant task (docs/plugin-specification.md
 * §2.1/§3.1, wired up in {@code build.xml}): reads DITA-OT's resolved
 * temp-directory job data, builds the normalized DITA model (§3.2) via
 * {@link DitaModelExtractor}, and shells out to the {@code dita2graph-core}
 * Rust binary (§3.4) to write the OKF bundle.
 *
 * <p>{@code emitGraphJson} is forwarded to the Rust core's
 * {@code --emit-graph-json} flag. {@code depth}/{@code mcp} are still
 * only accepted and logged, not yet passed through: {@code depth}
 * ("max relationship traversal depth captured in the graph", §2.3)
 * has no well-defined meaning against today's flat, non-recursive
 * extraction (deep/nested {@code topicref}/{@code topichead}/{@code
 * topicgroup} aren't walked at all yet, {@link DitaModelExtractor});
 * {@code mcp} would write an {@code mcp/mcp-server.toml} that
 * {@code dita2graph-mcp} doesn't read yet (it takes a bundle path
 * argument directly, no {@code --config}, §5.4). Both are honest gaps,
 * not silently dropped -- wiring them needs those blockers resolved
 * first, not just a CLI flag added here.
 */
public class ExtractTask extends Task {

    private String tempDir;
    private String outputDir;
    private String depth;
    private String emitGraphJson;
    private String mcp;
    private String store;
    private String includeDrafts;

    public void setTempDir(String tempDir) {
        this.tempDir = tempDir;
    }

    public void setOutputDir(String outputDir) {
        this.outputDir = outputDir;
    }

    public void setDepth(String depth) {
        this.depth = depth;
    }

    public void setEmitGraphJson(String emitGraphJson) {
        this.emitGraphJson = emitGraphJson;
    }

    public void setMcp(String mcp) {
        this.mcp = mcp;
    }

    public void setStore(String store) {
        this.store = store;
    }

    public void setIncludeDrafts(String includeDrafts) {
        this.includeDrafts = includeDrafts;
    }

    @Override
    public void execute() throws BuildException {
        require(tempDir, "tempDir");
        require(outputDir, "outputDir");

        File tempDirFile = new File(tempDir);
        if (!tempDirFile.isDirectory()) {
            throw new BuildException("dita2graph:extract: tempDir does not exist: " + tempDirFile);
        }

        boolean drafts = Boolean.parseBoolean(includeDrafts);
        log("dita2graph:extract: depth=" + depth + " mcp=" + mcp
                + " (accepted, not yet wired to dita2graph-core, §12) includeDrafts=" + drafts, Project.MSG_VERBOSE);

        List<Object> nodes;
        try {
            DitaModelExtractor extractor = new DitaModelExtractor(
                    tempDirFile,
                    drafts,
                    msg -> log(msg, Project.MSG_WARN),
                    msg -> log(msg, Project.MSG_INFO));
            nodes = extractor.extract();
        } catch (Exception e) {
            throw new BuildException("dita2graph:extract: failed to build the normalized DITA model: "
                    + e.getMessage(), e);
        }

        File modelFile;
        try {
            modelFile = File.createTempFile("dita2graph-model", ".json");
            modelFile.deleteOnExit();
            writeModel(modelFile, nodes);
        } catch (IOException e) {
            throw new BuildException("dita2graph:extract: failed to write the normalized model: " + e.getMessage(), e);
        }
        log("dita2graph:extract: wrote normalized model (" + nodes.size() + " nodes) to " + modelFile, Project.MSG_VERBOSE);

        String coreBin = resolveCoreBinary();
        String resolvedStore = (store == null || store.isEmpty()) ? "sqlite" : store;
        String resolvedEmitGraphJson = (emitGraphJson == null || emitGraphJson.isEmpty()) ? "true" : emitGraphJson;
        int exitCode = runCore(coreBin, modelFile, outputDir, resolvedStore, resolvedEmitGraphJson);
        if (exitCode != 0) {
            throw new BuildException("dita2graph:extract: dita2graph-core exited with code " + exitCode
                    + " (§2.5: 0 success, 1 validation failure, 2 internal error)");
        }
        log("dita2graph:extract: wrote OKF bundle to " + outputDir + "/okf", Project.MSG_INFO);
    }

    private void writeModel(File file, List<Object> nodes) throws IOException {
        try (FileWriter writer = new FileWriter(file, StandardCharsets.UTF_8)) {
            writer.write('[');
            for (int i = 0; i < nodes.size(); i++) {
                if (i > 0) {
                    writer.write(',');
                }
                Object node = nodes.get(i);
                writer.write(node instanceof MapNode ? ((MapNode) node).toJson() : ((TopicNode) node).toJson());
            }
            writer.write(']');
        }
    }

    /**
     * Locates the {@code dita2graph-core} binary: {@code DITA2GRAPH_CORE_BIN}
     * env var first, then bare {@code dita2graph-core} on {@code PATH}.
     * Deliberately not a repo-relative path (e.g. into {@code target/}):
     * this task runs from an installed plugin zip, which has no
     * {@code target/} directory of its own -- bundling/locating the
     * platform-specific Rust binary for a real release is Phase 4/5 work
     * (§12), not solved here.
     */
    private String resolveCoreBinary() {
        String env = System.getenv("DITA2GRAPH_CORE_BIN");
        if (env != null && !env.isEmpty()) {
            return env;
        }
        return "dita2graph-core";
    }

    private int runCore(String coreBin, File modelFile, String outputDir, String store, String emitGraphJson)
            throws BuildException {
        ProcessBuilder pb = new ProcessBuilder(
                coreBin, "build",
                "--input", modelFile.getAbsolutePath(),
                "--output", outputDir,
                "--store", store,
                "--emit-graph-json", emitGraphJson);
        pb.redirectErrorStream(true);
        try {
            Process process = pb.start();
            try (BufferedReader reader = new BufferedReader(
                    new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8))) {
                String line;
                while ((line = reader.readLine()) != null) {
                    log("dita2graph-core: " + line, Project.MSG_INFO);
                }
            }
            return process.waitFor();
        } catch (IOException e) {
            throw new BuildException("dita2graph:extract: could not run '" + coreBin + "' ("
                    + e.getMessage() + "). Set DITA2GRAPH_CORE_BIN or put dita2graph-core on PATH.", e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new BuildException("dita2graph:extract: interrupted while running dita2graph-core", e);
        }
    }

    private void require(String value, String attrName) {
        if (value == null || value.isEmpty()) {
            throw new BuildException("dita2graph:extract: required attribute '" + attrName + "' is missing");
        }
    }
}
