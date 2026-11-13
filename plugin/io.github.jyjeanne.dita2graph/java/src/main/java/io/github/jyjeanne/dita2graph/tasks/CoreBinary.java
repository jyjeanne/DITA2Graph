package io.github.jyjeanne.dita2graph.tasks;

import java.io.File;
import java.util.Locale;

/**
 * Finds the {@code dita2graph-core} binary {@link ExtractTask} runs.
 *
 * <p>The plug-in ZIP ships one build per platform, as
 * {@code bin/<os>-<arch>/dita2graph-core} ({@code .exe} on Windows), so a
 * {@code dita install} is all it takes. In order, the binary run is:
 * <ol>
 *   <li>the one {@code DITA2GRAPH_CORE_BIN} names, for a local build or a
 *       platform the plug-in ships nothing for;</li>
 *   <li>the one the plug-in ships for this platform;</li>
 *   <li>{@code dita2graph-core} on {@code PATH}.</li>
 * </ol>
 *
 * <p>{@code dita install} expands the ZIP with {@code java.util.zip}, which
 * drops the Unix permission bits, so the shipped binary is made executable
 * here before it is run.
 */
final class CoreBinary {

    static final String NAME = "dita2graph-core";

    private CoreBinary() {
    }

    /**
     * The {@code bin/} subdirectory holding the build for this OS and
     * architecture, e.g. {@code linux-x86_64}, or {@code null} for a
     * platform the plug-in ships no build for.
     */
    static String platform(String osName, String osArch) {
        String os = osName == null ? "" : osName.toLowerCase(Locale.ROOT);
        String family;
        if (os.startsWith("windows")) {
            family = "windows";
        } else if (os.startsWith("mac")) {
            family = "macos";
        } else if (os.startsWith("linux")) {
            family = "linux";
        } else {
            return null;
        }
        String arch = switch (osArch == null ? "" : osArch.toLowerCase(Locale.ROOT)) {
            case "amd64", "x86_64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            default -> null;
        };
        return arch == null ? null : family + "-" + arch;
    }

    /**
     * The binary to run: {@code override} when set, else the shipped build
     * for this platform when the plug-in has one, else {@link #NAME}, which
     * the system looks up on {@code PATH}.
     *
     * @throws IllegalStateException when the shipped build cannot be made
     *         executable
     */
    static String resolve(String override, File pluginDir, String osName, String osArch) {
        if (override != null && !override.isEmpty()) {
            return override;
        }
        File shipped = shipped(pluginDir, osName, osArch);
        if (shipped == null) {
            return NAME;
        }
        if (!shipped.canExecute() && !shipped.setExecutable(true, false)) {
            throw new IllegalStateException(shipped + " is not executable and its permissions cannot be"
                    + " changed: run chmod +x on it, or set DITA2GRAPH_CORE_BIN to another build.");
        }
        return shipped.getAbsolutePath();
    }

    /** The build the plug-in ships for this platform, or {@code null} when it has none. */
    static File shipped(File pluginDir, String osName, String osArch) {
        String platform = platform(osName, osArch);
        if (pluginDir == null || platform == null) {
            return null;
        }
        String file = platform.startsWith("windows") ? NAME + ".exe" : NAME;
        File candidate = new File(new File(new File(pluginDir, "bin"), platform), file);
        return candidate.isFile() ? candidate : null;
    }
}
