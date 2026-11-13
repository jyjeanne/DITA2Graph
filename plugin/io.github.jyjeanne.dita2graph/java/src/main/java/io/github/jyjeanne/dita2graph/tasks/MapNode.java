package io.github.jyjeanne.dita2graph.tasks;

import java.util.ArrayList;
import java.util.List;

/**
 * The ditamap node of the normalized DITA model (§3.2), mirroring
 * {@code NormalizedMap} in {@code core/dita2graph-core/src/model.rs}.
 */
final class MapNode {
    String id;
    String title = "";
    String sourceFile;
    final List<Link> links = new ArrayList<>();
    /** Effective key definitions, in map order: the first definition of a key in its scope wins. */
    final List<KeyDef> keyDefs = new ArrayList<>();
    /** The map's own topicmeta/bookmeta metadata (dates, product...). */
    Metadata metadata;

    String toJson() {
        StringBuilder sb = new StringBuilder();
        sb.append("{\"type\":\"map\"");
        sb.append(",\"id\":").append(Json.string(id));
        sb.append(",\"title\":").append(Json.string(title));
        sb.append(",\"sourceFile\":").append(Json.string(sourceFile));
        if (!keyDefs.isEmpty()) {
            sb.append(",\"keyDefs\":[");
            for (int i = 0; i < keyDefs.size(); i++) {
                if (i > 0) {
                    sb.append(',');
                }
                sb.append(keyDefs.get(i).toJson());
            }
            sb.append(']');
        }
        if (metadata != null && !metadata.isEmpty()) {
            sb.append(",\"metadata\":").append(metadata.toJson());
        }
        sb.append(",\"links\":[");
        for (int i = 0; i < links.size(); i++) {
            if (i > 0) {
                sb.append(',');
            }
            sb.append(links.get(i).toJson());
        }
        sb.append("]}");
        return sb.toString();
    }
}
