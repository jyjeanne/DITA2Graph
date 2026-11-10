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
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
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
 *   <li>any other local {@code <xref>}/{@code <link>} -&gt; {@code references};
 *   <li>a {@code <link>} in {@code <related-links>} that is not map
 *       hierarchy navigation -&gt; {@code references}. DITA-OT's maplink
 *       step writes both kinds into the same block: hierarchy links carry
 *       {@code role="parent"}/{@code "child"}/{@code "sibling"}/... and
 *       are skipped (the {@code contains} edges already say it), while
 *       relationship-table links carry {@code role="friend"} (confirmed
 *       against a live DITA-OT 4.4.1 run on dita-ot/docs) and are real,
 *       author-declared relations. They only become an edge when no
 *       inline link already connects the same pair, so a target is never
 *       listed under two headings of one concept.
 * </ul>
 * A {@code <topicref>} with {@code processing-role="resource-only"} (a
 * conref warehouse, a key-definition target) is not part of the
 * navigation: it gets no {@code contains} edge. DITA-OT cascades the
 * attribute onto every topicref it applies to before this class runs, so
 * the topicref's own attribute is enough. The topic itself is still a
 * node, since it is a real source of reused content.
 * {@code <mapref>}/{@code anchorref} submap composition needs no extra
 * code here at all: DITA-OT's own preprocessing flattens a
 * {@code <mapref>}-included submap's {@code <topicref>} tree directly
 * into the resolved base map before this class ever runs, so the
 * recursive walk above picks it up the same way it picks up ordinary
 * nesting -- confirmed against a live DITA-OT 4.4
 * ({@code sample-docs-mapref/README.md}, {@code docs/dev/
 * phase-0-findings.md} finding 14). {@code <navref>} is genuinely
 * unsupported -- DITA-OT never resolves it for this transtype at all,
 * so the referenced map/topics never reach this extractor's input in
 * the first place, and this class doesn't independently parse/merge
 * navigation maps either (that would mean bypassing DITA-OT's own
 * keyref/conref resolution and DITAVAL filtering for that content
 * alone). Unlike a plain gap, though, this one is surfaced: {@link
 * #walkMapChildren} detects a {@code <navref>} element directly (it
 * survives verbatim in the resolved map) and logs {@code
 * DITA2GRAPH060W} instead of silently walking past it, so a map author
 * relying on {@code <navref>} composition gets a visible warning that
 * that content is missing from the graph, not silence (finding 16).
 * {@code generated-from} *is* extracted here too (finding 15), just not
 * from markup directly: every resolved element carries an {@code xtrf}
 * attribute tracing its true source file, and DITA-OT replaces a
 * {@code conref}/{@code conkeyref}-resolved element wholesale with the
 * referenced one (inheriting its {@code xtrf}), while a plain {@code
 * keyref}-resolved {@code <keyword>}/{@code <ph>} (variable substitution,
 * not reuse) keeps its own -- so a descendant element whose {@code xtrf}
 * differs from its containing topic's own file is real, deterministic
 * proof of conref/conkeyref reuse, not a guess.
 * {@code applies-to} and {@code related-to} both require actual
 * heuristic inference and are computed downstream in the Rust core
 * instead (§3.3, {@code relations.rs}): {@code related-to} from {@code
 * product} metadata (finding 13), {@code applies-to} from {@code
 * uicontrols}/{@code cmdUicontrols} (this class extracts both -- the
 * former from anywhere in a topic's body, the latter scoped to {@code
 * <cmd>} elements only, matching "a task's {@code <cmd>} referencing a
 * {@code <uicontrol>} defined in a reference topic" precisely -- but the
 * actual cross-topic matching, including the ambiguous-match-drop rule,
 * happens in Rust, finding 15).
 *
 * <p>Each topic's body element ({@code conbody}/{@code taskbody}/{@code
 * refbody}/{@code glossdef}/generic {@code body}, per {@link
 * #bodyElementTag}) is also captured as whitespace-normalized plain text
 * -- markup stripped, nothing else cleaned up -- for both the OKF
 * bundle's body content and the RAG index (§4.4, §13.1). A *block-level*
 * subtree ({@code <p>}/{@code <step>}/{@code <note>}/{@code <li>}/etc.)
 * pulled in via {@code conref}/{@code conkeyref} is excluded from that
 * capture (canonical-node deduplication, {@code
 * docs/dev/canonical-node-dedup-spec.md}): it's already fully
 * represented in its source topic's own body, and this topic's {@code
 * generated-from} edge to that source is the pointer, so duplicating the
 * text here too would just inflate the bundle and RAG index with copies
 * of identical content -- exactly the failure mode a heavily-reused real
 * DITA corpus (shared warnings, boilerplate, common steps) would hit.
 * A foreign *inline* element ({@link #INLINE_ELEMENTS}) is never
 * excluded this way, even though it's still recorded for {@code
 * generated-from} -- dropping a reused word or two out of the middle of
 * an authored sentence would mangle the surrounding prose, not cleanly
 * remove a self-contained chunk of it the way a whole excluded {@code
 * <p>}/{@code <step>} does.
 *
 * <p>Each {@code generated-from} edge also names the reused fragments:
 * the {@code @id} of each conref'd element in its source topic. DITA-OT
 * drops that {@code @id} from the pulled-in copy but keeps its {@code
 * xtrc} (element name, occurrence and position in the source file), and
 * the source topic's own resolved copy carries the same {@code xtrc} next
 * to the {@code @id}, so the id is looked up, not guessed. A fragment with
 * no id to find falls back to {@code element@line} from its {@code xtrc}.
 * The same walk also builds {@code resolvedBody}: the body text with every
 * reused fragment kept, between {@code [reused from <file>#<id>]} and
 * {@code [end reused]} markers -- what a reader of the published topic
 * sees, for tools that must show a topic's full text.
 *
 * <p>A topic's graph node id normally comes straight from its own {@code
 * id} attribute -- but two different topics sharing the same {@code id}
 * (confirmed against a real, large third-party corpus: 33 separate
 * topics all carrying the literal, unfilled authoring-template
 * placeholder {@code id="ID"}, plus several genuine {@code id} reuses
 * across near-duplicate topics) would otherwise collapse onto the same
 * OKF concept file path, the second one written silently overwriting
 * the first -- real data loss a small hand-built fixture would never
 * surface. Detected and disambiguated (the later topic's own id plus a
 * source-path-derived suffix), logged as {@code DITA2GRAPH070W} naming
 * both files, never silently dropped.
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

    /** Prefix of a heading's key in {@link RawContainment}: no resolved path starts with it. */
    private static final String HEADING_KEY = "#heading-";
    /** Table-of-contents headings found while walking the map, by key, in document order. */
    private final Map<String, TopicNode> headings = new LinkedHashMap<>();
    private final Map<String, Integer> headingIdCounts = new HashMap<>();
    /** {@code src} URI -> temp path of every job.xml file, to place a heading in its own map. */
    private Map<String, String> srcToPath = Map.of();
    /** Temp paths of every "dita" job.xml file, to tell whether a {@code copy-to} copy was written. */
    private Set<String> jobDitaPaths = Set.of();
    /**
     * Temp paths of the {@code copy-to} copies the map places in its table
     * of contents. DITA-OT leaves the topicref's {@code href} on the source
     * topic and writes the copy as its own job.xml file, with the *same*
     * {@code src} as the source (confirmed in a kept temp directory of a
     * DITA-OT 4.4.1 run, a {@code copy-to} fixture): without knowing
     * which file is the copy, the TOC entry pointed at the source (two
     * parents, the copy none) and the shared {@code src} mapped to
     * whichever file came last, so one of the two looked like it reused
     * the other's whole content.
     */
    private final Set<String> copyPaths = new HashSet<>();
    /** Temp paths named by a resource-only topicref. */
    private final Set<String> resourceOnlyTargets = new HashSet<>();
    /** Temp paths named by a topicref of the table of contents (any depth). */
    private final Set<String> navigationTargets = new HashSet<>();

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
        /** DITA-OT's own {@code resource-only="true"}, when it writes one. */
        boolean resourceOnly;
        /** The {@code src} attribute (an absolute {@code file:} URI) -- matches resolved elements' own {@code xtrf} values verbatim (finding 15). */
        String src;
    }

    /** Raw, unresolved xref/link found while parsing a topic, resolved in a second pass. */
    private static final class RawLink {
        final String sourceTopicPath;
        final String href;
        final boolean hasKeyref;
        /** From a {@code <related-links>} block (a relationship table or an authored related link), not the prose. */
        final boolean related;

        RawLink(String sourceTopicPath, String href, boolean hasKeyref, boolean related) {
            this.sourceTopicPath = sourceTopicPath;
            this.href = href;
            this.hasKeyref = hasKeyref;
            this.related = related;
        }
    }

    /**
     * {@code role} values DITA-OT's maplink step gives the navigation links
     * it derives from the map hierarchy and collection types. Anything else
     * in {@code <related-links>} -- {@code role="friend"} from a
     * relationship table, or a link the author wrote -- is a real relation.
     */
    private static final Set<String> HIERARCHY_ROLES = Set.of(
            "parent", "child", "sibling", "next", "previous", "ancestor", "descendant", "cousin");

    /**
     * One "this topic pulled content in from that file" record, found by
     * scanning a topic's resolved body for descendant elements whose
     * {@code xtrf} attribute points at a *different* file than the
     * topic's own -- DITA-OT's own trace of where {@code conref}/{@code
     * conkeyref}-resolved content really came from (finding 15). Resolved
     * into a {@code generated-from} edge once every topic's id is known,
     * same two-pass shape as {@link RawLink}.
     */
    private static final class RawGeneratedFrom {
        final String sourceTopicPath;
        final String generatedFromPath;
        /** {@code xtrc} of each reused fragment's root element, in document order. */
        final Set<String> fragmentTraces = new LinkedHashSet<>();

        RawGeneratedFrom(String sourceTopicPath, String generatedFromPath) {
            this.sourceTopicPath = sourceTopicPath;
            this.generatedFromPath = generatedFromPath;
        }
    }

    /**
     * Per-topic state of the combined body-text/generated-from walk
     * ({@link #walkGeneratedFromAndBodyText}): the deduplicated body, the
     * resolved body with placeholder markers, and the reuse records found.
     */
    private static final class BodyWalk {
        final String ownPath;
        final Map<String, String> ditaSrcToPath;
        final Element bodyRoot;
        final StringBuilder ownText = new StringBuilder();
        final StringBuilder resolvedText = new StringBuilder();
        /** {fromPath, xtrc} per marker, indexed by the number in its placeholder. */
        final List<String[]> markers = new ArrayList<>();
        final Map<String, RawGeneratedFrom> bySource = new LinkedHashMap<>();

        BodyWalk(String ownPath, Map<String, String> ditaSrcToPath, Element bodyRoot) {
            this.ownPath = ownPath;
            this.ditaSrcToPath = ditaSrcToPath;
            this.bodyRoot = bodyRoot;
        }
    }

    /** Opens a reused fragment in {@link BodyWalk#resolvedText}; the number indexes {@link BodyWalk#markers}. */
    private static final char MARK_OPEN = '\u0001';
    /** Closes the reused fragment opened by the matching {@link #MARK_OPEN}. */
    private static final char MARK_CLOSE = '\u0002';

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
        mapNode.title = childText(mapRoot, "title", "");
        if (mapNode.title.isEmpty()) {
            // A bookmap's title is booktitle/mainbooktitle, not title.
            for (Element booktitle : directChildren(mapRoot, "booktitle")) {
                mapNode.title = childText(booktitle, "mainbooktitle", "").replaceAll("\\s+", " ");
            }
        }
        if (mapNode.title.isEmpty()) {
            mapNode.title = mapNode.id;
        }
        mapNode.sourceFile = mapFile.path;

        headings.clear();
        headingIdCounts.clear();
        copyPaths.clear();
        resourceOnlyTargets.clear();
        navigationTargets.clear();
        jobDitaPaths = new HashSet<>();
        for (JobFile file : files) {
            if ("dita".equals(file.format)) {
                jobDitaPaths.add(file.path);
            }
        }
        List<RawContainment> containments = new ArrayList<>();
        Map<String, List<String>> keysByPath = new HashMap<>();
        // Only headings read srcToPath, during the walk, to find their
        // map; maps are never copy-to copies, so the copies the walk is
        // about to find cannot change the entries they need.
        srcToPath = srcToPathPreferringSources(files, null);
        walkMapChildren(mapRoot, mapFile.path, null, 1, containments, keysByPath);

        // Every resolved element DITA-OT writes carries an xtrf attribute
        // tracing its true source file (as an absolute file: URI,
        // matching a JobFile's own src verbatim) -- used below to detect
        // conref/conkeyref-pulled content for generated-from (finding 15).
        // A copy-to copy shares its source's src, which must keep mapping
        // to the source: the copy's content then reads as derived from
        // the source (generated-from), never the other way round.
        Map<String, String> ditaSrcToPath = srcToPathPreferringSources(files, "dita");

        // Pass 2: parse every topic file for its own metadata, deferring
        // link resolution (target ids may not be known yet).
        Map<String, TopicNode> topicsByPath = new LinkedHashMap<>();
        List<RawLink> rawLinks = new ArrayList<>();
        List<RawGeneratedFrom> rawGeneratedFrom = new ArrayList<>();
        // path -> (xtrc -> @id) of each topic's own elements: resolves a
        // reused fragment's xtrc back to its id in the source topic, once
        // every topic has been parsed (pass 3).
        Map<String, Map<String, String>> fragmentIdsByPath = new HashMap<>();
        // Topics whose body holds reused fragments, kept until pass 3 can
        // name those fragments in resolvedBody.
        Map<String, BodyWalk> bodyWalksWithReuse = new LinkedHashMap<>();
        // Duplicate-topic-id guard (confirmed against a real, large
        // third-party corpus, dita-ot/docs: 33 separate topics sharing
        // the literal id "ID" -- an unfilled authoring-template
        // placeholder, common enough in real DITA content that a small
        // hand-built fixture would never surface it). Without this, two
        // topics sharing an id collapse into one OKF concept file path
        // (id.okf_type()'s bundle_dir + "/" + id + ".md") and the second
        // one written silently overwrites the first -- real data loss,
        // no error, no warning. `Map<id, path of the topic that
        // currently owns it>` so a collision's warning can name both
        // files, not just announce that *a* collision happened.
        Map<String, String> topicIdOwners = new HashMap<>();
        for (TopicNode heading : headings.values()) {
            topicIdOwners.put(heading.id, heading.sourceFile);
        }
        // Sources before their copy-to copies, whatever job.xml's order:
        // the source keeps its own id, the copy gets the suffixed one.
        List<JobFile> ditaFiles = new ArrayList<>();
        for (JobFile file : files) {
            if ("dita".equals(file.format)) {
                ditaFiles.add(file);
            }
        }
        ditaFiles.sort(Comparator.comparing(f -> copyPaths.contains(f.path)));
        for (JobFile file : ditaFiles) {
            Document topicDoc = parse(builder, new File(tempDir, file.path));
            Element root = topicDoc.getDocumentElement();

            TopicNode topic = new TopicNode();
            topic.id = attr(root, "id", stem(file.path));
            String existingOwner = topicIdOwners.putIfAbsent(topic.id, file.path);
            if (existingOwner != null) {
                String original = topic.id;
                String disambiguated = original + "--" + file.path.replace('/', '-');
                int suffix = 2;
                while (topicIdOwners.putIfAbsent(disambiguated, file.path) != null) {
                    disambiguated = original + "--" + file.path.replace('/', '-') + "-" + suffix;
                    suffix++;
                }
                topic.id = disambiguated;
                if (copyPaths.contains(file.path)) {
                    // Expected: a copy-to copy carries its source's id.
                    info.accept("DITA2GRAPH071I: " + file.path + " is a copy-to copy of a topic with id \""
                            + original + "\" (" + existingOwner + ") -- using \"" + disambiguated
                            + "\" for the copy's graph node");
                } else {
                    warn.accept("DITA2GRAPH070W: " + file.path + " has the same id (\"" + original
                            + "\") as " + existingOwner + " -- using \"" + disambiguated
                            + "\" for this topic's graph node instead of silently overwriting "
                            + existingOwner + "'s OKF concept file");
                }
            }
            topic.topicType = mapTopicType(root.getTagName());
            if ("topic".equals(topic.topicType) && !root.getTagName().equals("topic")) {
                // Plain ASCII: Ant's console logging mangles "§" into "?"
                // (confirmed against a live DITA-OT 4.4 run for the
                // DITA2GRAPH060W message below; the same encoding path
                // applies here too).
                warn.accept("DITA2GRAPH040W: " + file.path + " has unrecognized topic type <"
                        + root.getTagName() + ">; emitting as generic Topic");
            }
            String ditaClass = normalizeSpace(root.getAttribute("class"));
            topic.ditaClass = ditaClass.isEmpty() ? null : ditaClass;
            // Resource-only only when nothing in the table of contents
            // names the file too: one navigation topicref publishes it.
            boolean resourceOnly = file.resourceOnly || resourceOnlyTargets.contains(file.path);
            if (resourceOnly && !navigationTargets.contains(file.path)) {
                topic.processingRole = "resource-only";
            }
            topic.title = childText(root, "title", topic.id);
            String shortdesc = directChildText(root, "shortdesc");
            topic.shortdesc = shortdesc.isEmpty() ? null : shortdesc;
            topic.audience.addAll(splitAttr(root, "audience"));
            topic.product.addAll(splitAttr(root, "product"));
            topic.sourceFile = file.path;
            topic.status = root.getAttribute("status");
            List<String> keys = keysByPath.get(file.path);
            if (keys != null) {
                topic.keys.addAll(keys);
            }

            List<Element> uicontrolElements = new ArrayList<>();
            for (Element uicontrol : descendants(root, "uicontrol")) {
                if (!isInsideRelatedLinks(uicontrol)) {
                    uicontrolElements.add(uicontrol);
                }
            }
            topic.uicontrols.addAll(collectDistinctText(uicontrolElements));

            List<Element> cmdUicontrolElements = new ArrayList<>();
            for (Element cmd : descendants(root, "cmd")) {
                cmdUicontrolElements.addAll(descendants(cmd, "uicontrol"));
            }
            topic.cmdUicontrols.addAll(collectDistinctText(cmdUicontrolElements));

            fragmentIdsByPath.put(file.path, ownFragmentIds(root, file.path, ditaSrcToPath));

            // generated-from (finding 15) and body-text extraction
            // (canonical-node dedup, docs/dev/canonical-node-dedup-spec.md)
            // in one combined walk of the topic instead of two separate
            // full-topic scans -- both need the same xtrf-mismatch check
            // at every element, so they share a single traversal rather
            // than paying for it twice per topic.
            BodyWalk walk = extractBodyAndGeneratedFrom(root, topic.topicType, file.path, ditaSrcToPath);
            rawGeneratedFrom.addAll(walk.bySource.values());
            String body = normalizeSpace(walk.ownText);
            topic.body = body.isEmpty() ? null : body;
            if (!walk.markers.isEmpty()) {
                bodyWalksWithReuse.put(file.path, walk);
            }

            for (Element xref : descendants(root, "xref", "link")) {
                boolean related = isInsideRelatedLinks(xref);
                if (related && HIERARCHY_ROLES.contains(xref.getAttribute("role"))) {
                    // DITA-OT's own preprocessing auto-generates
                    // parent/child/sibling navigation <link> elements
                    // from the map hierarchy itself (confirmed directly:
                    // a genuinely nested topicref/topichead/topicgroup
                    // map produces these in every affected topic's
                    // resolved output). These are TOC navigation the
                    // contains edges already capture, not authored
                    // cross-references -- extracting them as
                    // "references" would mislabel generated navigation
                    // as author intent.
                    continue;
                }
                String href = xref.getAttribute("href");
                if (href.isEmpty() || isExternal(href, xref.getAttribute("scope"))) {
                    continue;
                }
                String fragmentless = href.contains("#") ? href.substring(0, href.indexOf('#')) : href;
                if (fragmentless.isEmpty()) {
                    continue; // same-topic fragment reference
                }
                boolean hasKeyref = !xref.getAttribute("keyref").isEmpty();
                rawLinks.add(new RawLink(file.path, resolve(file.path, fragmentless), hasKeyref, related));
            }

            topicsByPath.put(file.path, topic);
        }

        // Pass 3: resolve map topicrefs and topic-body links now that
        // every topic's real id is known.
        for (RawContainment containment : containments) {
            TopicNode target = containment.targetPath.startsWith(HEADING_KEY)
                    ? headings.get(containment.targetPath)
                    : topicsByPath.get(containment.targetPath);
            if (target == null) {
                warn.accept("DITA2GRAPH010W: unresolved topicref target " + containment.targetPath
                        + " in " + mapFile.path);
                continue;
            }
            if (containment.containerPath == null) {
                mapNode.links.add(new Link("contains", target.id));
                continue;
            }
            TopicNode container = containment.containerPath.startsWith(HEADING_KEY)
                    ? headings.get(containment.containerPath)
                    : topicsByPath.get(containment.containerPath);
            if (container == null) {
                // The containing topicref's own target never resolved;
                // already warned about that when its own RawContainment
                // record was processed above.
                continue;
            }
            container.links.add(new Link("contains", target.id));
        }
        // Deduplicated by (source, relation, target) -- confirmed against
        // a real, large third-party corpus (dita-ot/docs): prose that
        // mentions the same keyref-based cross-reference several times
        // (e.g. a glossary-style term referenced from multiple sentences
        // in one topic) produced one RawLink per mention, so the same
        // target ended up repeated under one topic's "# Requires"/
        // "# References" heading up to four times over -- real, visible
        // noise (`okf_validator`'s own "redundant link" warning fired
        // ~150 times across that corpus) that a small hand-built fixture
        // never surfaces, since none of them repeat the same
        // xref/keyref mention within one topic. One edge per distinct
        // (source, relation, target), same "one edge per distinct
        // source" discipline `generated-from` below already follows.
        Set<String> seenLinks = new HashSet<>();
        // source path + target id of every pair the prose already links:
        // a related link to the same target adds nothing but a second
        // heading listing it (okf_validator's "redundant link" warning).
        Set<String> linkedPairs = new HashSet<>();
        for (boolean relatedPass : new boolean[] {false, true}) {
            for (RawLink raw : rawLinks) {
                if (raw.related != relatedPass) {
                    continue;
                }
                TopicNode source = topicsByPath.get(raw.sourceTopicPath);
                TopicNode target = topicsByPath.get(raw.href);
                if (source == null || target == null) {
                    warn.accept("DITA2GRAPH010W: unresolved link target " + raw.href + " in " + raw.sourceTopicPath);
                    continue;
                }
                if (source == target) {
                    continue;
                }
                String pair = raw.sourceTopicPath + ' ' + target.id;
                if (raw.related) {
                    // spec §4.3: references = xref + related-links. A
                    // related link is a "see also", never a prerequisite,
                    // whatever keyref put it there.
                    if (!linkedPairs.add(pair)) {
                        continue;
                    }
                    source.links.add(new Link("references", target.id));
                    continue;
                }
                String relation = raw.hasKeyref ? "requires" : "references";
                if (!seenLinks.add(raw.sourceTopicPath + ' ' + relation + ' ' + target.id)) {
                    continue;
                }
                linkedPairs.add(pair);
                source.links.add(new Link(relation, target.id));
            }
        }
        for (RawGeneratedFrom raw : rawGeneratedFrom) {
            TopicNode source = topicsByPath.get(raw.sourceTopicPath);
            TopicNode target = topicsByPath.get(raw.generatedFromPath);
            // Both paths came from the same ditaSrcToPath/topicsByPath
            // built off the same files list, so an unresolved lookup
            // here would mean DITA-OT's own xtrf pointed somewhere
            // outside the job's own dita-format files -- not observed in
            // practice, so this is a defensive skip, not a warned gap.
            if (source == null || target == null || source == target) {
                continue;
            }
            List<String> fragments = new ArrayList<>();
            for (String trace : raw.fragmentTraces) {
                String fragment = fragmentId(raw.generatedFromPath, trace, fragmentIdsByPath);
                if (!fragments.contains(fragment)) {
                    fragments.add(fragment);
                }
            }
            source.links.add(new Link("generated-from", target.id, fragments));
        }
        for (Map.Entry<String, BodyWalk> entry : bodyWalksWithReuse.entrySet()) {
            String resolved = renderResolvedBody(entry.getValue(), fragmentIdsByPath);
            if (!resolved.isEmpty()) {
                topicsByPath.get(entry.getKey()).resolvedBody = resolved;
            }
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
        nodes.addAll(headings.values());
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

    /**
     * {@code src} URI -> temp path of job.xml's files ({@code format} only,
     * when given). A {@code copy-to} copy shares its source's {@code src};
     * the source wins, and the copy is only used when its source is not
     * in the job at all.
     */
    private Map<String, String> srcToPathPreferringSources(List<JobFile> files, String format) {
        Map<String, String> map = new HashMap<>();
        for (boolean copies : new boolean[] {true, false}) {
            for (JobFile file : files) {
                if (file.src.isEmpty() || (format != null && !format.equals(file.format))
                        || copyPaths.contains(file.path) != copies) {
                    continue;
                }
                map.put(file.src, file.path);
            }
        }
        return map;
    }

    private List<JobFile> parseJobXml(File jobXml) throws Exception {
        Document doc = parse(newDocumentBuilder(), jobXml);
        List<JobFile> files = new ArrayList<>();
        NodeList fileNodes = doc.getElementsByTagName("file");
        for (int i = 0; i < fileNodes.getLength(); i++) {
            Element el = (Element) fileNodes.item(i);
            JobFile f = new JobFile();
            // DITA-OT writes job.xml's path attribute with the platform
            // separator -- "topics\foo.dita" on Windows (confirmed against
            // a real DITA-OT 4.4.1 run there). Everything downstream
            // (resolve(), the topicsByPath/keysByPath lookups, duplicate-id
            // disambiguation) assumes '/', so without this every link
            // lookup missed on Windows, and a disambiguated id kept its
            // backslashes and broke the core's okf/topics/{id}.md write.
            f.path = el.getAttribute("path").replace('\\', '/');
            f.format = el.getAttribute("format");
            f.src = el.getAttribute("src");
            f.input = "true".equals(el.getAttribute("input"));
            f.resourceOnly = "true".equals(el.getAttribute("resource-only"));
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
     * every {@code topicref}, recognized by its {@code @class} ({@code
     * - map/topicref }) so that specializations count too: a bookmap's
     * {@code <chapter>}/{@code <part>}/{@code <appendix>}/{@code
     * <notices>}, a {@code <glossref>}, as well as {@code <topichead>}
     * and {@code <topicgroup>}. Matching by element name alone missed the
     * chapters DITA-OT leaves as they are in a resolved bookmap (one
     * pointing straight at a topic, one with only a navtitle) and every
     * glossref -- 2 of the 25 chapters of the Oxygen XML user guide and
     * its whole glossary (dita2graph-demo/bench, question o4). Without a
     * {@code @class} (hand-written fixtures), the element name decides.
     *
     * <p>One {@link RawContainment} per topicref with an {@code href},
     * descending into every one (arbitrarily deep, up to {@link
     * #maxDepth}). {@code containerPath} is the resolved path of the
     * nearest container so far ({@code null} means "the map itself"),
     * or the key of a heading. A topicref without {@code href} but with a
     * navtitle -- a {@code topichead}, a navtitle-only {@code chapter} --
     * is a heading of the table of contents: it becomes a node of type
     * {@code topichead} that contains its children, so the graph keeps
     * the outline a reader sees. A {@code topicgroup} (DITA-OT also wraps
     * every resolved submap in one) or an href-less topicref with no
     * navtitle has nothing to show: its children keep the *same* {@code
     * containerPath} and {@code level}, skipping through it.
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
            String kind = isNavref(child) ? "navref" : isTopicref(child) ? "topicref" : "";
            switch (kind) {
                case "topicref": {
                    String href = child.getAttribute("href");
                    if (href.isEmpty()) {
                        String heading = isTopicgroup(child)
                                || "resource-only".equals(child.getAttribute("processing-role"))
                                ? "" : navtitle(child);
                        if (heading.isEmpty()) {
                            walkMapChildren(child, mapPath, containerPath, level, containments, keysByPath);
                        } else if (level <= maxDepth) {
                            String key = addHeading(heading, child, mapPath);
                            containments.add(new RawContainment(containerPath, key));
                            walkMapChildren(child, mapPath, key, level + 1, containments, keysByPath);
                        }
                        break;
                    }
                    if (isExternal(href, child.getAttribute("scope"))) {
                        // A topicref pointing outside the corpus entirely
                        // -- most commonly a keyref-resolved <keydef
                        // href="https://..." scope="external"> (a
                        // "related talk"/"further reading" entry, DITA-OT
                        // resolves the keyref onto the topicref itself
                        // during preprocessing before this class ever
                        // sees it), confirmed directly against a real,
                        // large third-party corpus (dita-ot/docs'
                        // conference-talk maps) where this pattern is
                        // common and previously produced one
                        // DITA2GRAPH010W "unresolved topicref target" per
                        // external link -- real noise, not a real gap:
                        // there was never a local topic for it to be. No
                        // contains edge, no warning, same treatment
                        // <xref>/<link> already give an external target
                        // below. Still walked (not skipped) for any
                        // nested *local* topicrefs, same as
                        // topichead/topicgroup -- an external topicref
                        // isn't a real container any more than they are,
                        // so descendants keep the same containerPath/
                        // level rather than treating this href as a new
                        // container level.
                        walkMapChildren(child, mapPath, containerPath, level, containments, keysByPath);
                        break;
                    }
                    String format = child.getAttribute("format");
                    if (!format.isEmpty() && !"dita".equals(format)) {
                        // Not a topic (a ditavalref's .ditaval, a local
                        // PDF...): nothing to contain, same as external.
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
                    String copyTo = child.getAttribute("copy-to");
                    if (!copyTo.isEmpty() && jobDitaPaths.contains(resolve(mapPath, copyTo))) {
                        // The TOC entry is the copy DITA-OT wrote, not the
                        // source its href still names (see copyPaths).
                        targetPath = resolve(mapPath, copyTo);
                        copyPaths.add(targetPath);
                    }
                    String keys = child.getAttribute("keys");
                    if (!keys.isEmpty()) {
                        keysByPath.put(targetPath, List.of(keys.trim().split("\\s+")));
                    }
                    if ("resource-only".equals(child.getAttribute("processing-role"))) {
                        // Not navigation (a conref warehouse, a key
                        // target): no contains edge, and no new container
                        // level for its children, which DITA-OT has
                        // already marked resource-only too unless they
                        // explicitly opt back in.
                        resourceOnlyTargets.add(targetPath);
                        walkMapChildren(child, mapPath, containerPath, level, containments, keysByPath);
                        break;
                    }
                    navigationTargets.add(targetPath);
                    containments.add(new RawContainment(containerPath, targetPath));
                    walkMapChildren(child, mapPath, targetPath, level + 1, containments, keysByPath);
                    break;
                }
                case "navref": {
                    // Unlike <mapref>/anchorref (finding 14), DITA-OT
                    // never resolves <navref> for this transtype at all
                    // -- confirmed directly: the referenced map/topics
                    // never enter job.xml, so it survives verbatim here.
                    // This plugin doesn't independently parse/merge
                    // navigation maps either (that would mean bypassing
                    // DITA-OT's own preprocessing -- keyref/conref
                    // resolution, DITAVAL filtering -- for that content
                    // alone, a materially different and riskier
                    // undertaking than mapref support). Surfacing this
                    // as a warning, not silently dropping it, is the
                    // actual scope closed here (finding 16).
                    String mapref = child.getAttribute("mapref");
                    String href = child.getAttribute("href");
                    String target = !mapref.isEmpty() ? mapref : href;
                    // Plain ASCII only: Ant's console logging mangled a
                    // "§3.3" citation tried here into "?3.3" (confirmed
                    // against a live DITA-OT 4.4 run), and none of this
                    // class's other warn/info messages cite spec
                    // sections either -- kept consistent.
                    warn.accept("DITA2GRAPH060W: <navref"
                            + (target.isEmpty() ? "" : " mapref=\"" + target + "\"")
                            + "> in " + mapPath + " is not resolved by DITA-OT for this transtype "
                            + "and is not independently parsed by this plugin; its content is "
                            + "excluded from the graph");
                    break;
                }
                default:
                    break;
            }
        }
    }

    /**
     * Whether {@code href}/{@code scope} point outside this corpus
     * entirely -- a bare {@code http://}/{@code https://} URL, or an
     * explicit {@code scope="external"}/{@code scope="peer"} (the DITA
     * way of saying "don't resolve this within the current map/topic
     * set" even for an otherwise plausible-looking relative {@code
     * href}). Shared between {@code <xref>}/{@code <link>} handling and
     * {@code <topicref>} containment: both need the same "is this
     * pointing at real content in this corpus, or somewhere else
     * entirely" check, and a {@code <topicref>} carrying it is just as
     * common as an {@code <xref>} carrying it -- most often a {@code
     * keyref}-resolved {@code <keydef href="https://..."
     * scope="external">} (a "related talk"/"further reading" entry),
     * which DITA-OT resolves onto the {@code topicref} itself during
     * preprocessing before this class ever sees it (confirmed directly
     * against a real, large third-party corpus where this pattern is
     * common).
     */
    private static boolean isExternal(String href, String scope) {
        return href.startsWith("http://") || href.startsWith("https://")
                || "external".equals(scope) || "peer".equals(scope);
    }

    /** A topicref or one of its specializations (chapter, glossref, topichead...), by {@code @class}. */
    private static boolean isTopicref(Element el) {
        String cls = el.getAttribute("class");
        if (!cls.isEmpty()) {
            return cls.contains(" map/topicref ");
        }
        return Set.of("topicref", "topichead", "topicgroup", "chapter", "part", "appendix", "glossref", "notices")
                .contains(el.getNodeName());
    }

    /** A topicgroup (including DITA-OT's resolved-submap wrapper): never a heading. */
    private static boolean isTopicgroup(Element el) {
        String cls = el.getAttribute("class");
        return cls.isEmpty() ? "topicgroup".equals(el.getNodeName()) : cls.contains(" mapgroup-d/topicgroup ");
    }

    private static boolean isNavref(Element el) {
        String cls = el.getAttribute("class");
        return cls.isEmpty() ? "navref".equals(el.getNodeName()) : cls.contains(" map/navref ");
    }

    /** {@code @navtitle}, else {@code topicmeta/navtitle}, whitespace-normalized; empty when neither. */
    private static String navtitle(Element el) {
        String attr = el.getAttribute("navtitle").trim();
        if (!attr.isEmpty()) {
            return attr.replaceAll("\\s+", " ");
        }
        for (Element topicmeta : directChildren(el, "topicmeta")) {
            for (Element navtitle : directChildren(topicmeta, "navtitle")) {
                return navtitle.getTextContent().trim().replaceAll("\\s+", " ");
            }
        }
        return "";
    }

    /**
     * Registers a table-of-contents heading as a node of type {@code
     * topichead} (id {@code topichead--<slug>}, made unique with a
     * suffix) and returns the key its {@link RawContainment} records use.
     * Its source file is the map that declares it (from its {@code xtrf}
     * when DITA-OT recorded one) plus {@code #<id>}: the OKF bundle's
     * {@code resource} is a concept's identity, and the map node and
     * every other heading of that map already point at the bare file.
     */
    private String addHeading(String title, Element el, String mapPath) {
        String slug = title.toLowerCase(java.util.Locale.ROOT).replaceAll("[^a-z0-9]+", "-").replaceAll("^-|-$", "");
        String base = "topichead--" + (slug.isEmpty() ? "heading" : slug);
        int count = headingIdCounts.merge(base, 1, Integer::sum);
        TopicNode heading = new TopicNode();
        heading.id = count == 1 ? base : base + "-" + count;
        heading.topicType = "topichead";
        heading.title = title;
        heading.sourceFile = srcToPath.getOrDefault(el.getAttribute("xtrf"), mapPath) + "#" + heading.id;
        String key = HEADING_KEY + headings.size();
        headings.put(key, heading);
        return key;
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
     * Each element's whitespace-normalized text content, deduplicated
     * (first-seen order preserved) and with empty results dropped -- used
     * for {@code <uicontrol>} text collection (§3.3's {@code applies-to}
     * inference input).
     */
    private static List<String> collectDistinctText(List<Element> elements) {
        Set<String> distinct = new LinkedHashSet<>();
        for (Element el : elements) {
            String text = el.getTextContent();
            if (text == null) {
                continue;
            }
            String trimmed = text.trim().replaceAll("\\s+", " ");
            if (!trimmed.isEmpty()) {
                distinct.add(trimmed);
            }
        }
        return new ArrayList<>(distinct);
    }

    /**
     * DITA phrase/inline elements (the "inline" content model, as opposed
     * to block containers like {@code <p>}/{@code <step>}/{@code <note>}/
     * {@code <li>}) -- canonical-node dedup (below) never excludes one of
     * these from body text even when its {@code xtrf} is foreign, per
     * {@code docs/dev/canonical-node-dedup-spec.md}'s v1 policy: excluding
     * a reused word or two out of the *middle* of an authored sentence
     * (e.g. {@code Click the <ph conref="...">Save</ph> button.}) would
     * mangle the surrounding prose, not just drop a self-contained chunk
     * of it the way excluding a whole {@code <p>}/{@code <step>} does.
     * Deliberately conservative (a known, common subset of the DITA base
     * phrase domain, not exhaustive) -- an unrecognized element defaults
     * to block behavior (excludable), the safer direction: worst case a
     * rare inline element still gets dropped like today, not a garbled
     * sentence from treating an actual block wrapper as inline.
     */
    private static final Set<String> INLINE_ELEMENTS = Set.of(
            "ph", "b", "i", "u", "tt", "sup", "sub", "cite", "q", "term",
            "keyword", "wintitle", "menucascade", "shortcut", "uicontrol",
            "filepath", "userinput", "systemoutput", "msgnum", "msgph",
            "varname", "apiname", "parmname", "option", "cmdname", "envvar",
            "codeph", "synph", "tm", "indexterm", "indextermref", "xref",
            "image", "data", "text", "numcharref", "textentity");

    /**
     * One combined walk of the whole topic doing two things that both
     * need the same per-element {@code xtrf}-mismatch check, instead of
     * two separate full-topic scans: (1) {@code generated-from} detection
     * (finding 15) -- any element whose {@code xtrf} points at a
     * different source file than {@code ownPath} was pulled in via
     * {@code conref}/{@code conkeyref}, recorded once per distinct
     * source, anywhere in the topic (title/shortdesc/prolog included, not
     * just the body); (2) the body element's own text, markup stripped
     * and whitespace collapsed to single spaces -- the "cleaned text"
     * input for both the OKF bundle body and the RAG index
     * (docs/plugin-specification.md §13.1, §4.4). {@code shortdesc} is a
     * separate sibling element in DITA and is not included here (see
     * {@link #directChildText} for that).
     *
     * <p>(2)'s "own" excludes any *block-level* subtree pulled in via
     * {@code conref}/{@code conkeyref} -- canonical-node deduplication
     * ({@code docs/dev/canonical-node-dedup-spec.md}): that text is
     * already fully represented in its source topic's own body, reachable
     * from this topic via the {@code generated-from} edge ({@link
     * RawGeneratedFrom}) this same walk just recorded, so duplicating it
     * here too would inflate both the OKF bundle and RAG chunk text with
     * copies of identical content -- the exact real-dataset problem
     * heavily-reused DITA content (shared warnings, boilerplate, common
     * steps) creates. A foreign {@link #INLINE_ELEMENTS} element is
     * never excluded from (2), even though it's still recorded for (1) --
     * see that field's own docs.
     *
     * <p>(3) is the resolved body: every text node of the body, reused or
     * not, with placeholder markers around each outermost reused fragment
     * that {@link #renderResolvedBody} turns into readable ones in pass 3.
     */
    private static BodyWalk extractBodyAndGeneratedFrom(Element root, String topicType, String ownPath,
            Map<String, String> ditaSrcToPath) {
        Element bodyRoot = null;
        for (Element child : directChildren(root, bodyElementTag(topicType))) {
            bodyRoot = child;
            break;
        }
        BodyWalk walk = new BodyWalk(ownPath, ditaSrcToPath, bodyRoot);
        walkGeneratedFromAndBodyText(walk, root, false, false, null);
        return walk;
    }

    /**
     * {@code xtrc} -&gt; {@code @id} for every element of this topic's own
     * content (its {@code xtrf} is the topic's own file) that has an id:
     * the conref targets other topics may pull in.
     */
    private static Map<String, String> ownFragmentIds(Element root, String ownPath, Map<String, String> ditaSrcToPath) {
        Map<String, String> ids = new HashMap<>();
        List<Element> elements = new ArrayList<>();
        elements.add(root);
        NodeList all = root.getElementsByTagName("*");
        for (int i = 0; i < all.getLength(); i++) {
            elements.add((Element) all.item(i));
        }
        for (Element el : elements) {
            String id = el.getAttribute("id");
            String xtrc = el.getAttribute("xtrc");
            if (!id.isEmpty() && !xtrc.isEmpty() && ownPath.equals(ditaSrcToPath.get(el.getAttribute("xtrf")))) {
                ids.putIfAbsent(xtrc, id);
            }
        }
        return ids;
    }

    /**
     * The id of the reused element traced by {@code xtrc} in the topic at
     * {@code sourcePath}, or {@code element@line} from the trace itself
     * ({@code "li:3;37:32"} -&gt; {@code "li@37"}) when that topic has no
     * id for it.
     */
    private static String fragmentId(String sourcePath, String xtrc, Map<String, Map<String, String>> fragmentIdsByPath) {
        Map<String, String> ids = fragmentIdsByPath.get(sourcePath);
        String id = ids == null ? null : ids.get(xtrc);
        if (id != null) {
            return id;
        }
        int colon = xtrc.indexOf(':');
        int semicolon = xtrc.indexOf(';');
        if (colon > 0 && semicolon > colon) {
            int lineEnd = xtrc.indexOf(':', semicolon);
            String line = xtrc.substring(semicolon + 1, lineEnd > semicolon ? lineEnd : xtrc.length());
            return xtrc.substring(0, colon) + "@" + line;
        }
        return xtrc;
    }

    /**
     * {@link BodyWalk#resolvedText} with its placeholders replaced by
     * {@code [reused from <file>#<fragment>]} ... {@code [end reused]},
     * whitespace collapsed like the plain body.
     */
    private static String renderResolvedBody(BodyWalk walk, Map<String, Map<String, String>> fragmentIdsByPath) {
        String raw = walk.resolvedText.toString();
        StringBuilder out = new StringBuilder(raw.length() + 64 * walk.markers.size());
        int i = 0;
        while (i < raw.length()) {
            char c = raw.charAt(i);
            if (c == MARK_OPEN) {
                int end = raw.indexOf(MARK_OPEN, i + 1);
                String[] marker = walk.markers.get(Integer.parseInt(raw.substring(i + 1, end)));
                String label = marker[1].isEmpty()
                        ? marker[0]
                        : marker[0] + "#" + fragmentId(marker[0], marker[1], fragmentIdsByPath);
                out.append(" [reused from ").append(label).append("] ");
                i = end + 1;
            } else if (c == MARK_CLOSE) {
                out.append(" [end reused] ");
                i++;
            } else {
                out.append(c);
                i++;
            }
        }
        return normalizeSpace(out);
    }

    private static String normalizeSpace(CharSequence text) {
        return text.toString().trim().replaceAll("\\s+", " ");
    }

    /**
     * @param insideBody         whether {@code el} is {@code bodyRoot} or
     *                           one of its descendants -- text only
     *                           accumulates within the body, but
     *                           generated-from detection runs everywhere.
     * @param textExcludedByAncestor
     *                           whether an ancestor of {@code el} was
     *                           itself a foreign, non-inline (block)
     *                           element -- once true, stays true for
     *                           every descendant regardless of that
     *                           descendant's own {@code xtrf}: DITA-OT
     *                           replaces a {@code conref}'d/{@code
     *                           conkeyref}'d element wholesale with the
     *                           referenced one (inheriting its whole
     *                           subtree's {@code xtrf}, finding 15), so
     *                           there's no "partially own, partially
     *                           foreign" subtree to worry about
     *                           splitting -- but the walk still descends
     *                           into it regardless, for generated-from
     *                           coverage, matching the previous
     *                           full-topic scan's exhaustive semantics
     *                           (a second, distinct foreign source
     *                           nested inside an already-excluded
     *                           subtree, however unlikely, must still be
     *                           detected).
     * @param foreignAncestorSource
     *                           the source path of the nearest reused
     *                           ancestor, or {@code null}: an element
     *                           reused from a different source than that
     *                           (or with no reused ancestor at all) is the
     *                           root of a fragment, whose {@code xtrc} names
     *                           it on the {@code generated-from} edge.
     */
    private static void walkGeneratedFromAndBodyText(BodyWalk walk, Element el, boolean insideBody,
            boolean textExcludedByAncestor, String foreignAncestorSource) {
        if (isInsideRelatedLinks(el) || "related-links".equals(el.getNodeName())) {
            return;
        }
        boolean nowInsideBody = insideBody || el == walk.bodyRoot;

        String fromPath = null;
        String xtrf = el.getAttribute("xtrf");
        if (!xtrf.isEmpty()) {
            String path = walk.ditaSrcToPath.get(xtrf);
            if (path != null && !path.equals(walk.ownPath)) {
                fromPath = path;
            }
        }
        boolean foreign = fromPath != null;
        boolean fragmentRoot = foreign && !fromPath.equals(foreignAncestorSource);
        if (fragmentRoot) {
            RawGeneratedFrom raw = walk.bySource.computeIfAbsent(fromPath, p -> new RawGeneratedFrom(walk.ownPath, p));
            String xtrc = el.getAttribute("xtrc");
            if (!xtrc.isEmpty()) {
                raw.fragmentTraces.add(xtrc);
            }
        }
        // Markers only around the outermost fragment: one nested in it
        // (a conref inside conref'd content) is already within them.
        boolean marked = nowInsideBody && fragmentRoot && foreignAncestorSource == null;
        if (marked) {
            walk.resolvedText.append(' ').append(MARK_OPEN).append(walk.markers.size()).append(MARK_OPEN).append(' ');
            walk.markers.add(new String[] {fromPath, el.getAttribute("xtrc")});
        }

        boolean excludeThisElementText = foreign && !INLINE_ELEMENTS.contains(el.getNodeName());
        boolean textExcluded = textExcludedByAncestor || excludeThisElementText;
        String childForeignSource = foreign ? fromPath : foreignAncestorSource;

        NodeList children = el.getChildNodes();
        for (int i = 0; i < children.getLength(); i++) {
            Node n = children.item(i);
            if (n.getNodeType() == Node.ELEMENT_NODE) {
                walkGeneratedFromAndBodyText(walk, (Element) n, nowInsideBody, textExcluded, childForeignSource);
            } else if (nowInsideBody
                    && (n.getNodeType() == Node.TEXT_NODE || n.getNodeType() == Node.CDATA_SECTION_NODE)) {
                String text = n.getTextContent();
                walk.resolvedText.append(text);
                if (!textExcluded) {
                    walk.ownText.append(text);
                }
            }
        }
        if (marked) {
            walk.resolvedText.append(' ').append(MARK_CLOSE).append(' ');
        }
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
