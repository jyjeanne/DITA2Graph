package io.github.jyjeanne.dita2graph.tasks;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

class CoreBinaryTest {

    @Test
    void eachShippedPlatformHasItsDirectory() {
        // os.name and os.arch as the JDKs report them.
        assertEquals("linux-x86_64", CoreBinary.platform("Linux", "amd64"));
        assertEquals("linux-aarch64", CoreBinary.platform("Linux", "aarch64"));
        assertEquals("macos-x86_64", CoreBinary.platform("Mac OS X", "x86_64"));
        assertEquals("macos-aarch64", CoreBinary.platform("Mac OS X", "aarch64"));
        assertEquals("windows-x86_64", CoreBinary.platform("Windows 11", "amd64"));
        assertEquals("windows-aarch64", CoreBinary.platform("Windows Server 2025", "arm64"));
    }

    @Test
    void anotherPlatformHasNone() {
        assertNull(CoreBinary.platform("FreeBSD", "amd64"));
        assertNull(CoreBinary.platform("Linux", "ppc64le"));
        assertNull(CoreBinary.platform(null, null));
    }

    @Test
    void theShippedBuildIsRunAndMadeExecutable(@TempDir Path pluginDir) throws IOException {
        // As dita install leaves it: a plain file, without the execute bit.
        Path binary = shipped(pluginDir, "linux-x86_64", "dita2graph-core");
        binary.toFile().setExecutable(false, false);

        assertEquals(binary.toFile().getAbsolutePath(),
                CoreBinary.resolve(null, pluginDir.toFile(), "Linux", "amd64"));
        assertTrue(binary.toFile().canExecute());
    }

    @Test
    void theWindowsBuildIsTheExe(@TempDir Path pluginDir) throws IOException {
        Path binary = shipped(pluginDir, "windows-x86_64", "dita2graph-core.exe");

        assertEquals(binary.toFile().getAbsolutePath(),
                CoreBinary.resolve("", pluginDir.toFile(), "Windows 11", "amd64"));
    }

    @Test
    void theEnvironmentVariableWinsOverTheShippedBuild(@TempDir Path pluginDir) throws IOException {
        shipped(pluginDir, "linux-x86_64", "dita2graph-core");

        assertEquals("/opt/dita2graph/dita2graph-core",
                CoreBinary.resolve("/opt/dita2graph/dita2graph-core", pluginDir.toFile(), "Linux", "amd64"));
    }

    @Test
    void withoutAShippedBuildPathDecides(@TempDir Path pluginDir) throws IOException {
        shipped(pluginDir, "linux-x86_64", "dita2graph-core");

        // Another platform, a plug-in without bin/, no plug-in directory.
        assertEquals("dita2graph-core", CoreBinary.resolve(null, pluginDir.toFile(), "Mac OS X", "aarch64"));
        assertEquals("dita2graph-core", CoreBinary.resolve(null, new File(pluginDir.toFile(), "none"), "Linux", "amd64"));
        assertEquals("dita2graph-core", CoreBinary.resolve(null, null, "Linux", "amd64"));
    }

    private static Path shipped(Path pluginDir, String platform, String file) throws IOException {
        Path dir = Files.createDirectories(pluginDir.resolve("bin").resolve(platform));
        return Files.writeString(dir.resolve(file), "binary");
    }
}
