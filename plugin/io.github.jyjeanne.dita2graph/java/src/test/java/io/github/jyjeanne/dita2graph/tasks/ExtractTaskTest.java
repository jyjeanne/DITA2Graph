package io.github.jyjeanne.dita2graph.tasks;

import org.apache.tools.ant.BuildEvent;
import org.apache.tools.ant.BuildException;
import org.apache.tools.ant.BuildListener;
import org.apache.tools.ant.Project;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Runs the {@code dita2graph:extract} Ant task the way DITA-OT does,
 * against a resolved temp directory, with a stand-in for
 * {@code dita2graph-core} that records its arguments, writes the kind of
 * lines the real one writes, and exits with a chosen code.
 */
class ExtractTaskTest {

    private static final boolean WINDOWS = System.getProperty("os.name").startsWith("Windows");

    @TempDir
    Path work;

    private Path tempDir;
    private Path outputDir;
    private final List<String[]> logged = new ArrayList<>();

    @BeforeEach
    void ditaOtTempDir() throws IOException {
        tempDir = work.resolve("temp");
        outputDir = work.resolve("out");
        write(tempDir.resolve(".job.xml"), "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/configuration.dita\" uri=\"topics/configuration.dita\" "
                + "path=\"topics/configuration.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
                + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>");
        write(tempDir.resolve("user-guide.ditamap"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/configuration.dita\"/></map>");
        write(tempDir.resolve("topics/configuration.dita"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"configuration\"><title>Configuración</title>"
                + "<conbody><p>Settings.</p></conbody></concept>");
    }

    @Test
    void theCoreGetsTheModelAndEveryArgument() throws IOException {
        ExtractTask task = task(fakeCore(0));
        task.setStore("none");
        task.setEmitGraphJson("false");
        task.setMcp("true");
        task.setRdf("true");
        task.setRdfBase("  https://example.org/docs/  ");
        task.execute();

        List<String> args = coreArguments();
        assertEquals(List.of("build", "--input", args.get(2), "--output", outputDir.toString(),
                "--store", "none", "--emit-graph-json", "false", "--mcp", "true",
                "--emit-rdf", "true", "--rdf-base", "https://example.org/docs/"), args);

        // The model the core reads, in UTF-8 whatever the platform's charset.
        String model = Files.readString(Path.of(args.get(2)), StandardCharsets.UTF_8);
        assertTrue(model.startsWith("[{\"type\":\"map\",\"id\":\"user-guide\""), model);
        assertTrue(model.contains("{\"type\":\"topic\",\"id\":\"configuration\",\"topicType\":\"concept\","
                + "\"title\":\"Configuración\""), model);
    }

    @Test
    void unsetArgumentsTakeTheBuildXmlDefaults() throws IOException {
        ExtractTask task = task(fakeCore(0));
        // An empty base, build.xml's default, means no base at all.
        task.setRdfBase("   ");
        task.execute();

        List<String> args = coreArguments();
        assertEquals(List.of("--store", "sqlite", "--emit-graph-json", "true", "--mcp", "false",
                "--emit-rdf", "false"), args.subList(5, args.size()));
    }

    @Test
    void coreDiagnosticsAreLoggedAtTheirOwnLevel() throws IOException {
        task(fakeCore(0)).execute();

        assertTrue(wasLogged(Project.MSG_WARN, "DITA2GRAPH081W: a warning"), describeLog());
        assertTrue(wasLogged(Project.MSG_ERR, "DITA2GRAPH080E: an error"), describeLog());
        assertTrue(wasLogged(Project.MSG_INFO, "dita2graph-core: wrote 1 topic"), describeLog());
    }

    @Test
    void aFailingCoreFailsTheBuildWithItsExitCode() throws IOException {
        ExtractTask task = task(fakeCore(1));

        BuildException e = assertThrows(BuildException.class, task::execute);
        assertTrue(e.getMessage().contains("dita2graph-core exited with code 1"), e.getMessage());
    }

    @Test
    void aMissingCoreSaysHowToProvideOne() {
        ExtractTask task = task(work.resolve("no-such-core").toString());

        BuildException e = assertThrows(BuildException.class, task::execute);
        assertTrue(e.getMessage().contains("DITA2GRAPH_CORE_BIN"), e.getMessage());
    }

    @Test
    void depthIsUnlimitedOrAPositiveInteger() throws IOException {
        String core = fakeCore(0);
        for (String depth : List.of("0", "-2", "two")) {
            ExtractTask task = task(core);
            task.setDepth(depth);

            BuildException e = assertThrows(BuildException.class, task::execute, depth);
            assertTrue(e.getMessage().contains("args.dita2graph.depth"), e.getMessage());
        }
        assertFalse(Files.exists(work.resolve("args.txt")), "the core never ran");

        for (String depth : List.of("unlimited", "3")) {
            ExtractTask task = task(core);
            task.setDepth(depth);
            task.execute();
        }
    }

    @Test
    void theTempAndOutputDirectoriesAreRequired() {
        ExtractTask noTemp = task("core");
        noTemp.setTempDir(null);
        assertTrue(assertThrows(BuildException.class, noTemp::execute).getMessage()
                .contains("required attribute 'tempDir'"));

        ExtractTask noOutput = task("core");
        noOutput.setOutputDir("");
        assertTrue(assertThrows(BuildException.class, noOutput::execute).getMessage()
                .contains("required attribute 'outputDir'"));

        ExtractTask missingTemp = task("core");
        missingTemp.setTempDir(work.resolve("gone").toString());
        assertTrue(assertThrows(BuildException.class, missingTemp::execute).getMessage()
                .contains("tempDir does not exist"));
    }

    private ExtractTask task(String core) {
        Project project = new Project();
        project.addBuildListener(new Recorder());
        ExtractTask task = new ExtractTask();
        task.setProject(project);
        task.setTaskName("dita2graph:extract");
        task.setTempDir(tempDir.toString());
        task.setOutputDir(outputDir.toString());
        task.env = name -> "DITA2GRAPH_CORE_BIN".equals(name) ? core : null;
        return task;
    }

    /**
     * A stand-in for {@code dita2graph-core}: writes its arguments to
     * {@code args.txt}, one per line, prints a warning and an error as
     * JSON lines (src/diagnostics.rs) and a plain line, then exits with
     * {@code exitCode}.
     */
    private String fakeCore(int exitCode) throws IOException {
        String warning = "{\"id\":\"DITA2GRAPH081W\",\"severity\":\"warning\",\"message\":\"a warning\"}";
        String error = "{\"id\":\"DITA2GRAPH080E\",\"severity\":\"error\",\"message\":\"an error\"}";
        Path script;
        if (WINDOWS) {
            script = work.resolve("fake-core.bat");
            write(script, String.join("\r\n",
                    "@echo off",
                    // Before the loop: shift moves %0 along with the arguments.
                    "set \"args=%~dp0args.txt\"",
                    "type nul > \"%args%\"",
                    ":next",
                    "if \"%~1\"==\"\" goto done",
                    ">> \"%args%\" echo(%~1",
                    "shift",
                    "goto next",
                    ":done",
                    "echo " + warning,
                    "1>&2 echo " + error,
                    "echo wrote 1 topic",
                    "exit /b " + exitCode,
                    ""));
        } else {
            script = work.resolve("fake-core");
            write(script, String.join("\n",
                    "#!/bin/sh",
                    "for arg in \"$@\"; do printf '%s\\n' \"$arg\"; done > \"$(dirname \"$0\")/args.txt\"",
                    "echo '" + warning + "'",
                    "echo '" + error + "' >&2",
                    "echo 'wrote 1 topic'",
                    "exit " + exitCode,
                    ""));
            assertTrue(script.toFile().setExecutable(true));
        }
        return script.toString();
    }

    private List<String> coreArguments() throws IOException {
        return Files.readAllLines(work.resolve("args.txt"), StandardCharsets.UTF_8).stream()
                .map(String::strip)
                .toList();
    }

    private boolean wasLogged(int priority, String message) {
        return logged.stream().anyMatch(entry -> entry[0].equals(String.valueOf(priority)) && entry[1].equals(message));
    }

    private String describeLog() {
        StringBuilder sb = new StringBuilder("logged:");
        logged.forEach(entry -> sb.append("\n  ").append(entry[0]).append(' ').append(entry[1]));
        return sb.toString();
    }

    private static void write(Path path, String content) throws IOException {
        Files.createDirectories(path.getParent());
        Files.writeString(path, content, StandardCharsets.UTF_8);
    }

    private class Recorder implements BuildListener {
        @Override
        public void messageLogged(BuildEvent event) {
            logged.add(new String[] {String.valueOf(event.getPriority()), event.getMessage()});
        }

        @Override
        public void buildStarted(BuildEvent event) {
        }

        @Override
        public void buildFinished(BuildEvent event) {
        }

        @Override
        public void targetStarted(BuildEvent event) {
        }

        @Override
        public void targetFinished(BuildEvent event) {
        }

        @Override
        public void taskStarted(BuildEvent event) {
        }

        @Override
        public void taskFinished(BuildEvent event) {
        }
    }
}
