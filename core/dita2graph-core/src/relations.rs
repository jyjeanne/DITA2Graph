//! Relation inference (`docs/plugin-specification.md` §3.3, §4.3):
//! deriving edges DITA doesn't state explicitly, as opposed to the
//! `contains`/`requires`/`references`/`generated-from` edges
//! `DitaModelExtractor` already derives directly and deterministically
//! (from markup and DITA-OT's own `xtrf` source-trace attributes
//! respectively, finding 15) before this crate ever sees the model.
//!
//! Two heuristics are implemented here: `related-to` (§3.3's "topics
//! sharing a `product` value are `related-to`", from `NormalizedTopic::
//! product`) and `applies-to` (§3.3's "a task's `<cmd>` referencing a
//! `<uicontrol>` defined in a reference topic", from `NormalizedTopic::
//! cmd_uicontrols`/`uicontrols` -- both populated by the Java extractor,
//! finding 15). `applies-to` runs first (main.rs's `run_build`): it's a
//! higher-confidence, directional, type-scoped signal, so it gets first
//! claim on a pair before the broader, symmetric `related-to` sweep
//! considers it.

use crate::diagnostics::{self, AMBIGUOUS_RELATION};
use crate::model::{Link, NormalizedNode, Relation, TopicType};
use std::collections::BTreeSet;

/// Adds a `related-to` `Link` to every pair of topics that share at
/// least one `product` value and aren't already connected by some other
/// relation (in either direction) -- an edge already present, of any
/// kind, is a stronger signal than the inferred one and takes
/// precedence, matching §2.5's `DITA2GRAPH010W` spirit of dropping a
/// lower-confidence inference rather than layering it on top of a real
/// one. Symmetric: inferring A `related-to` B always also adds B
/// `related-to` A, the same way a real `<related-links role="related">`
/// block would appear in both topics. Map nodes have no `product` field
/// and are never involved. Returns the number of edges added (both
/// directions counted, so one related pair yields 2).
///
/// O(n²) in topic count -- fine for the corpus sizes this scaffold
/// targets; revisit with a `product -> topic ids` index if that stops
/// being true.
pub fn infer_related_to(nodes: &mut [NormalizedNode]) -> usize {
    let topics: Vec<(String, Vec<String>)> = nodes
        .iter()
        .filter_map(|n| match n {
            NormalizedNode::Topic(t) => Some((t.id.clone(), t.product.clone())),
            NormalizedNode::Map(_) => None,
        })
        .collect();

    let mut connected: BTreeSet<(String, String)> = BTreeSet::new();
    for n in nodes.iter() {
        for link in n.links() {
            connected.insert(ordered_pair(n.id(), &link.target));
        }
    }

    let mut new_edges: Vec<(String, String)> = Vec::new();
    for i in 0..topics.len() {
        for j in (i + 1)..topics.len() {
            let (id_a, products_a) = &topics[i];
            let (id_b, products_b) = &topics[j];
            if connected.contains(&ordered_pair(id_a, id_b)) {
                continue;
            }
            if products_a.iter().any(|p| products_b.contains(p)) {
                new_edges.push((id_a.clone(), id_b.clone()));
                new_edges.push((id_b.clone(), id_a.clone()));
            }
        }
    }

    let count = new_edges.len();
    for (from, to) in new_edges {
        if let Some(NormalizedNode::Topic(t)) = nodes.iter_mut().find(|n| n.id() == from) {
            t.links.push(Link {
                relation: Relation::RelatedTo,
                target: to,
            });
        }
    }
    count
}

