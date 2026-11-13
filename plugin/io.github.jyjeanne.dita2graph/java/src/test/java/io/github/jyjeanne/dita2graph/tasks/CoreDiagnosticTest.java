package io.github.jyjeanne.dita2graph.tasks;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
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

    @Test
    void otherCoreOutputIsNotADiagnostic() {
        assertNull(CoreDiagnostic.parse("wrote 228 concept(s) to out/okf"));
        assertNull(CoreDiagnostic.parse("{\"id\":\"OTHER\",\"severity\":\"warning\",\"message\":\"x\"}"));
    }
}
