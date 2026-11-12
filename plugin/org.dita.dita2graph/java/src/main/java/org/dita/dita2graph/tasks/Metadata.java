package org.dita.dita2graph.tasks;

import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;

import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/**
 * A topic's or a map's metadata as DITA-OT's resolved output carries it
 * ({@code docs/semantic-model.md}, "DITA metadata"): a topic's prolog, or a map's
 * own {@code topicmeta}/{@code bookmeta}.
 *
 * <p>DITA-OT's preprocessing already cascades a map's metadata into the
 * topics it references, as DITA defines the cascade (confirmed on a DITA-OT
 * 4.4.1 run: the root map's {@code critdates} and {@code prodinfo} and a
 * topicref's {@code audience}, {@code permissions} and {@code keywords} end
 * up in the resolved topic's prolog). So this reads what the resolved
 * topic holds and cascades nothing itself.
 *
 * <p>Index terms come from the whole topic, prolog and body, each as the
 * path from its top level to an entry ({@code ["languages", "adding support
 * for"]}): the publication's index, topic by topic.
 *
 * <p>Elements are recognized by their {@code @class} token, so a
 * specialization counts, and by their name when a hand-written document
 * has no {@code @class}.
 */
final class Metadata {
    String created;
    /** The most recent {@code revised/@modified}. */
    String modified;
    /** {@code @golive} of the most recent revision that has one, else of {@code created}. */
    String golive;
    /** {@code @expiry}, chosen like {@link #golive}. */
    String expiry;
    /** {@code permissions/@view}. */
    String permissions;
    /** {name, type}: type is {@code creator}, {@code contributor} or empty. */
    final List<String[]> authors = new ArrayList<>();
    /** {name, version}: version joins {@code vrm} version, release and modification, or is null. */
    final List<String[]> products = new ArrayList<>();
    /** {type, job, experienceLevel}, any of them null. */
    final List<String[]> audiences = new ArrayList<>();
    final Set<String> keywords = new LinkedHashSet<>();
    final Set<List<String>> indexTerms = new LinkedHashSet<>();

    /** The metadata of a topic: its prolog, and the index terms of the whole topic. */
    static Metadata ofTopic(Element root) {
        Metadata m = new Metadata();
        for (Element prolog : children(root, "topic/prolog")) {
            m.read(prolog);
        }
        m.readIndexTerms(root);
        return m;
    }

    /** The metadata of a map: its own {@code topicmeta}, or a bookmap's {@code bookmeta}. */
    static Metadata ofMap(Element root) {
        Metadata m = new Metadata();
        for (Element meta : children(root, "map/topicmeta")) {
            m.read(meta);
        }
        return m;
    }

    boolean isEmpty() {
        return created == null && modified == null && golive == null && expiry == null && permissions == null
                && authors.isEmpty() && products.isEmpty() && audiences.isEmpty() && keywords.isEmpty()
                && indexTerms.isEmpty();
    }

    private void read(Element container) {
        for (Element critdates : descendants(container, "topic/critdates")) {
            for (Element c : children(critdates, "topic/created")) {
                created = firstNonEmpty(created, c.getAttribute("date"));
                golive = firstNonEmpty(golive, c.getAttribute("golive"));
                expiry = firstNonEmpty(expiry, c.getAttribute("expiry"));
            }
            // DITA lists revisions oldest first: the last one with a value wins.
            for (Element r : children(critdates, "topic/revised")) {
                modified = lastNonEmpty(modified, r.getAttribute("modified"));
                golive = lastNonEmpty(golive, r.getAttribute("golive"));
                expiry = lastNonEmpty(expiry, r.getAttribute("expiry"));
            }
        }
        for (Element author : descendants(container, "topic/author")) {
            String name = text(author);
            if (!name.isEmpty()) {
                authors.add(new String[] {name, author.getAttribute("type")});
            }
        }
        for (Element p : descendants(container, "topic/permissions")) {
            permissions = firstNonEmpty(permissions, p.getAttribute("view"));
        }
        for (Element audience : descendants(container, "topic/audience")) {
            String type = other(audience, "type", "othertype");
            String job = other(audience, "job", "otherjob");
            String level = emptyToNull(audience.getAttribute("experiencelevel"));
            if (type != null || job != null || level != null) {
                audiences.add(new String[] {type, job, level});
            }
        }
        for (Element keywordsEl : descendants(container, "topic/keywords")) {
            for (Element keyword : children(keywordsEl, "topic/keyword")) {
                String k = text(keyword);
                if (!k.isEmpty()) {
                    keywords.add(k);
                }
            }
        }
        for (Element prodinfo : descendants(container, "topic/prodinfo")) {
            String name = "";
            for (Element prodname : descendants(prodinfo, "topic/prodname")) {
                name = name.isEmpty() ? text(prodname) : name;
            }
            if (name.isEmpty()) {
                continue;
            }
            List<String> versions = new ArrayList<>();
            for (Element vrm : descendants(prodinfo, "topic/vrm")) {
                List<String> parts = new ArrayList<>();
                for (String a : new String[] {"version", "release", "modification"}) {
                    String v = vrm.getAttribute(a).trim();
                    if (v.isEmpty()) {
                        break;
                    }
                    parts.add(v);
                }
                if (!parts.isEmpty()) {
                    versions.add(String.join(".", parts));
                }
            }
            if (versions.isEmpty()) {
                products.add(new String[] {name, null});
            }
            for (String version : versions) {
                products.add(new String[] {name, version});
            }
        }
    }