/// Adds an `applies-to` `Link` from a Task topic to a Reference topic
/// when a `<uicontrol>` term the task uses in a `<cmd>` (`
/// cmd_uicontrols`) also appears anywhere in that reference topic's body
/// (`uicontrols`) -- §3.3's "a task's `<cmd>` referencing a `<uicontrol>`
/// defined in a reference topic" example, made precise: the match must
/// be *unambiguous* (exactly one candidate Reference topic for that
/// term) to become an edge. A term matching two or more Reference
/// topics is the canonical `DITA2GRAPH010W` case (§2.5's own example is
/// literally "two candidate `applies-to` targets") -- dropped and
/// logged, not guessed at by picking one. Same "an existing relation (in
/// either direction) wins" precedence as `infer_related_to`, checked
/// once against the model's state *before* this function runs (a term
/// match doesn't get to invalidate another term match found earlier in
/// the same call). At most one edge per (task, reference) pair even if
/// several distinct terms all resolve to the same reference topic.
/// Returns the number of edges added.
pub fn infer_applies_to(nodes: &mut [NormalizedNode]) -> usize {
    let tasks: Vec<(String, Vec<String>)> = nodes
        .iter()
        .filter_map(|n| match n {
            NormalizedNode::Topic(t) if t.topic_type == TopicType::Task => {
                Some((t.id.clone(), t.cmd_uicontrols.clone()))
            }
            _ => None,
        })
        .collect();

    let references: Vec<(String, Vec<String>)> = nodes
        .iter()
        .filter_map(|n| match n {
            NormalizedNode::Topic(t) if t.topic_type == TopicType::Reference => {
                Some((t.id.clone(), t.uicontrols.clone()))
            }
            _ => None,
        })
        .collect();

    let mut connected: BTreeSet<(String, String)> = BTreeSet::new();
    for n in nodes.iter() {
        for link in n.links() {
            connected.insert(ordered_pair(n.id(), &link.target));
        }
    }

    let mut new_edges: Vec<(String, String)> = Vec::new();
    let mut added_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for (task_id, terms) in &tasks {
        for term in terms {
            let matches: Vec<&String> = references
                .iter()
                .filter(|(_, uicontrols)| uicontrols.contains(term))
                .map(|(id, _)| id)
                .collect();
            match matches.as_slice() {
                [reference_id] => {
                    if task_id != *reference_id
                        && !connected.contains(&ordered_pair(task_id, reference_id))
                        && added_pairs.insert((task_id.clone(), (*reference_id).clone()))
                    {
                        new_edges.push((task_id.clone(), (*reference_id).clone()));
                    }
                }
                [] => {}
                _ => {
                    let candidates = matches
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    diagnostics::emit(
                        AMBIGUOUS_RELATION,
                        &format!(
                            "applies-to: uicontrol {term:?} used by task {task_id:?} matches \
                             {} reference topics ({candidates}); dropping rather than guessing",
                            matches.len()
                        ),
                    );
                }
            }
        }
    }

    let count = new_edges.len();
    for (task_id, reference_id) in new_edges {
        if let Some(NormalizedNode::Topic(t)) = nodes.iter_mut().find(|n| n.id() == task_id) {
            t.links.push(Link {
                relation: Relation::AppliesTo,
                target: reference_id,
            });
        }
    }
    count
}

fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NormalizedMap, NormalizedTopic, TopicType};

    fn topic(id: &str, product: &[&str]) -> NormalizedNode {
        NormalizedNode::Topic(NormalizedTopic {
            id: id.into(),
            topic_type: TopicType::Concept,
            title: id.into(),
            shortdesc: None,
            body: None,
            audience: vec![],
            product: product.iter().map(|s| s.to_string()).collect(),
            keys: vec![],
            uicontrols: vec![],
            cmd_uicontrols: vec![],
            source_file: format!("topics/{id}.dita"),
            links: vec![],
        })
    }

    fn task_topic(id: &str, cmd_uicontrols: &[&str]) -> NormalizedNode {
        NormalizedNode::Topic(NormalizedTopic {
            id: id.into(),
            topic_type: TopicType::Task,
            title: id.into(),
            shortdesc: None,
            body: None,
            audience: vec![],
            product: vec![],
            keys: vec![],
            uicontrols: vec![],
            cmd_uicontrols: cmd_uicontrols.iter().map(|s| s.to_string()).collect(),
            source_file: format!("topics/{id}.dita"),
            links: vec![],
        })
    }

    fn reference_topic(id: &str, uicontrols: &[&str]) -> NormalizedNode {
        NormalizedNode::Topic(NormalizedTopic {
            id: id.into(),
            topic_type: TopicType::Reference,
            title: id.into(),
            shortdesc: None,
            body: None,
            audience: vec![],
            product: vec![],
            keys: vec![],
            uicontrols: uicontrols.iter().map(|s| s.to_string()).collect(),
            cmd_uicontrols: vec![],
            source_file: format!("topics/{id}.dita"),
            links: vec![],
        })
    }

    #[test]
    fn links_two_topics_sharing_a_product_value() {
        let mut nodes = vec![topic("a", &["enterprise"]), topic("b", &["enterprise"])];
        let count = infer_related_to(&mut nodes);
        assert_eq!(count, 2);

        let a_links = nodes[0].links();
        assert_eq!(a_links.len(), 1);
        assert_eq!(a_links[0].relation, Relation::RelatedTo);
        assert_eq!(a_links[0].target, "b");

        let b_links = nodes[1].links();
        assert_eq!(b_links.len(), 1);
        assert_eq!(b_links[0].relation, Relation::RelatedTo);
        assert_eq!(b_links[0].target, "a");
    }

    #[test]
    fn does_not_link_topics_with_no_shared_product() {
        let mut nodes = vec![topic("a", &["enterprise"]), topic("b", &["community"])];
        assert_eq!(infer_related_to(&mut nodes), 0);
        assert!(nodes[0].links().is_empty());
        assert!(nodes[1].links().is_empty());
    }

    #[test]
    fn does_not_link_topics_with_no_product_at_all() {
        let mut nodes = vec![topic("a", &[]), topic("b", &[])];
        assert_eq!(infer_related_to(&mut nodes), 0);
    }

    #[test]
    fn skips_pairs_already_connected_by_another_relation() {
        let mut nodes = vec![topic("a", &["enterprise"]), topic("b", &["enterprise"])];
        if let NormalizedNode::Topic(t) = &mut nodes[0] {
            t.links.push(Link {
                relation: Relation::Requires,
                target: "b".into(),
            });
        }
        assert_eq!(
            infer_related_to(&mut nodes),
            0,
            "an existing requires edge should take precedence over the inferred related-to"
        );
        assert_eq!(nodes[0].links().len(), 1);
        assert_eq!(nodes[0].links()[0].relation, Relation::Requires);
        assert!(nodes[1].links().is_empty());
    }

    #[test]
    fn skips_pairs_already_connected_in_the_reverse_direction() {
        let mut nodes = vec![topic("a", &["enterprise"]), topic("b", &["enterprise"])];
        if let NormalizedNode::Topic(t) = &mut nodes[1] {
            t.links.push(Link {
                relation: Relation::References,
                target: "a".into(),
            });
        }
        assert_eq!(infer_related_to(&mut nodes), 0);
    }

    #[test]
    fn maps_are_never_considered() {
        let mut nodes = vec![
            NormalizedNode::Map(NormalizedMap {
                id: "guide".into(),
                title: "Guide".into(),
                source_file: "guide.ditamap".into(),
                links: vec![],
            }),
            topic("a", &["enterprise"]),
        ];
        assert_eq!(infer_related_to(&mut nodes), 0);
    }

    #[test]
    fn matches_on_any_shared_value_when_a_topic_has_multiple_products() {
        let mut nodes = vec![
            topic("a", &["enterprise", "community"]),
            topic("b", &["community", "cloud"]),
        ];
        assert_eq!(infer_related_to(&mut nodes), 2);
    }

    #[test]
    fn three_topics_sharing_a_product_each_get_a_pairwise_edge() {
        let mut nodes = vec![
            topic("a", &["enterprise"]),
            topic("b", &["enterprise"]),
            topic("c", &["enterprise"]),
        ];
        assert_eq!(infer_related_to(&mut nodes), 6);
        for node in &nodes {
            assert_eq!(node.links().len(), 2);
        }
    }

    #[test]
    fn unambiguous_uicontrol_match_creates_an_applies_to_edge() {
        let mut nodes = vec![
            task_topic("save-task", &["Save"]),
            reference_topic("ui-reference", &["Save"]),
        ];
        assert_eq!(infer_applies_to(&mut nodes), 1);
        assert_eq!(nodes[0].links().len(), 1);
        assert_eq!(nodes[0].links()[0].relation, Relation::AppliesTo);
        assert_eq!(nodes[0].links()[0].target, "ui-reference");
        assert!(
            nodes[1].links().is_empty(),
            "applies-to is directional: task -> reference only"
        );
    }

    #[test]
    fn no_edge_when_no_reference_topic_documents_the_uicontrol() {
        let mut nodes = vec![
            task_topic("save-task", &["Save"]),
            reference_topic("ui-reference", &["Cancel"]),
        ];
        assert_eq!(infer_applies_to(&mut nodes), 0);
    }

    #[test]
    fn ambiguous_uicontrol_match_across_two_reference_topics_is_dropped() {
        let mut nodes = vec![
            task_topic("save-task", &["Cancel"]),
            reference_topic("ui-reference", &["Cancel"]),
            reference_topic("other-ui-reference", &["Cancel"]),
        ];
        assert_eq!(
            infer_applies_to(&mut nodes),
            0,
            "two candidate reference topics for the same term must be dropped, not guessed"
        );
        assert!(nodes[0].links().is_empty());
    }

    #[test]
    fn one_unambiguous_term_and_one_ambiguous_term_on_the_same_task_only_the_unambiguous_one_links()
    {
        let mut nodes = vec![
            task_topic("save-task", &["Save", "Cancel"]),
            reference_topic("ui-reference", &["Save", "Cancel"]),
            reference_topic("other-ui-reference", &["Cancel"]),
        ];
        assert_eq!(infer_applies_to(&mut nodes), 1);
        assert_eq!(nodes[0].links().len(), 1);
        assert_eq!(nodes[0].links()[0].target, "ui-reference");
    }

    #[test]
    fn multiple_terms_resolving_to_the_same_reference_produce_one_edge_not_two() {
        let mut nodes = vec![
            task_topic("save-task", &["Save", "Discard"]),
            reference_topic("ui-reference", &["Save", "Discard"]),
        ];
        assert_eq!(infer_applies_to(&mut nodes), 1);
        assert_eq!(nodes[0].links().len(), 1);
    }

    #[test]
    fn only_cmd_scoped_uicontrols_count_as_source_terms() {
        // "Save" is in the task's whole-body uicontrols (e.g. a <result>
        // paragraph mentioning it) but not cmd_uicontrols -- a casual
        // mention outside <cmd> must not trigger applies-to.
        let mut nodes = vec![
            NormalizedNode::Topic(NormalizedTopic {
                id: "save-task".into(),
                topic_type: TopicType::Task,
                title: "save-task".into(),
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec!["Save".into()],
                cmd_uicontrols: vec![],
                source_file: "topics/save-task.dita".into(),
                links: vec![],
            }),
            reference_topic("ui-reference", &["Save"]),
        ];
        assert_eq!(infer_applies_to(&mut nodes), 0);
    }

    #[test]
    fn only_reference_topics_count_as_valid_targets() {
        // A Concept topic happening to mention "Save" as a uicontrol
        // must not become an applies-to target -- only Reference topics
        // do, matching §3.3's wording precisely.
        let mut nodes = vec![
            task_topic("save-task", &["Save"]),
            NormalizedNode::Topic(NormalizedTopic {
                id: "some-concept".into(),
                topic_type: TopicType::Concept,
                title: "some-concept".into(),
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec!["Save".into()],
                cmd_uicontrols: vec![],
                source_file: "topics/some-concept.dita".into(),
                links: vec![],
            }),
        ];
        assert_eq!(infer_applies_to(&mut nodes), 0);
    }

    #[test]
    fn skips_a_pair_already_connected_by_another_relation() {
        let mut nodes = vec![
            task_topic("save-task", &["Save"]),
            reference_topic("ui-reference", &["Save"]),
        ];
        if let NormalizedNode::Topic(t) = &mut nodes[0] {
            t.links.push(Link {
                relation: Relation::References,
                target: "ui-reference".into(),
            });
        }
        assert_eq!(
            infer_applies_to(&mut nodes),
            0,
            "an existing references edge should take precedence over the inferred applies-to"
        );
        assert_eq!(nodes[0].links().len(), 1);
        assert_eq!(nodes[0].links()[0].relation, Relation::References);
    }
}
