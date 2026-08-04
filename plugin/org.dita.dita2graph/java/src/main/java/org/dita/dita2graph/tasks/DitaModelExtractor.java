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
 *   <li>a map's direct {@code <topicref>} children -&gt; {@code contains}
 *       (deep/nested {@code topicref}, {@code topichead}, and
 *       {@code topicgroup} are not walked -- future work, §13);
 *   <li>an {@code <xref>}/{@code <link>} carrying a {@code keyref}
 *       attribute -&gt; {@code requires} (using a key rather than a bare
 *       {@code href} signals an intentional, named dependency);
 *   <li>any other local {@code <xref>}/{@code <link>} -&gt; {@code references}.
 * </ul>
 * {@code applies-to}, {@code related-to}, and {@code generated-from} all
 * require heuristic inference this extractor doesn't attempt (§3.3); they
 * never appear in its output.
 *
 * <p>Each topic's body element ({@code conbody}/{@code taskbody}/{@code
 * refbody}/{@code glossdef}/generic {@code body}, per {@link
 * #bodyElementTag}) is also captured as whitespace-normalized plain text
 * -- markup stripped, nothing else cleaned up -- for both the OKF
 * bundle's body content and the RAG index (§4.4, §13.1).
 */
final class DitaModelExtractor {

    private final File tempDir;
    private final boolean includeDrafts;
    private final Consumer<String> warn;
    private final Consumer<String> info;

    DitaModelExtractor(File tempDir, boolean includeDrafts, Consumer<String> warn, Consumer<String> info) {
        this.tempDir = tempDir;
        this.includeDrafts = includeDrafts;
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

        List<String> topicrefHrefs = new ArrayList<>();
        for (Element topicref : directChildren(mapRoot, "topicref")) {
            String href = topicref.getAttribute("href");
            if (!href.isEmpty()) {
                topicrefHrefs.add(resolve(mapFile.path, href));
            }
        }
        Map<String, List<String>> keysByPath = new HashMap<>();
        for (Element topicref : directChildren(mapRoot, "topicref")) {
            String href = topicref.getAttribute("href");
            String keys = topicref.getAttribute("keys");
            if (!href.isEmpty() && !keys.isEmpty()) {
                keysByPath.put(resolve(mapFile.path, href), List.of(keys.trim().split("\\s+")));
            }
        }

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
        for (String targetPath : topicrefHrefs) {
            TopicNode target = topicsByPath.get(targetPath);
            if (target == null) {
                warn.accept("DITA2GRAPH010W: unresolved topicref target " + targetPath + " in " + mapFile.path);
                continue;
            }
            mapNode.links.add(new Link("contains", target.id));
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
