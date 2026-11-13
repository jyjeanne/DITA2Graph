package io.github.jyjeanne.dita2graph.tasks;

import java.util.List;

/**
 * One edge of the DITA relation taxonomy (docs/plugin-specification.md
 * §4.3): {@code relation} is {@code contains}, {@code references}, or
 * {@code requires} -- derived directly from DITA markup (map {@code
 * topicref} hierarchy, and {@code xref}/{@code link} elements, a task's
 * prerequisite links being {@code requires}) -- {@code copy-of}, from a
 * {@code copy-to} copy to its source topic, or {@code generated-from},
 * derived from DITA-OT's own {@code xtrf} source-trace attributes
 * (finding 15): a resolved element whose {@code xtrf} points at a
 * *different* file than its containing topic's own source was pulled in
 * via {@code conref}/{@code conkeyref}, both equally deterministic, not
 * inference. {@code applies-to} and {@code related-to} require actual
 * heuristic inference (matching {@code uicontrol} text or {@code
 * product} metadata across topics) and are computed downstream in the
 * Rust core (`relations.rs`) instead, never here.
 *
 * <p>{@code fragments} is only ever set on {@code generated-from}: the ids
 * of the reused elements in the target topic ({@code plug-in}, or {@code
 * li@37} when the element has no id). Omitted from JSON when empty.
 */
final class Link {
    final String relation;
    final String target;
    final List<String> fragments;

    Link(String relation, String target) {
        this(relation, target, List.of());
    }

    Link(String relation, String target, List<String> fragments) {
        this.relation = relation;
        this.target = target;
        this.fragments = fragments;
    }

    String toJson() {
        String json = "{\"relation\":" + Json.string(relation) + ",\"target\":" + Json.string(target);
        if (!fragments.isEmpty()) {
            json += ",\"fragments\":" + Json.stringArray(fragments);
        }
        return json + "}";
    }
}
