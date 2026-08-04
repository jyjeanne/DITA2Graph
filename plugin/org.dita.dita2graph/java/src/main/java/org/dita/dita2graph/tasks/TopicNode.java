package org.dita.dita2graph.tasks;

import java.util.ArrayList;
import java.util.List;

/**
 * One topic node of the normalized DITA model (docs/plugin-specification.md
 * §3.2). Mirrors {@code core/dita2graph-core/src/model.rs}'s
 * {@code NormalizedTopic} field-for-field; the wire format produced by
 * {@link #toJson()} is read directly by that Rust struct's
 * {@code serde(deserialize)} impl, so the two must stay in sync.
 */
final class TopicNode {
    String id;
    /** concept | task | reference | glossentry | topic (§4.1's OKF type mapping). */
    String topicType = "topic";
    String title = "";
    /** Nullable: omitted from JSON entirely when absent, matching the Rust side's Option&lt;String&gt;. */
    String shortdesc;
    /**
     * Whitespace-normalized text content of the topic's body element
     * (concept/task/reference/glossentry/generic-topic, per {@link
     * DitaModelExtractor#bodyElementTag}) -- markup stripped, no
     * further cleanup. Nullable and omitted when empty, same convention
     * as {@link #shortdesc}.
     */
    String body;
    final List<String> audience = new ArrayList<>();
    final List<String> product = new ArrayList<>();
    final List<String> keys = new ArrayList<>();
    /**
     * Distinct {@code <uicontrol>} text found anywhere in this topic's
     * body -- what a reference topic "defines" (§3.3's {@code applies-to}
     * inference target side, {@code core/dita2graph-core/src/relations.rs}).
     */
    final List<String> uicontrols = new ArrayList<>();
    /**
     * Distinct {@code <uicontrol>} text found specifically inside
     * {@code <cmd>} elements -- what a task step "invokes" (§3.3's
     * {@code applies-to} inference source side).
     */
    final List<String> cmdUicontrols = new ArrayList<>();
    String sourceFile;
    final List<Link> links = new ArrayList<>();
    /** DITA {@code status} attribute, e.g. "draft" -- used for include-drafts filtering, not serialized. */
    String status;

    String toJson() {
        StringBuilder sb = new StringBuilder();
        sb.append("{\"type\":\"topic\"");
        sb.append(",\"id\":").append(Json.string(id));
        sb.append(",\"topicType\":").append(Json.string(topicType));
        sb.append(",\"title\":").append(Json.string(title));
        if (shortdesc != null && !shortdesc.isEmpty()) {
            sb.append(",\"shortdesc\":").append(Json.string(shortdesc));
        }
        if (body != null && !body.isEmpty()) {
            sb.append(",\"body\":").append(Json.string(body));
        }
        sb.append(",\"audience\":").append(Json.stringArray(audience));
        sb.append(",\"product\":").append(Json.stringArray(product));
        sb.append(",\"keys\":").append(Json.stringArray(keys));
        sb.append(",\"uicontrols\":").append(Json.stringArray(uicontrols));
        sb.append(",\"cmdUicontrols\":").append(Json.stringArray(cmdUicontrols));
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
