package org.dita.dita2graph.tasks;

/**
 * One edge of the DITA relation taxonomy (docs/plugin-specification.md
 * §4.3): {@code relation} is one of {@code contains}, {@code references},
 * or {@code requires} -- the only three this extractor derives directly
 * from DITA markup (map {@code topicref} hierarchy, and {@code xref}/
 * {@code link} elements gated on whether they carry a {@code keyref}).
 * {@code applies-to}, {@code related-to}, and {@code generated-from}
 * require inference this Java layer doesn't attempt yet (§3.3, §13
 * future work) and never appear here.
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