    /** Every index entry of the topic, as the path from its top-level term. */
    private void readIndexTerms(Element root) {
        for (Element term : descendants(root, "topic/indexterm")) {
            if (!(term.getParentNode() instanceof Element parent && is(parent, "topic/indexterm"))) {
                addIndexPaths(term, new ArrayList<>());
            }
        }
    }

    private void addIndexPaths(Element term, List<String> above) {
        String label = indexLabel(term);
        if (label.isEmpty()) {
            return;
        }
        List<String> path = new ArrayList<>(above);
        path.add(label);
        List<Element> subterms = children(term, "topic/indexterm");
        if (subterms.isEmpty()) {
            indexTerms.add(List.copyOf(path));
        }
        for (Element subterm : subterms) {
            addIndexPaths(subterm, path);
        }
    }

    /** An index term's own words: its text without its subterms, see/see-also references and sort key. */
    private static String indexLabel(Element term) {
        StringBuilder sb = new StringBuilder();
        appendLabelText(term, sb);
        return normalize(sb);
    }

    private static void appendLabelText(Node node, StringBuilder sb) {
        NodeList children = node.getChildNodes();
        for (int i = 0; i < children.getLength(); i++) {
            Node child = children.item(i);
            if (child.getNodeType() == Node.TEXT_NODE || child.getNodeType() == Node.CDATA_SECTION_NODE) {
                sb.append(child.getNodeValue());
            } else if (child instanceof Element e && !is(e, "topic/indexterm") && !is(e, "topic/index-base")
                    && !is(e, "indexing-d/index-see") && !is(e, "indexing-d/index-see-also")
                    && !is(e, "indexing-d/index-sort-as")) {
                appendLabelText(e, sb);
            }
        }
    }

    String toJson() {
        StringBuilder sb = new StringBuilder("{");
        String sep = "";
        for (String[] field : new String[][] {{"created", created}, {"modified", modified}, {"golive", golive},
                {"expiry", expiry}, {"permissions", permissions}}) {
            if (field[1] != null) {
                sb.append(sep).append(Json.string(field[0])).append(':').append(Json.string(field[1]));
                sep = ",";
            }
        }
        if (!authors.isEmpty()) {
            sb.append(sep).append("\"authors\":").append(objects(authors, "name", "type"));
            sep = ",";
        }
        if (!products.isEmpty()) {
            sb.append(sep).append("\"products\":").append(objects(products, "name", "version"));
            sep = ",";
        }
        if (!audiences.isEmpty()) {
            sb.append(sep).append("\"audiences\":").append(objects(audiences, "type", "job", "experienceLevel"));
            sep = ",";
        }
        if (!keywords.isEmpty()) {
            sb.append(sep).append("\"keywords\":").append(Json.stringArray(new ArrayList<>(keywords)));
            sep = ",";
        }
        if (!indexTerms.isEmpty()) {
            sb.append(sep).append("\"indexTerms\":[");
            String comma = "";
            for (List<String> path : indexTerms) {
                sb.append(comma).append(Json.stringArray(path));
                comma = ",";
            }
            sb.append(']');
        }
        return sb.append('}').toString();
    }

    /** JSON objects with these keys, a null or empty value left out. */
    private static String objects(List<String[]> rows, String... keys) {
        StringBuilder sb = new StringBuilder("[");
        for (int r = 0; r < rows.size(); r++) {
            sb.append(r == 0 ? "{" : ",{");
            String sep = "";
            for (int k = 0; k < keys.length; k++) {
                String value = rows.get(r)[k];
                if (value != null && !value.isEmpty()) {
                    sb.append(sep).append(Json.string(keys[k])).append(':').append(Json.string(value));
                    sep = ",";
                }
            }
            sb.append('}');
        }
        return sb.append(']').toString();
    }

    // -- XML helpers --------------------------------------------------------

    /** Whether the element is (a specialization of) {@code token}, e.g. {@code topic/indexterm}. */
    static boolean is(Element el, String token) {
        String cls = el.getAttribute("class");
        if (cls.isEmpty()) {
            return el.getTagName().equals(token.substring(token.indexOf('/') + 1));
        }
        return (" " + cls + " ").contains(" " + token + " ");
    }

    private static List<Element> children(Element parent, String token) {
        List<Element> result = new ArrayList<>();
        NodeList nodes = parent.getChildNodes();
        for (int i = 0; i < nodes.getLength(); i++) {
            if (nodes.item(i) instanceof Element e && is(e, token)) {
                result.add(e);
            }
        }
        return result;
    }

    private static List<Element> descendants(Element root, String token) {
        List<Element> result = new ArrayList<>();
        NodeList nodes = root.getElementsByTagName("*");
        for (int i = 0; i < nodes.getLength(); i++) {
            Element e = (Element) nodes.item(i);
            if (is(e, token)) {
                result.add(e);
            }
        }
        return result;
    }

    private static String text(Element el) {
        return normalize(el.getTextContent());
    }

    private static String normalize(CharSequence s) {
        return s == null ? "" : s.toString().replaceAll("\\s+", " ").trim();
    }

    private static String other(Element el, String name, String otherName) {
        String value = el.getAttribute(name).trim();
        if ("other".equals(value) && !el.getAttribute(otherName).trim().isEmpty()) {
            return el.getAttribute(otherName).trim();
        }
        return emptyToNull(value);
    }

    private static String emptyToNull(String s) {
        return s == null || s.trim().isEmpty() ? null : s.trim();
    }

    private static String firstNonEmpty(String current, String candidate) {
        return current != null ? current : emptyToNull(candidate);
    }

    private static String lastNonEmpty(String current, String candidate) {
        String c = emptyToNull(candidate);
        return c != null ? c : current;
    }
}
