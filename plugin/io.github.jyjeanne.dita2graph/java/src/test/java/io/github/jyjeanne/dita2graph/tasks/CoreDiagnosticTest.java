package io.github.jyjeanne.dita2graph.tasks;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;

class CoreDiagnosticTest {

    @Test
    void coreDiagnosticsKeepTheirSeverityAndTheirText() {
        // The lines dita2graph-core writes (src/diagnostics.rs), quotes and
        // a non-ASCII character escaped as serde_json may escape them.
        assertEquals(new CoreDiagnostic("warning",
                        "DITA2GRAPH081W: PublishedTopicShape on https://ex.org/node/a: \"Published\" topic \u00e9"),
                CoreDiagnostic.parse("{\"id\":\"DITA2GRAPH081W\",\"severity\":\"warning\","
                        + "\"message\":\"PublishedTopicShape on https://ex.org/node/a: \\\"Published\\\" topic \\u00e9\"}"));
        assertEquals(new CoreDiagnostic("error", "DITA2GRAPH080E: a loop"),
                CoreDiagnostic.parse("{\"id\":\"DITA2GRAPH080E\",\"severity\":\"error\",\"message\":\"a loop\"}"));
        assertEquals(new CoreDiagnostic("info", "DITA2GRAPH082I: a note"),
                CoreDiagnostic.parse("{\"id\":\"DITA2GRAPH082I\",\"severity\":\"info\",\"message\":\"a note\"}"));
    }

    /**
     * The lines dita2graph-core writes, from the golden file its own
     * {@code tests/diagnostics.rs} checks them against: one per message of
     * its catalog, then the messages below.
     */
    @Test
    void everyLineTheCoreWritesIsParsed() throws IOException {
        Path golden = Path.of("../../../core/dita2graph-core/tests/data/diagnostics.jsonl");
        List<String> lines = Files.readAllLines(golden, StandardCharsets.UTF_8);
        List<CoreDiagnostic> special = List.of(
                new CoreDiagnostic("warning",
                        "DITA2GRAPH081W: PublishedTopicShape on https://ex.org/node/a: \"Published\" topic \u00e9"),
                new CoreDiagnostic("error", "DITA2GRAPH080E: C:\\docs\\a.dita loops back\tto itself"),
                new CoreDiagnostic("info", "DITA2GRAPH082I: first line\nsecond line"),
                new CoreDiagnostic("warning", "DITA2GRAPH083W: \u65e5\u672c\u8a9e \ud83d\ude80 \u0001"));
        int catalog = lines.size() - special.size();

        Map<Character, String> severities = Map.of('E', "error", 'W', "warning", 'I', "info");
        for (String line : lines.subList(0, catalog)) {
            CoreDiagnostic diagnostic = CoreDiagnostic.parse(line);
            assertNotNull(diagnostic, line);
            String id = diagnostic.text().substring(0, "DITA2GRAPH000E".length());
            assertEquals(severities.get(id.charAt(id.length() - 1)), diagnostic.severity(), line);
            assertEquals(id + ": sample message", diagnostic.text(), line);
        }
        assertEquals(special, lines.subList(catalog, lines.size()).stream().map(CoreDiagnostic::parse).toList());
    }

    @Test
    void otherCoreOutputIsNotADiagnostic() {
        assertNull(CoreDiagnostic.parse("wrote 228 concept(s) to out/okf"));
        assertNull(CoreDiagnostic.parse("{\"id\":\"OTHER\",\"severity\":\"warning\",\"message\":\"x\"}"));
    }
}
