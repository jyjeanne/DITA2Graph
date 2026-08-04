package org.dita.dita2graph.tasks;

import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;

import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;
import java.io.File;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Reads DITA-OT's resolved {@code .job.xml} and temp-directory topics/maps
 * and builds the normalized DITA model (docs/plugin-specification.md §3.2).
 *
 * <p>Deliberately not an Ant {@code Task} itself -- {@link ExtractTask}
 * is the thin Ant-facing wrapper; this class takes a plain directory and
 * a warning sink, so it can be unit tested without a full Ant
 * {@code Project} context.
 *
 * <p>Relation extraction is intentionally limited to what's directly,
 * deterministically derivable from DITA markup, per the "verify before
 * you infer" discipline this codebase has followed throughout (see
 * {@code docs/dev/phase-0-findings.md}):
 * <ul>
 *   <li>every {@code <topicref>} anywhere in the map tree (nested
 *       arbitrarily deep, through {@code <topicref>}, {@code
 *       <topichead>}, and {@code <topicgroup>} ancestors) with a
 *       resolvable {@code href} -&gt; {@code contains}, attached to its
 *       *nearest* containing topic (the map itself for a top-level
 *       {@code topicref}, or the closest ancestor {@code topicref}'s own
 *       target topic for a nested one) -- {@code topichead}/{@code
 *       topicgroup} and a bare, href-less {@code topicref} have no
 *       topic of their own, so a {@code contains} edge skips through
 *       them to the nearest real container, per real DITA map
 *       semantics (a chapter containing sections, not a flat list);
 *   <li>an {@code <xref>}/{@code <link>} carrying a {@code keyref}
 *       attribute -&gt; {@code requires} (using a key rather than a bare
 *       {@code href} signals an intentional, named dependency);
 *   <li>any other local {@code <xref>}/{@code <link>} -&gt; {@code references}.
 * </ul>
 * {@code <navref>/<anchorref>/<mapref>} (map composition via a separate
 * navigation/anchor/sub-map mechanism, not simple containment) are
 * deliberately out of scope, same discipline.
 * {@code applies-to}, {@code related-to}, and {@code generated-from} all
 * require heuristic inference this extractor doesn't attempt (§3.3); they
 * never appear in its output.
 *
 * <p>Each topic's body element ({@code conbody}/{@code taskbody}/{@code
 * refbody}/{@code glossdef}/generic {@code body}, per {@link
 * #bodyElementTag}) is also captured as whitespace-normalized plain text
 * -- markup stripped, nothing else cleaned up -- for both the OKF
 * bundle's body content and the RAG index (§4.4, §13.1).
 *
 * <p>{@code maxDepth} (§2.3's {@code args.dita2graph.depth}) limits how
 * many levels of *real* map containment the {@code contains} edges
 * captured in the graph go -- level 1 is a top-level {@code topicref},
 * level 2 is one nested inside that, and so on. {@code topichead}/{@code
 * topicgroup}/an href-less {@code topicref} don't consume a level, since
 * they never become a node in the graph themselves (this counts depth
 * in the *resulting graph*, not raw XML nesting). A topic beyond the
 * limit is still extracted as its own node (still a real topic DITA-OT
 * resolved) -- only its {@code contains} edge from its parent is
 * omitted, the same graceful-degradation choice as an unresolved
 * topicref target (§2.5's {@code DITA2GRAPH010W}).
 */
final class DitaModelExtractor {

    private final File tempDir;
    private final boolean includeDrafts;
    private final int maxDepth;
    private final Consumer<String> warn;
    private final Consumer<String> info;

    DitaModelExtractor(File tempDir, boolean includeDrafts, Consumer<String> warn, Consumer<String> info) {
        this(tempDir, includeDrafts, Integer.MAX_VALUE, warn, info);
    }

    DitaModelExtractor(File tempDir, boolean includeDrafts, int maxDepth, Consumer<String> warn, Consumer<String> info) {
        this.tempDir = tempDir;
        this.includeDrafts = includeDrafts;
        this.maxDepth = maxDepth;
        this.warn = warn;
        this.info = info;
    }

    /** One {@code <file>} entry from job.xml. */
    private static final class JobFile {
        String path;
        String format;
        boolean input;
    }

    /** Raw, unresolved xref/link found while parsing a topic, resolved in a second pass. */
    private static final class RawLink {
        final String sourceTopicPath;
        final String href;
        final boolean hasKeyref;

        RawLink(String sourceTopicPath, String href, boolean hasKeyref) {
            this.sourceTopicPath = sourceTopicPath;
            this.href = href;
            this.hasKeyref = hasKeyref;
        }
    }

    /**
     * One "container contains this topicref's target" record collected
     * while recursively walking the map tree ({@link #walkMapChildren}),
     * resolved into a real {@code contains} edge once every topic's id
     * is known. {@code containerPath} is {@code null} for a top-level
     * {@code topicref} (the map itself is the container); otherwise it's
     * the resolved path of the nearest ancestor {@code topicref}'s own
     * target topic.
     */
    private static final class RawContainment {
        final String containerPath;
        final String targetPath;

        RawContainment(String containerPath, String targetPath) {
            this.containerPath = containerPath;
            this.targetPath = targetPath;
        }
    }

    List<Object> extract() throws Exception {
        File jobXml = findJobXml();
        List<JobFile> files = parseJobXml(jobXml);

        JobFile mapFile = files.stream()
                .filter(f -> "ditamap".equals(f.format) && f.input)
                .findFirst()
                .orElseGet(() -> files.stream()
                        .filter(f -> "ditamap".equals(f.format))
                        .findFirst()
                        .orElse(null));
        if (mapFile == null) {
            throw new IllegalStateException("no ditamap file found in " + jobXml);
        }

        DocumentBuilder builder = newDocumentBuilder();

        // Pass 1: parse the map for its own metadata and topicref hrefs
        // (resolved in pass 3, once every topic's real id is known).
        Document mapDoc = parse(builder, new File(tempDir, mapFile.path));
        Element mapRoot = mapDoc.getDocumentElement();
        MapNode mapNode = new MapNode();
        mapNode.id = attr(mapRoot, "id", stem(mapFile.path));
        mapNode.title = childText(mapRoot, "title", mapNode.id);
        mapNode.sourceFile = mapFile.path;

        List<RawContainment> containments = new ArrayList<>();
        Map<String, List<String>> keysByPath = new HashMap<>();
        walkMapChildren(mapRoot, mapFile.path, null, 1, containments, keysByPath);

        // Pass 2: parse every topic file for its own metadata, deferring
        // link resolution (target ids may not be known yet).
        Map<String, TopicNode> topicsByPath = new LinkedHashMap<>();
        List<RawLink> rawLinks = new ArrayList<>();
        for (JobFile file : files) {
            if (!"dita".equals(file.format)) {
                continue;
            }
            Document topicDoc = parse(builder, new File(tempDir, file.path));
            Element root = topicDoc.getDocumentElement();

            TopicNode topic = new TopicNode();
            topic.id = attr(root, "id", stem(file.path));
            topic.topicType = mapTopicType(root.getTagName());
            if ("topic".equals(topic.topicType) && !root.getTagName().equals("topic")) {
                warn.accept("DITA2GRAPH040W: " + file.path + " has unrecognized topic type <"
                        + root.getTagName() + ">; emitting as generic Topic (§4.1)");
            }
            topic.title = childText(root, "title", topic.id);
            String shortdesc = directChildText(root, "shortdesc");
            topic.shortdesc = shortdesc.isEmpty() ? null : shortdesc;
            String body = bodyText(root, topic.topicType);
            topic.body = body.isEmpty() ? null : body;
            topic.audience.addAll(splitAttr(root, "audience"));
            topic.product.addAll(splitAttr(root, "product"));
            topic.sourceFile = file.path;
            topic.status = root.getAttribute("status");
            List<String> keys = keysByPath.get(file.path);
            if (keys != null) {
                topic.keys.addAll(keys);
            }

            for (Element xref : descendants(root, "xref", "link")) {
                if (isInsideRelatedLinks(xref)) {
                    // DITA-OT's own preprocessing auto-generates a
                    // <related-links> block of parent/child/sibling
                    // navigation <link> elements from the map hierarchy
                    // itself (confirmed directly: a genuinely nested
                    // topicref/topichead/topicgroup map produces these
                    // in every affected topic's resolved output, even
                    // though sample-docs/'s flat map never triggers it
                    // at all). These are auto-generated TOC navigation,
                    // not authored cross-references -- extracting them
                    // as "references"/"requires" edges would mislabel
                    // generated navigation as author intent, the exact
                    // "guessy" inference this extractor otherwise avoids.
                    continue;
                }
                String href = xref.getAttribute("href");
                String scope = xref.getAttribute("scope");
                if (href.isEmpty() || href.startsWith("http://") || href.startsWith("https://")
                        || "external".equals(scope) || "peer".equals(scope)) {
                    continue;
                }
                String fragmentless = href.contains("#") ? href.substring(0, href.indexOf('#')) : href;
                if (fragmentless.isEmpty()) {
                    continue; // same-topic fragment reference
                }
                boolean hasKeyref = !xref.getAttribute("keyref").isEmpty();
                rawLinks.add(new RawLink(file.path, resolve(file.path, fragmentless), hasKeyref));
            }

            topicsByPath.put(file.path, topic);
        }

        // Pass 3: resolve map topicrefs and topic-body links now that
        // every topic's real id is known.
        for (RawContainment containment : containments) {
            TopicNode target = topicsByPath.get(containment.targetPath);
            if (target == null) {
                warn.accept("DITA2GRAPH010W: unresolved topicref target " + containment.targetPath
                        + " in " + mapFile.path);
                continue;
            }
            if (containment.containerPath == null) {
                mapNode.links.add(new Link("contains", target.id));
                continue;
            }
            TopicNode container = topicsByPath.get(containment.containerPath);
            if (container == null) {
                // The containing topicref's own target never resolved;
                // already warned about that when its own RawContainment
                // record was processed above.
                continue;
            }
            container.links.add(new Link("contains", target.id));
        }
        for (RawLink raw : rawLinks) {
            TopicNode source = topicsByPath.get(raw.sourceTopicPath);
            TopicNode target = topicsByPath.get(raw.href);
            if (source == null || target == null) {
                warn.accept("DITA2GRAPH010W: unresolved link target " + raw.href + " in " + raw.sourceTopicPath);
                continue;
            }
            if (source == target) {
                continue;
            }
            source.links.add(new Link(raw.hasKeyref ? "requires" : "references", target.id));
        }

        List<Object> nodes = new ArrayList<>();
        nodes.add(mapNode);
        for (TopicNode topic : topicsByPath.values()) {
            if (!includeDrafts && "draft".equals(topic.status)) {
                info.accept("DITA2GRAPH020I: skipping " + topic.sourceFile + " (status=\"draft\")");
                continue;
            }
            nodes.add(topic);
        }
        return nodes;
    }

    // ==================== job.xml ====================

    private File findJobXml() {
        File hidden = new File(tempDir, ".job.xml");
        if (hidden.isFile()) {
            return hidden;
        }
        File plain = new File(tempDir, "job.xml");
        if (plain.isFile()) {
            return plain;
        }
        throw new IllegalStateException("no .job.xml/job.xml found under " + tempDir);
    }

    private List<JobFile> parseJobXml(File jobXml) throws Exception {
        Document doc = parse(newDocumentBuilder(), jobXml);
        List<JobFile> files = new ArrayList<>();
        NodeList fileNodes = doc.getElementsByTagName("file");
        for (int i = 0; i < fileNodes.getLength(); i++) {
            Element el = (Element) fileNodes.item(i);
            JobFile f = new JobFile();
            f.path = el.getAttribute("path");
            f.format = el.getAttribute("format");
            f.input = "true".equals(el.getAttribute("input"));
            files.add(f);
        }
        return files;
    }

    // ==================== XML helpers ====================

    private DocumentBuilder newDocumentBuilder() throws Exception {
        DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
        factory.setNamespaceAware(true);
        // DITA-OT's resolved output declares a <!DOCTYPE ... PUBLIC ...>
        // prolog; allow it (roxmltree/DitaLinkCheckTask do the same) but
        // disable external entity/DTD resolution -- this is a purely
        // local, non-validating parse, never fetching anything over the
        // network or filesystem to resolve the doctype.
        factory.setFeature("http://apache.org/xml/features/nonvalidating/load-external-dtd", false);
        factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
        factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
        return factory.newDocumentBuilder();
    }

    private Document parse(DocumentBuilder builder, File file) throws Exception {
        return builder.parse(file);
    }

    /**
     * Recursively walks {@code parent}'s element children looking for
     * {@code <topicref>}/{@code <topichead>}/{@code <topicgroup>},
     * collecting one {@link RawContainment} per {@code topicref} that
     * has an {@code href} and descending into every one of the three
     * (arbitrarily deep, up to {@link #maxDepth}) so nested map
     * structures are captured, not just the top level. {@code
     * containerPath} is the resolved path of the nearest containing
     * real topic so far ({@code null} means "the map itself" -- true
     * only before the first real topicref is seen); {@code
     * topichead}/{@code topicgroup}/an href-less {@code topicref} have
     * no topic of their own, so their children keep the *same* {@code
     * containerPath} and {@code level}, skipping through them.
     */
    private void walkMapChildren(Element parent, String mapPath, String containerPath, int level,
            List<RawContainment> containments, Map<String, List<String>> keysByPath) {
        NodeList children = parent.getChildNodes();
        for (int i = 0; i < children.getLength(); i++) {
            Node n = children.item(i);
            if (n.getNodeType() != Node.ELEMENT_NODE) {
                continue;
            }
            Element child = (Element) n;
            switch (child.getNodeName()) {
                case "topicref": {
                    String href = child.getAttribute("href");
                    if (href.isEmpty()) {
                        walkMapChildren(child, mapPath, containerPath, level, containments, keysByPath);
                        break;
                    }
                    if (level > maxDepth) {
                        // Beyond args.dita2graph.depth (§2.3): the topic
                        // itself is still extracted as a node (Pass 2
                        // parses every resolved "dita" file regardless),
                        // just without a contains edge from its parent.
                        break;
                    }
                    String targetPath = resolve(mapPath, href);
                    containments.add(new RawContainment(containerPath, targetPath));
                    String keys = child.getAttribute("keys");
                    if (!keys.isEmpty()) {
                        keysByPath.put(targetPath, List.of(keys.trim().split("\\s+")));
                    }
                    walkMapChildren(child, mapPath, targetPath, level + 1, containments, keysByPath);
                    break;
                }
                case "topichead":
                case "topicgroup":
                    walkMapChildren(child, mapPath, containerPath, level, containments, keysByPath);
                    break;
                default:
                    break;
            }
        }
    }

    /** Whether {@code el} has a {@code <related-links>} ancestor (see the caller). */
    private static boolean isInsideRelatedLinks(Element el) {
        Node n = el.getParentNode();
        while (n != null) {
            if (n.getNodeType() == Node.ELEMENT_NODE && "related-links".equals(n.getNodeName())) {
                return true;
            }
            n = n.getParentNode();
        }
        return false;
    }

    private static List<Element> directChildren(Element parent, String tagName) {
        List<Element> result = new ArrayList<>();
        NodeList children = parent.getChildNodes();
        for (int i = 0; i < children.getLength(); i++) {
            Node n = children.item(i);
            if (n.getNodeType() == Node.ELEMENT_NODE && tagName.equals(n.getNodeName())) {
                result.add((Element) n);
            }
        }
        return result;
    }

    private static List<Element> descendants(Element root, String... tagNames) {
        List<Element> result = new ArrayList<>();
        for (String tag : tagNames) {
            NodeList nodes = root.getElementsByTagName(tag);
            for (int i = 0; i < nodes.getLength(); i++) {
                result.add((Element) nodes.item(i));
            }
        }
        return result;
    }

    private static String attr(Element el, String name, String fallback) {
        String value = el.getAttribute(name);
        return value.isEmpty() ? fallback : value;
    }

    private static List<String> splitAttr(Element el, String name) {
        String value = el.getAttribute(name);
        if (value.isEmpty()) {
            return List.of();
        }
        return List.of(value.trim().split("\\s+"));
    }

    private static String childText(Element parent, String tagName, String fallback) {
        for (Element child : directChildren(parent, tagName)) {
            String text = child.getTextContent();
            if (text != null && !text.trim().isEmpty()) {
                return text.trim();
            }
        }
        return fallback;
    }

    private static String directChildText(Element parent, String tagName) {
        for (Element child : directChildren(parent, tagName)) {
            String text = child.getTextContent();
            if (text != null) {
                return text.trim();
            }
        }
        return "";
    }

    /**
     * The topic's body element's full text content, markup stripped and
     * whitespace collapsed to single spaces -- the "cleaned text" input
     * for the RAG index (docs/plugin-specification.md §13.1). {@code
     * shortdesc} is a separate sibling element in DITA and is not
     * included here (see {@link #directChildText} for that).
     */
    private static String bodyText(Element root, String topicType) {
        String tag = bodyElementTag(topicType);
        for (Element child : directChildren(root, tag)) {
            String text = child.getTextContent();
            if (text == null) {
                return "";
            }
            return text.trim().replaceAll("\\s+", " ");
        }
        return "";
    }

    private static String bodyElementTag(String topicType) {
        switch (topicType) {
            case "concept":
                return "conbody";
            case "task":
                return "taskbody";
            case "reference":
                return "refbody";
            case "glossentry":
                return "glossdef";
            default:
                return "body";
        }
    }

    private static String mapTopicType(String rootTagName) {
        switch (rootTagName) {
            case "concept":
                return "concept";
            case "task":
                return "task";
            case "reference":
                return "reference";
            case "glossentry":
                return "glossentry";
            default:
                return "topic";
        }
    }

    /** Resolves {@code href} relative to {@code fromPath}'s directory, POSIX-style (job.xml paths always are). */
    private static String resolve(String fromPath, String href) {
        Path base = Paths.get(fromPath).getParent();
        Path resolved = base == null ? Paths.get(href) : base.resolve(href).normalize();
        return resolved.toString().replace(File.separatorChar, '/');
    }

    private static String stem(String path) {
        String name = path.contains("/") ? path.substring(path.lastIndexOf('/') + 1) : path;
        int dot = name.lastIndexOf('.');
        return dot > 0 ? name.substring(0, dot) : name;
    }
}
