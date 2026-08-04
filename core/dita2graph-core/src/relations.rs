//! Relation inference (`docs/plugin-specification.md` §3.3, §4.3):
//! deriving edges DITA doesn't state explicitly, as opposed to the
//! `contains`/`requires`/`references` edges `DitaModelExtractor` already
//! reads directly off authored markup.
//!
//! Only `related-to` is implemented so far -- the §3.3 example "topics
//! sharing a `product` value are `related-to`" is directly computable
//! from data already flowing through the normalized model
//! (`NormalizedTopic::product`), unlike `applies-to` (needs `<uicontrol>`
//! extraction the Java side doesn't do yet) or `generated-from` (needs
//! `conref`/`conkeyref` provenance tracking, also not implemented). Both
//! remain documented gaps rather than guessed at (`docs/dev/
//! phase-0-findings.md`).

use crate::model::{Link, NormalizedNode, Relation};
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
}
