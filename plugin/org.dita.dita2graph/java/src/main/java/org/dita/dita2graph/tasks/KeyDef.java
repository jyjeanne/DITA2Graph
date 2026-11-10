package org.dita.dita2graph.tasks;

/**
 * One key definition of the map (a {@code <keydef>}, or any topicref
 * with {@code @keys}), mirroring {@code KeyDef} in
 * {@code core/dita2graph-core/src/model.rs}. At most one of {@link
 * #target}, {@link #url} and {@link #file} is set; a key with none of
 * them is a text key (a variable), whose value is {@link #text}.
 */
final class KeyDef {
    String name;
    /** Dotted key scope ({@code "a.b"}); null for the root scope. */
    String scope;
    /** Graph id of the topic the key names. */
    String target;
    /** URL of an external resource the key names. */
    String url;
    /** Temp-relative path of a local non-topic resource the key names (an image, a PDF). */
    String file;
    /** The key's text (keyword, link text or navtitle), kept when it names no topic. */
    String text;
    /** The map file that defines the key (the submap, not the root map, when they differ). */
    String sourceFile;

    String toJson() {
        StringBuilder sb = new StringBuilder();
        sb.append("{\"name\":").append(Json.string(name));
        optional(sb, "scope", scope);
        optional(sb, "target", target);
        optional(sb, "url", url);
        optional(sb, "file", file);
        optional(sb, "text", text);
        sb.append(",\"sourceFile\":").append(Json.string(sourceFile));
        return sb.append('}').toString();
    }

    private static void optional(StringBuilder sb, String field, String value) {
        if (value != null && !value.isEmpty()) {
            sb.append(",\"").append(field).append("\":").append(Json.string(value));
        }
    }
}
