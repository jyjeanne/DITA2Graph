package io.github.jyjeanne.dita2graph.tasks;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Checks on the files DITA-OT reads when it installs the plugin: the
 * Gradle project sits in the plugin's java/ directory.
 */
class PluginDescriptorTest {

    private static final Path PLUGIN = Path.of("..");

    /**
     * DITA-OT 4.4.1 on Java 17 under Windows, whose default charset is
     * windows-1252, failed to install the plugin while a parameter
     * description in plugin.xml had a "§" ("Invalid byte 1 of 1-byte UTF-8
     * sequence"): its integrator writes what it gathers from the
     * descriptors in the default charset and reads it back as UTF-8. Java 18
     * and later default to UTF-8, which hides it. Keeping these files ASCII
     * rules it out.
     */
    @Test
    void theFilesDitaOtReadsAreAscii() throws IOException {
        for (String file : List.of("plugin.xml", "build.xml", "cfg/dita2graph.xml", "cfg/messages.xml")) {
            List<String> lines = Files.readAllLines(PLUGIN.resolve(file), StandardCharsets.UTF_8);
            for (int i = 0; i < lines.size(); i++) {
                String line = lines.get(i);
                assertTrue(line.chars().allMatch(c -> c < 0x80), file + ":" + (i + 1) + " is not ASCII: " + line);
            }
        }
    }

    /** The plugin's version is the release's, Cargo.toml's (tag.yml checks the tag against both). */
    @Test
    void thePluginVersionIsTheWorkspaceVersion() throws IOException {
        String plugin = Files.readString(PLUGIN.resolve("plugin.xml"), StandardCharsets.UTF_8);
        String cargo = Files.readString(PLUGIN.resolve("../../Cargo.toml"), StandardCharsets.UTF_8);

        Matcher pluginVersion = Pattern.compile("<plugin id=\"io\\.github\\.jyjeanne\\.dita2graph\" version=\"([^\"]+)\">")
                .matcher(plugin);
        Matcher cargoVersion = Pattern.compile("(?m)^\\[workspace\\.package\\]\\s+version = \"([^\"]+)\"").matcher(cargo);
        assertTrue(pluginVersion.find(), "plugin.xml has no id and version");
        assertTrue(cargoVersion.find(), "Cargo.toml has no [workspace.package] version");
        assertEquals(cargoVersion.group(1), pluginVersion.group(1));
    }

    /**
     * build.xml's dita2graph:extract is ExtractTask: Ant resolves
     * antlib:io.github.jyjeanne.dita2graph to that package's antlib.xml in
     * the jar, and the task finds the binaries the plugin ships through
     * DITA-OT's property for this plugin's directory.
     */
    @Test
    void buildXmlRunsTheTaskOfThisJar() throws IOException {
        String build = Files.readString(PLUGIN.resolve("build.xml"), StandardCharsets.UTF_8);
        assertTrue(build.contains("xmlns:dita2graph=\"antlib:io.github.jyjeanne.dita2graph\""), build);
        assertTrue(build.contains("pluginDir=\"${dita.plugin.io.github.jyjeanne.dita2graph.dir}\""), build);

        try (var antlib = getClass().getClassLoader().getResourceAsStream("io/github/jyjeanne/dita2graph/antlib.xml")) {
            assertTrue(antlib != null, "no antlib.xml for antlib:io.github.jyjeanne.dita2graph");
            String definitions = new String(antlib.readAllBytes(), StandardCharsets.UTF_8);
            assertTrue(definitions.contains("<taskdef name=\"extract\" classname=\"" + ExtractTask.class.getName() + "\"/>"),
                    definitions);
        }
    }
}
