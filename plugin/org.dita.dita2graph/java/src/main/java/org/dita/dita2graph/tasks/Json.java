package org.dita.dita2graph.tasks;

import java.util.List;

/**
 * A minimal JSON string writer, just enough for the normalized DITA
 * model's flat schema (docs/plugin-specification.md §3.2): strings,
 * string arrays, and arrays of two-field link objects. Deliberately not
 * a general-purpose JSON library -- avoids adding a compile dependency
 * beyond Ant itself, matching {@code core/dita2graph-core}'s own "thin
 * Java bridge" framing (§2.1).
 */
final class Json {

    private Json() {
    }

    static String string(String s) {
        if (s == null) {
            return "null";
        }
        StringBuilder sb = new StringBuilder(s.length() + 8);
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"':
                    sb.append("\\\"");
                    break;
                case '\\':
                    sb.append("\\\\");
                    break;
                case '\n':
                    sb.append("\\n");
                    break;
                case '\r':
                    sb.append("\\r");
                    break;
                case '\t':
                    sb.append("\\t");
                    break;
                default:
                    if (c < 0x20) {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
            }
        }
        sb.append('"');
        return sb.toString();
    }

    static String stringArray(List<String> values) {
        StringBuilder sb = new StringBuilder("[");
        for (int i = 0; i < values.size(); i++) {
            if (i > 0) {
                sb.append(',');
            }
            sb.append(string(values.get(i)));
        }
        return sb.append(']').toString();
    }
}
