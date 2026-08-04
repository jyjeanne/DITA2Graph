package org.dita.dita2graph.tasks;

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

    String toJson() {
        StringBuilder sb = new StringBuilder();
        sb.append("{\"type\":\"map\"");
        sb.append(",\"id\":").append(Json.string(id));
        sb.append(",\"title\":").append(Json.string(title));
        sb.append(",\"sourceFile\":").append(Json.string(sourceFile));
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
