package org.dita.dita2graph.tasks;

/**
 * One edge of the DITA relation taxonomy (docs/plugin-specification.md
 * §4.3): {@code relation} is {@code contains}, {@code references}, or
 * {@code requires} -- derived directly from DITA markup (map {@code
 * topicref} hierarchy, and {@code xref}/{@code link} elements gated on
 * whether they carry a {@code keyref}) -- or {@code generated-from},
 * derived from DITA-OT's own {@code xtrf} source-trace attributes
 * (finding 15): a resolved element whose {@code xtrf} points at a
 * *different* file than its containing topic's own source was pulled in
 * via {@code conref}/{@code conkeyref}, both equally deterministic, not
 * inference. {@code applies-to} and {@code related-to} require actual
 * heuristic inference (matching {@code uicontrol} text or {@code
 * product} metadata across topics) and are computed downstream in the
 * Rust core (`relations.rs`) instead, never here.
 */
final class Link {
    final String relation;
    final String target;

    Link(String relation, String target) {
        this.relation = relation;
        this.target = target;
    }

    String toJson() {
        return "{\"relation\":" + Json.string(relation) + ",\"target\":" + Json.string(target) + "}";
    }
}
