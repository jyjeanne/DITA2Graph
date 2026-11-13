package io.github.jyjeanne.dita2graph.tasks;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * A diagnostic {@code dita2graph-core} writes as a JSON line (§2.5,
 * {@code core/dita2graph-core/src/diagnostics.rs}):
 * {@code {"id":"DITA2GRAPH081W","severity":"warning","message":"..."}}.
 *
 * @param severity {@code error}, {@code warning} or {@code info}
 * @param text     the id and the message, as the extractor's own diagnostics read
 */
record CoreDiagnostic(String severity, String text) {

    private static final Pattern LINE = Pattern.compile(
            "^\\{\"id\":\"(DITA2GRAPH\\d{3}[EWI])\",\"severity\":\"(error|warning|info)\",\"message\":\"(.*)\"\\}$");

    /** The diagnostic on this line of the core's output, or null for any other output. */
    static CoreDiagnostic parse(String line) {
        Matcher m = LINE.matcher(line);
        if (!m.matches()) {
            return null;
        }
        return new CoreDiagnostic(m.group(2), m.group(1) + ": " + Json.unescape(m.group(3)));
    }
}
