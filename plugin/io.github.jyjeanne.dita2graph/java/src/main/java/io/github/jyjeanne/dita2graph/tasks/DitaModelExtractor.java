package io.github.jyjeanne.dita2graph.tasks;

import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;

import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;
import java.io.File;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.Deque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
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
 *   <li>a local {@code <xref>}/{@code <link>} inside a task's {@code
 *       <prereq>}, or a task's {@code <link importance="required">} -&gt;
 *       {@code requires}: the only DITA markup that says "do or read this
 *       first" (docs/semantic-model.md). A link that merely goes through
 *       a key is a {@code references}, its key recorded in {@code keyrefs};
 *   <li>any other local {@code <xref>}/{@code <link>} -&gt; {@code references};
 *   <li>an {@code http(s)} {@code <xref>}/{@code <link>} -&gt; an entry of
 *       {@code externalLinks}, with the key it went through;
 *   <li>a {@code copy-to} copy -&gt; {@code copy-of} its source topic.
 * </ul>
 * The map's key definitions ({@code <keydef>} and every topicref with
 * {@code @keys}) survive DITA-OT's preprocessing in the resolved map, each
 * with the {@code xtrf} of the submap that declares it: they become the
 * map's {@code keyDefs}, the first definition of a key in its scope
 * winning, as in DITA. {@code @keyref} survives on the resolved elements
 * that used a key. Two things do not survive and are read from the
 * topic's source file instead (its job.xml {@code src}): {@code
 * <coderef>}, which DITA-OT replaces with the file's text (job.xml lists
 * the file, format {@code coderef}, but not who includes it), and {@code
 * @conkeyref}, turned into a plain conref then resolved away. Each is
 * kept only when the resolved topic confirms it survived DITAVAL
 * filtering: the coderef's parent element is still there (same {@code
 * xtrc}), the conkeyref's key names a topic this topic reuses content
 * from ({@code generated-from}).
 * <ul>
 *   <li>a {@code <link>} in {@code <related-links>} that is not map
 *       hierarchy navigation -&gt; {@code references}. DITA-OT's maplink
 *       step writes both kinds into the same block: hierarchy links carry
 *       {@code role="parent"}/{@code "child"}/{@code "sibling"}/... and
 *       are skipped (the {@code contains} edges already say it), while
 *       relationship-table links carry {@code role="friend"} (confirmed
 *       against a live DITA-OT 4.4.1 run on dita-ot/docs) and are real,
 *       author-declared relations. They only become an edge when no
 *       inline link already connects the same pair, so a target is never
 *       listed under two headings of one concept. A relationship-table
 *       link loses the {@code @keyref} of the map topicref it comes from,
 *       but keeps that topicref's {@code xtrf}/{@code xtrc}: the key is
 *       found back through them and counts as used by the topic showing
 *       the link. The link text and description DITA-OT copies into a
 *       generated link (the target's title and shortdesc) are the
 *       target's: their links and keys are not this topic's.
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
 * bundle's body content and the RAG index (§4.4, §13.1). In a composite
 * file, the title, summary and body of every nested topic join that text:
 * {@code dita-ot/docs}' release notes and Ant parameter details are a root
 * {@code <reference>} with no body of its own, all their content in nested
 * topics, and had no text at all before. A *block-level*
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
    /**
     * The directory job.xml's paths are relative to, as a {@code file:}
     * URI: the input map's {@code src} minus its {@code path}. Null when
     * the input map has no {@code src}.
     */
    private URI sourceBase;
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
        /** In a task's {@code <prereq>}, or a task's link with {@code importance="required"}. */
        final boolean required;
        /** From a {@code <related-links>} block (a relationship table or an authored related link), not the prose. */
        final boolean related;

        RawLink(String sourceTopicPath, String href, boolean required, boolean related) {
            this.sourceTopicPath = sourceTopicPath;
            this.href = href;
            this.required = required;
            this.related = related;
        }
    }

    /** A key definition found while walking the map, its topic target resolved in pass 3. */
    private static final class RawKeyDef {
        final KeyDef def = new KeyDef();
        /** Temp path of the topic the key names, or null. */
        String targetPath;
    }

    /** Effective key definitions by {@code scope + " " + name}, in map order. */
    private final Map<String, RawKeyDef> keyDefs = new LinkedHashMap<>();
    /** Temp path -> the key scopes ({@code ""}: root) of the topicrefs naming the file. */
    private final Map<String, Set<String>> scopesByPath = new HashMap<>();

    /** A key a map topicref uses ({@code @keyref}), with the scope it resolves in. */
    private record MapKeyref(String name, String scope) {
    }

    /**
     * The map topicrefs that use a key, by {@code xtrf + " " + xtrc}. A
     * relationship-table link DITA-OT's maplink step generates from such a
     * topicref keeps the topicref's {@code xtrf}/{@code xtrc} but not its
     * {@code @keyref} (confirmed against dita-ot/docs' resolved
     * topics/ant.dita: the reltable's {@code keyref="ant-manual"} becomes a
     * plain {@code href="http://ant.apache.org/manual"}): the traces lead
     * back to the key.
     */
    private final Map<String, MapKeyref> mapKeyrefsByTrace = new HashMap<>();

    /** A link of a topic made through a map topicref's key; {@code url} is null for a link inside the corpus. */
    private record MapKeyUse(String url, MapKeyref key) {
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
        /** The elements whose text is the topic's body (a glossentry has two). */
        final Set<Element> bodyRoots;
        /** The file's root topic: any other topic element under it is a nested topic. */
        final Element topicRoot;
        final StringBuilder ownText = new StringBuilder();
        final StringBuilder resolvedText = new StringBuilder();
        /** {fromPath, xtrc} per marker, indexed by the number in its placeholder. */
        final List<String[]> markers = new ArrayList<>();
        final Map<String, RawGeneratedFrom> bySource = new LinkedHashMap<>();

        BodyWalk(String ownPath, Map<String, String> ditaSrcToPath, Set<Element> bodyRoots, Element topicRoot) {
            this.ownPath = ownPath;
            this.ditaSrcToPath = ditaSrcToPath;
            this.bodyRoots = bodyRoots;
            this.topicRoot = topicRoot;
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
        mapNode.metadata = Metadata.ofMap(mapRoot);
        sourceBase = sourceBase(mapFile);

        headings.clear();
        headingIdCounts.clear();
        copyPaths.clear();
        resourceOnlyTargets.clear();
        navigationTargets.clear();
        keyDefs.clear();
        scopesByPath.clear();
        mapKeyrefsByTrace.clear();
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
        collectKeyDefsAndScopes(mapRoot, mapFile.path);
        walkMapChildren(mapRoot, mapFile.path, null, 1, containments, keysByPath);
        // A branch-filtering copy is a copy too: a <ditavalref> with a
        // <dvrResourcePrefix> or <dvrResourceSuffix> makes DITA-OT write the
        // topic under another name, with the src of the file it filters.
        // Without this, each such copy read as a second topic with the same
        // id (DITA2GRAPH070W) instead of an expected copy.
        for (JobFile file : files) {
            if ("dita".equals(file.format) && !authoredPath(file).equals(file.path)) {
                copyPaths.add(file.path);
            }
        }

        // Every resolved element DITA-OT writes carries an xtrf attribute
        // tracing its true source file (as an absolute file: URI,
        // matching a JobFile's own src verbatim) -- used below to detect
        // conref/conkeyref-pulled content for generated-from (finding 15).
        // A copy-to copy shares its source's src, which must keep mapping
        // to the source: the copy's content then reads as derived from
        // the source (generated-from), never the other way round.
        Map<String, String> ditaSrcToPath = srcToPathPreferringSources(files, "dita");
        // The sample files coderef includes (job.xml format "coderef"), by
        // src URI: who includes them is read from the topics' sources.
        Map<String, String> coderefSrcToPath = new HashMap<>();
        for (JobFile file : files) {
            if ("coderef".equals(file.format) && !file.src.isEmpty()) {
                coderefSrcToPath.put(file.src, file.path);
            }
        }
        // copy-to copy path -> its source's path, resolved to ids in pass 3.
        Map<String, String> copySources = new LinkedHashMap<>();
        // path -> key names the topic's source uses with conkeyref, kept
        // in pass 3 when the topic really reuses the key's target.
        Map<String, Set<String>> conkeyrefsByPath = new HashMap<>();
        // path -> the topic's relationship-table links made through a key,
        // the key resolved in pass 3 in the scope of the map topicref.
        Map<String, List<MapKeyUse>> mapKeyUsesByPath = new HashMap<>();

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
        // The map's node shares the id space: a topic named like the map
        // would otherwise collapse onto the map's node in graph.json.
        topicIdOwners.put(mapNode.id, mapFile.path);
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
                    // Expected: a copy (copy-to, branch filtering) carries
                    // its source's id.
                    info.accept("DITA2GRAPH071I: " + file.path + " is a copy of a topic with id \""
                            + original + "\" (" + existingOwner + ") -- using \"" + disambiguated
                            + "\" for the copy's graph node");
                } else {
                    warn.accept("DITA2GRAPH070W: " + file.path + " has the same id (\"" + original
                            + "\") as " + existingOwner + " -- using \"" + disambiguated
                            + "\" for this topic's graph node instead of silently overwriting "
                            + existingOwner + "'s OKF concept file");
                }
            }
            topic.topicType = topicTypeOf(root);
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
            topic.metadata = Metadata.ofTopic(root);
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
            // The copy keeps its own path, the concept's identity (OKF
            // rejects two concepts with one resource); the file it copies
            // travels beside it.
            topic.sourceFile = file.path;
            String authored = authoredPath(file);
            topic.authoredFile = authored.equals(file.path) ? null : authored;
            topic.status = root.getAttribute("status");
            List<String> keys = keysByPath.get(file.path);
            if (keys != null) {
                topic.keys.addAll(keys);
            }
            collectKeyrefs(root, topic.keyrefs);
            if (copyPaths.contains(file.path)) {
                // The source is the job's file of that src that is not a
                // copy: when the map only publishes filtered copies, there
                // is none, and the copies are not copies of each other.
                String sourcePath = ditaSrcToPath.get(file.src);
                if (sourcePath != null && !sourcePath.equals(file.path) && !copyPaths.contains(sourcePath)) {
                    copySources.put(file.path, sourcePath);
                }
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
            // The source can only add something when the job includes
            // sample files and the topic has a preformatted element their
            // text went into (coderef), or reuses content (conkeyref): the
            // other topics' sources are not read at all.
            boolean mayIncludeCode = !coderefSrcToPath.isEmpty() && hasPreformatted(root);
            if (mayIncludeCode || !walk.bySource.isEmpty()) {
                scanSource(builder, file, root, coderefSrcToPath, topic.codeRefs,
                        conkeyrefsByPath.computeIfAbsent(file.path, p -> new LinkedHashSet<>()));
            }
            String body = normalizeSpace(walk.ownText);
            topic.body = body.isEmpty() ? null : body;
            if (!walk.markers.isEmpty()) {
                bodyWalksWithReuse.put(file.path, walk);
            }

            for (Element xref : descendants(root, "xref", "link")) {
                if (isInsideLink(xref)) {
                    // In the description DITA-OT copies into a generated
                    // link from the target's shortdesc: the target's
                    // links, not this topic's.
                    continue;
                }
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
                if (href.isEmpty()) {
                    continue;
                }
                // A relationship-table link through a key: the key is
                // the map topicref's, found back by its traces.
                MapKeyref viaMap = related && xref.getAttribute("keyref").isEmpty()
                        ? mapKeyrefsByTrace.get(xref.getAttribute("xtrf") + " " + xref.getAttribute("xtrc"))
                        : null;
                if (isExternal(href, xref.getAttribute("scope"))) {
                    // Only absolute web URLs: a relative "external" or
                    // "peer" href names nothing a reader can follow.
                    if (href.startsWith("http://") || href.startsWith("https://")) {
                        String key = keyName(xref.getAttribute("keyref"));
                        topic.externalLinks.putIfAbsent(href, key.isEmpty() ? null : key);
                        if (viaMap != null) {
                            mapKeyUsesByPath.computeIfAbsent(file.path, p -> new ArrayList<>())
                                    .add(new MapKeyUse(href, viaMap));
                        }
                    }
                    continue;
                }
                if (viaMap != null) {
                    mapKeyUsesByPath.computeIfAbsent(file.path, p -> new ArrayList<>())
                            .add(new MapKeyUse(null, viaMap));
                }
                String fragmentless = href.contains("#") ? href.substring(0, href.indexOf('#')) : href;
                if (fragmentless.isEmpty()) {
                    continue; // same-topic fragment reference
                }
                boolean required = "task".equals(topic.topicType)
                        && (hasAncestor(xref, " task/prereq ", "prereq")
                                || "required".equals(xref.getAttribute("importance")));
                rawLinks.add(new RawLink(file.path, resolve(file.path, fragmentless), required, related));
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
        // One edge per (source, target) pair: requires when any link of the
        // pair says "required" (requires is a narrower references), else
        // references -- a target listed under two headings of one concept
        // is okf_validator's "redundant link" warning. Prose links come
        // first, then related links, so the edges keep the prose order.
        Set<String> requiredPairs = new HashSet<>();
        for (RawLink raw : rawLinks) {
            if (raw.required) {
                requiredPairs.add(raw.sourceTopicPath + ' ' + raw.href);
            }
        }
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
                if (source == target || !linkedPairs.add(raw.sourceTopicPath + ' ' + target.id)) {
                    continue;
                }
                boolean required = requiredPairs.contains(raw.sourceTopicPath + ' ' + raw.href);
                source.links.add(new Link(required ? "requires" : "references", target.id));
            }
        }
        for (Map.Entry<String, String> copy : copySources.entrySet()) {
            TopicNode copyNode = topicsByPath.get(copy.getKey());
            TopicNode source = topicsByPath.get(copy.getValue());
            if (copyNode != null && source != null) {
                copyNode.links.add(new Link("copy-of", source.id));
            }
        }
        // Authored file -> the reused elements of it that have no @id.
        Map<String, Set<String>> unnamedFragments = new TreeMap<>();
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
                Map<String, String> ids = fragmentIdsByPath.get(raw.generatedFromPath);
                if (ids == null || !ids.containsKey(trace)) {
                    String file = target.authoredFile != null ? target.authoredFile : target.sourceFile;
                    unnamedFragments.computeIfAbsent(file, k -> new TreeSet<>()).add(fragment);
                }
            }
            source.links.add(new Link("generated-from", target.id, fragments));
        }
        // A reused element without an @id is named element@line: its
        // fragment (and its IRI in the RDF export) changes as soon as a line
        // is added above it. One note per file, for its author.
        for (Map.Entry<String, Set<String>> entry : unnamedFragments.entrySet()) {
            Set<String> names = entry.getValue();
            info.accept("DITA2GRAPH073I: " + entry.getKey() + ": " + names.size()
                    + " reused element(s) without an @id (" + String.join(", ", names)
                    + "), named by their line: give them an @id so their fragment names stay stable");
        }
        for (Map.Entry<String, BodyWalk> entry : bodyWalksWithReuse.entrySet()) {
            String resolved = renderResolvedBody(entry.getValue(), fragmentIdsByPath);
            if (!resolved.isEmpty()) {
                topicsByPath.get(entry.getKey()).resolvedBody = resolved;
            }
        }
        for (RawKeyDef raw : keyDefs.values()) {
            TopicNode target = raw.targetPath == null ? null : topicsByPath.get(raw.targetPath);
            if (target != null) {
                raw.def.target = target.id;
            }
            mapNode.keyDefs.add(raw.def);
        }
        // A keyref names a key as the topic sees it, in the key scopes of
        // the topicrefs that reference the topic: keep the key's qualified
        // name (scope.key), the way the map's keyDefs name it.
        for (Map.Entry<String, TopicNode> entry : topicsByPath.entrySet()) {
            TopicNode topic = entry.getValue();
            Set<String> scopes = scopesByPath.getOrDefault(entry.getKey(), Set.of(""));
            Set<String> resolved = new LinkedHashSet<>();
            for (String key : topic.keyrefs) {
                resolved.addAll(resolveKeyNames(key, scopes));
            }
            topic.keyrefs.clear();
            topic.keyrefs.addAll(resolved);
            for (Map.Entry<String, String> link : topic.externalLinks.entrySet()) {
                if (link.getValue() != null) {
                    link.setValue(resolveKeyNames(link.getValue(), scopes).get(0));
                }
            }
        }
        // A relationship-table link through a key is a use of that key by
        // the topic showing the link: its page changes with the key. The
        // key resolves in the scope of the map topicref that names it.
        for (Map.Entry<String, List<MapKeyUse>> entry : mapKeyUsesByPath.entrySet()) {
            TopicNode topic = topicsByPath.get(entry.getKey());
            for (MapKeyUse use : entry.getValue()) {
                String qualified = resolveKeyNames(use.key().name(), Set.of(use.key().scope())).get(0);
                topic.keyrefs.add(qualified);
                if (use.url() != null) {
                    topic.externalLinks.replace(use.url(), null, qualified);
                }
            }
        }
        // A conkeyref read from the source counts once the resolved topic
        // confirms it: the topic reuses content from the key's target.
        for (Map.Entry<String, Set<String>> entry : conkeyrefsByPath.entrySet()) {
            TopicNode topic = topicsByPath.get(entry.getKey());
            if (topic == null) {
                continue;
            }
            Set<String> scopes = scopesByPath.getOrDefault(entry.getKey(), Set.of(""));
            for (String key : entry.getValue()) {
                for (String qualified : resolveKeyNames(key, scopes)) {
                    RawKeyDef def = findKeyDef(qualified);
                    if (def == null || def.def.target == null) {
                        continue;
                    }
                    for (Link link : topic.links) {
                        if ("generated-from".equals(link.relation) && link.target.equals(def.def.target)) {
                            topic.keyrefs.add(qualified);
                            break;
                        }
                    }
                }
            }
        }

        List<Object> nodes = new ArrayList<>();
        nodes.add(mapNode);
        Set<String> excluded = new HashSet<>();
        for (TopicNode topic : topicsByPath.values()) {
            if (!includeDrafts && "draft".equals(topic.status)) {
                info.accept("DITA2GRAPH020I: skipping " + topic.sourceFile + " (status=\"draft\")");
                excluded.add(topic.id);
                continue;
            }
            nodes.add(topic);
        }
        nodes.addAll(headings.values());
        // Nothing may point at a skipped draft: no node would be there.
        if (!excluded.isEmpty()) {
            mapNode.links.removeIf(l -> excluded.contains(l.target));
            for (TopicNode topic : topicsByPath.values()) {
                topic.links.removeIf(l -> excluded.contains(l.target));
            }
            for (TopicNode heading : headings.values()) {
                heading.links.removeIf(l -> excluded.contains(l.target));
            }
            for (KeyDef def : mapNode.keyDefs) {
                if (def.target != null && excluded.contains(def.target)) {
                    def.target = null;
                }
            }
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

    /**
     * The directory the input map's job.xml {@code path} is relative to:
     * its {@code src} with the path's own directories removed.
     */
    private static URI sourceBase(JobFile mapFile) {
        if (mapFile.src.isEmpty()) {
            return null;
        }
        try {
            URI base = new URI(mapFile.src).resolve(".");
            for (int i = 0; i < mapFile.path.chars().filter(c -> c == '/').count(); i++) {
                base = base.resolve("../");
            }
            return base;
        } catch (Exception e) {
            return null;
        }
    }

    /**
     * The file of the corpus a topic comes from, relative to the input
     * map's directory, as the author edits it. That is the job.xml path,
     * except for a copy DITA-OT writes under another name, whose
     * {@code src} is still the authored file: a {@code copy-to} target, or
     * a branch-filtering copy a {@code <ditavalref>} names with
     * {@code <dvrResourcePrefix>} or {@code <dvrResourceSuffix>}. The DITA-OT
     * documentation has two of the latter, {@code topics/build-using-dita-command.dita}
     * and {@code topics/first-build-using-dita-command.dita}, both from
     * {@code topics/using-dita-command.dita}: no file in the corpus has the
     * copy's name, so the MCP server's {@code validate_live} found nothing
     * to check. Falls back to the job.xml path when {@code src} is outside
     * the map's directory.
     */
    private String authoredPath(JobFile file) {
        if (sourceBase == null || file.src.isEmpty()) {
            return file.path;
        }
        try {
            URI relative = sourceBase.relativize(new URI(file.src));
            String path = relative.getPath();
            return relative.isAbsolute() || path == null || path.isEmpty() ? file.path : path;
        } catch (Exception e) {
            return file.path;
        }
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
                        String heading = isTopicgroup(child) || isResourceOnly(child) ? "" : navtitle(child);
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
                    if (isResourceOnly(child)) {
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

    /**
     * Records the keys {@code el} defines ({@code @keys}, one or more
     * names) unless an earlier definition in the same scope already
     * holds them: in DITA the first definition in map order wins. The
     * key names a topic (resolved to its id in pass 3, through the copy
     * when the topicref has a {@code copy-to}), an external URL, a local
     * non-topic file, or nothing (a text key, kept with its text).
     */
    private void recordKeyDefs(Element el, String mapPath) {
        String keys = el.getAttribute("keys").trim();
        if (keys.isEmpty()) {
            return;
        }
        String scope = keyScopeOf(el);
        String href = el.getAttribute("href");
        String format = el.getAttribute("format");
        for (String name : keys.split("\\s+")) {
            RawKeyDef raw = new RawKeyDef();
            if (keyDefs.putIfAbsent(scope + " " + name, raw) != null) {
                continue;
            }
            raw.def.name = name;
            raw.def.scope = scope.isEmpty() ? null : scope;
            raw.def.sourceFile = srcToPath.getOrDefault(el.getAttribute("xtrf"), mapPath);
            if (!href.isEmpty()) {
                String local = href.contains("#") ? href.substring(0, href.indexOf('#')) : href;
                if (isExternal(href, el.getAttribute("scope"))) {
                    if (href.startsWith("http://") || href.startsWith("https://")) {
                        raw.def.url = href;
                    }
                } else if (!local.isEmpty() && !format.isEmpty() && !"dita".equals(format)) {
                    raw.def.file = resolve(mapPath, local);
                } else if (!local.isEmpty()) {
                    String copyTo = el.getAttribute("copy-to");
                    raw.targetPath = !copyTo.isEmpty() && jobDitaPaths.contains(resolve(mapPath, copyTo))
                            ? resolve(mapPath, copyTo)
                            : resolve(mapPath, local);
                }
            }
            if (raw.targetPath == null) {
                String text = keyText(el);
                raw.def.text = text.isEmpty() ? null : text;
            }
        }
    }

    /**
     * Records the key definitions of every topicref of the map, in
     * document order -- relationship tables and topicrefs beyond
     * {@code args.dita2graph.depth} included, as DITA-OT's own key reader
     * does -- and, for each topic file a topicref names, the key scope of
     * that topicref: the scopes its keyrefs resolve in.
     */
    private void collectKeyDefsAndScopes(Element mapRoot, String mapPath) {
        NodeList all = mapRoot.getElementsByTagName("*");
        for (int i = 0; i < all.getLength(); i++) {
            Element el = (Element) all.item(i);
            if (!isTopicref(el)) {
                continue;
            }
            recordKeyDefs(el, mapPath);
            String keyref = keyName(el.getAttribute("keyref"));
            String trace = el.getAttribute("xtrc");
            if (!keyref.isEmpty() && !trace.isEmpty()) {
                mapKeyrefsByTrace.putIfAbsent(el.getAttribute("xtrf") + " " + trace,
                        new MapKeyref(keyref, keyScopeOf(el)));
            }
            String href = el.getAttribute("href");
            String format = el.getAttribute("format");
            if (href.isEmpty() || isExternal(href, el.getAttribute("scope"))
                    || (!format.isEmpty() && !"dita".equals(format))) {
                continue;
            }
            String local = href.contains("#") ? href.substring(0, href.indexOf('#')) : href;
            if (local.isEmpty()) {
                continue;
            }
            String copyTo = el.getAttribute("copy-to");
            String path = !copyTo.isEmpty() && jobDitaPaths.contains(resolve(mapPath, copyTo))
                    ? resolve(mapPath, copyTo)
                    : resolve(mapPath, local);
            scopesByPath.computeIfAbsent(path, p -> new LinkedHashSet<>()).add(keyScopeOf(el));
        }
    }

    /**
     * The qualified names ({@code scope.key}) of the definitions {@code
     * name} resolves to from each of {@code scopes}: in DITA a parent
     * scope's definition takes precedence over a child scope's, so the
     * scopes are tried from the root down; {@code name} may itself be
     * qualified relative to the scope. A name that resolves nowhere is
     * kept as it is (an undefined key).
     */
    private List<String> resolveKeyNames(String name, Set<String> scopes) {
        Set<String> found = new LinkedHashSet<>();
        for (String scope : scopes) {
            List<String> chain = new ArrayList<>();
            chain.add("");
            if (!scope.isEmpty()) {
                String[] parts = scope.split("\\.");
                StringBuilder prefix = new StringBuilder();
                for (String part : parts) {
                    prefix.append(prefix.length() == 0 ? "" : ".").append(part);
                    chain.add(prefix.toString());
                }
            }
            for (String base : chain) {
                RawKeyDef def = findKeyDef(base.isEmpty() ? name : base + "." + name);
                if (def != null) {
                    found.add(def.def.scope == null ? def.def.name : def.def.scope + "." + def.def.name);
                    break;
                }
            }
        }
        return found.isEmpty() ? List.of(name) : new ArrayList<>(found);
    }

    /** {@code processing-role="resource-only"}, which is a keydef's default when it has none. */
    private static boolean isResourceOnly(Element el) {
        String role = el.getAttribute("processing-role");
        if (!role.isEmpty()) {
            return "resource-only".equals(role);
        }
        String cls = el.getAttribute("class");
        return cls.isEmpty() ? "keydef".equals(el.getNodeName()) : cls.contains(" mapgroup-d/keydef ");
    }

    /** Whether the resolved topic has a preformatted element ({@code topic/pre}: codeblock, pre...). */
    private static boolean hasPreformatted(Element root) {
        NodeList all = root.getElementsByTagName("*");
        for (int i = 0; i < all.getLength(); i++) {
            Element el = (Element) all.item(i);
            String cls = el.getAttribute("class");
            if (cls.isEmpty() ? "codeblock".equals(el.getNodeName()) || "pre".equals(el.getNodeName())
                    : cls.contains(" topic/pre ")) {
                return true;
            }
        }
        return false;
    }

    /** The definition a (possibly scope-qualified, {@code scope.key}) key name refers to, or null. */
    private RawKeyDef findKeyDef(String qualified) {
        RawKeyDef def = keyDefs.get(" " + qualified);
        for (int dot = qualified.indexOf('.'); def == null && dot > 0; dot = qualified.indexOf('.', dot + 1)) {
            def = keyDefs.get(qualified.substring(0, dot) + " " + qualified.substring(dot + 1));
        }
        return def;
    }

    /**
     * The dotted key scope {@code el} is in: the first {@code @keyscope}
     * name of each ancestor, {@code el} included (its own keys belong to
     * the scope it opens), outermost first; empty for the root scope. The
     * root map's own {@code @keyscope} names the root scope, not a child.
     */
    private static String keyScopeOf(Element el) {
        Deque<String> names = new ArrayDeque<>();
        Element root = el.getOwnerDocument().getDocumentElement();
        for (Node n = el; n instanceof Element && n != root; n = n.getParentNode()) {
            String scope = ((Element) n).getAttribute("keyscope").trim();
            if (!scope.isEmpty()) {
                names.addFirst(scope.split("\\s+")[0]);
            }
        }
        return String.join(".", names);
    }

    /** A key's text: its keyword, else its link text, else its navtitle. */
    private static String keyText(Element el) {
        for (Element topicmeta : directChildren(el, "topicmeta")) {
            for (Element keywords : directChildren(topicmeta, "keywords")) {
                for (Element keyword : directChildren(keywords, "keyword")) {
                    String text = normalizeSpace(keyword.getTextContent());
                    if (!text.isEmpty()) {
                        return text;
                    }
                }
            }
            for (Element linktext : directChildren(topicmeta, "linktext")) {
                String text = normalizeSpace(linktext.getTextContent());
                if (!text.isEmpty()) {
                    return text;
                }
            }
        }
        return navtitle(el);
    }

    /** The key a {@code @keyref}/{@code @conkeyref} value names: {@code "key/element"} -&gt; {@code "key"}. */
    private static String keyName(String keyref) {
        String key = keyref.trim();
        int slash = key.indexOf('/');
        return slash >= 0 ? key.substring(0, slash) : key;
    }

    /**
     * Every key a resolved topic uses through {@code @keyref}, on any
     * element (a link, a keyword variable, an image), except the
     * navigation links DITA-OT generates from the map and the text it
     * copies into a related link (the target's shortdesc and title, with
     * the target's keys).
     */
    private static void collectKeyrefs(Element root, Set<String> into) {
        List<Element> elements = new ArrayList<>();
        elements.add(root);
        NodeList all = root.getElementsByTagName("*");
        for (int i = 0; i < all.getLength(); i++) {
            elements.add((Element) all.item(i));
        }
        for (Element el : elements) {
            String key = keyName(el.getAttribute("keyref"));
            if (key.isEmpty() || isInsideLink(el)
                    || (HIERARCHY_ROLES.contains(el.getAttribute("role")) && isInsideRelatedLinks(el))) {
                continue;
            }
            into.add(key);
        }
    }

    /**
     * Whether {@code el} is inside a {@code <link>} ({@code topic/link}):
     * in its link text or description, which DITA-OT fills from the
     * target topic's title and shortdesc.
     */
    private static boolean isInsideLink(Element el) {
        for (Node n = el.getParentNode(); n instanceof Element; n = n.getParentNode()) {
            Element ancestor = (Element) n;
            String cls = ancestor.getAttribute("class");
            if (cls.isEmpty() ? "link".equals(ancestor.getNodeName()) : cls.contains(" topic/link ")) {
                return true;
            }
            if ("related-links".equals(ancestor.getNodeName())) {
                return false;
            }
        }
        return false;
    }

    /** Whether an ancestor of {@code el} has {@code classToken} in its {@code @class} (its name, without one). */
    private static boolean hasAncestor(Element el, String classToken, String name) {
        for (Node n = el.getParentNode(); n instanceof Element; n = n.getParentNode()) {
            Element ancestor = (Element) n;
            String cls = ancestor.getAttribute("class");
            if (cls.isEmpty() ? name.equals(ancestor.getNodeName()) : (" " + cls + " ").contains(classToken)) {
                return true;
            }
        }
        return false;
    }

    /**
     * Reads what preprocessing removes from the topic's source file (its
     * job.xml {@code src}): the sample files its {@code <coderef>}s
     * include, and the keys its {@code @conkeyref}s name. A coderef counts
     * only when its parent element survived in the resolved topic (same
     * {@code xtrf} and element counter in {@code xtrc}), so a coderef
     * DITAVAL filtered out is not recorded. A source that does not parse
     * on its own (an entity its DTD would define) is skipped with
     * {@code DITA2GRAPH072I}.
     */
    private void scanSource(DocumentBuilder builder, JobFile file, Element resolvedRoot,
            Map<String, String> coderefSrcToPath, Set<String> codeRefs, Set<String> conkeyrefs) {
        if (!file.src.startsWith("file:")) {
            return;
        }
        Document doc;
        try {
            File source = new File(new URI(file.src));
            if (!source.isFile()) {
                return;
            }
            String raw = new String(Files.readAllBytes(source.toPath()), StandardCharsets.ISO_8859_1);
            if (!raw.contains("coderef") && !raw.contains("conkeyref")) {
                return;
            }
            doc = parse(builder, source);
        } catch (Exception e) {
            info.accept("DITA2GRAPH072I: could not read the source of " + file.path
                    + " for coderef/conkeyref (" + e.getMessage() + "); none recorded for it");
            return;
        }
        NodeList all = doc.getElementsByTagName("*");
        for (int i = 0; i < all.getLength(); i++) {
            Element el = (Element) all.item(i);
            String conkeyref = keyName(el.getAttribute("conkeyref"));
            if (!conkeyref.isEmpty()) {
                conkeyrefs.add(conkeyref);
            }
            if (!"coderef".equals(el.getNodeName())) {
                continue;
            }
            String path = null;
            String href = el.getAttribute("href");
            if (!href.isEmpty()) {
                String local = href.contains("#") ? href.substring(0, href.indexOf('#')) : href;
                try {
                    path = coderefSrcToPath.get(URI.create(file.src).resolve(local.replace(" ", "%20")).toString());
                } catch (IllegalArgumentException e) {
                    continue;
                }
            } else {
                RawKeyDef def = findKeyDef(keyName(el.getAttribute("keyref")));
                if (def != null && def.def.file != null && coderefSrcToPath.containsValue(def.def.file)) {
                    path = def.def.file;
                }
            }
            if (path != null && el.getParentNode() instanceof Element
                    && survived((Element) el.getParentNode(), doc, file.src, resolvedRoot)) {
                codeRefs.add(path);
            }
        }
    }

    /**
     * Whether the source element {@code el} is still in the resolved
     * topic: DITA-OT's {@code xtrc} names an element by its name and its
     * rank among the source file's elements of that name.
     */
    private static boolean survived(Element el, Document source, String src, Element resolvedRoot) {
        String tag = el.getNodeName();
        NodeList same = source.getElementsByTagName(tag);
        int rank = 0;
        for (int i = 0; i < same.getLength(); i++) {
            if (same.item(i) == el) {
                rank = i + 1;
                break;
            }
        }
        String prefix = tag + ":" + rank + ";";
        List<Element> candidates = descendants(resolvedRoot, tag);
        candidates.add(resolvedRoot);
        for (Element candidate : candidates) {
            if (src.equals(candidate.getAttribute("xtrf")) && candidate.getAttribute("xtrc").startsWith(prefix)) {
                return true;
            }
        }
        return false;
    }

    /** A topicref or one of its specializations (chapter, glossref, topichead...), by {@code @class}. */
    private static boolean isTopicref(Element el) {
        String cls = el.getAttribute("class");
        if (!cls.isEmpty()) {
            return cls.contains(" map/topicref ");
        }
        return Set.of("topicref", "topichead", "topicgroup", "keydef", "chapter", "part", "appendix", "glossref", "notices")
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
        BodyWalk walk = new BodyWalk(ownPath, ditaSrcToPath, bodyElements(root, topicType), root);
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
        // A composite file's nested topics (a <reference> holding one
        // <reference> per release, dita-ot/docs' release notes) are part
        // of what the topic publishes: their title, summary and body join
        // its text. Without this, a root with no body of its own had no
        // text at all.
        boolean nestedPart = !insideBody && isNestedTopicPart(el, walk.topicRoot);
        boolean bodyPart = !insideBody && walk.bodyRoots.contains(el);
        boolean nowInsideBody = insideBody || bodyPart || nestedPart;
        if (nestedPart || bodyPart) {
            walk.ownText.append(' ');
            walk.resolvedText.append(' ');
        }

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

    /** Tag names of topic elements, for a topic without {@code @class}. */
    private static final Set<String> TOPIC_TAGS = Set.of(
            "topic", "concept", "task", "reference", "glossentry", "glossgroup", "troubleshooting");
    /** Classes of the parts of a nested topic that carry its text. */
    private static final List<String> NESTED_TEXT_CLASSES = List.of(
            " topic/title ", " topic/shortdesc ", " topic/abstract ", " topic/body ");
    /** The same parts by name, for a topic without {@code @class}. */
    private static final Set<String> NESTED_TEXT_TAGS = Set.of(
            "title", "shortdesc", "abstract", "body", "conbody", "taskbody", "refbody", "troublebody",
            "glossterm", "glossdef");

    /** Whether {@code el} is a topic element (by {@code @class}, else by name). */
    private static boolean isTopicElement(Element el) {
        String cls = el.getAttribute("class");
        return cls.isEmpty() ? TOPIC_TAGS.contains(el.getNodeName()) : (" " + cls + " ").contains(" topic/topic ");
    }

    /**
     * Whether {@code el} is the title, summary or body of a topic nested
     * in {@code topicRoot} (at any depth), not of the root itself.
     */
    private static boolean isNestedTopicPart(Element el, Element topicRoot) {
        Node parent = el.getParentNode();
        if (!(parent instanceof Element) || parent == topicRoot || !isTopicElement((Element) parent)) {
            return false;
        }
        String cls = el.getAttribute("class");
        if (cls.isEmpty()) {
            return NESTED_TEXT_TAGS.contains(el.getNodeName());
        }
        String padded = " " + cls + " ";
        for (String part : NESTED_TEXT_CLASSES) {
            if (padded.contains(part)) {
                return true;
            }
        }
        return false;
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
            case "troubleshooting":
                return "troublebody";
            default:
                return "body";
        }
    }

    /**
     * The elements whose text is the topic's body: its first child whose
     * {@code @class} has {@code topic/body}, so a specialized body counts
     * too, by element name ({@link #bodyElementTag}) for a topic without
     * {@code @class}. A glossentry's definition is its {@code glossdef}
     * (a {@code topic/abstract}), and its {@code glossBody} (usage notes,
     * other forms) is a {@code topic/body}: both, in document order.
     */
    private static Set<Element> bodyElements(Element root, String topicType) {
        Set<Element> roots = new LinkedHashSet<>();
        NodeList children = root.getChildNodes();
        for (int i = 0; i < children.getLength(); i++) {
            Node n = children.item(i);
            if (n.getNodeType() != Node.ELEMENT_NODE) {
                continue;
            }
            Element child = (Element) n;
            String cls = " " + child.getAttribute("class") + " ";
            boolean glossdef = "glossentry".equals(topicType)
                    && (cls.contains(" glossentry/glossdef ") || "glossdef".equals(child.getNodeName()));
            if (glossdef || (cls.contains(" topic/body ") && roots.stream().noneMatch(r ->
                    (" " + r.getAttribute("class") + " ").contains(" topic/body ")))) {
                roots.add(child);
            }
        }
        if (roots.isEmpty()) {
            roots.addAll(directChildren(root, bodyElementTag(topicType)));
        }
        return roots;
    }

    /**
     * The topic type, read from the root element's {@code @class} so a
     * specialization takes its base type ({@code glossgroup} is a
     * concept); by element name for a topic without {@code @class}.
     */
    private static String topicTypeOf(Element root) {
        String cls = root.getAttribute("class");
        if (cls.trim().isEmpty()) {
            return mapTopicType(root.getTagName());
        }
        cls = " " + normalizeSpace(cls) + " ";
        if (cls.contains(" troubleshooting/troubleshooting ")) {
            return "troubleshooting";
        }
        if (cls.contains(" glossentry/glossentry ")) {
            return "glossentry";
        }
        if (cls.contains(" task/task ")) {
            return "task";
        }
        if (cls.contains(" reference/reference ")) {
            return "reference";
        }
        if (cls.contains(" concept/concept ")) {
            return "concept";
        }
        return "topic";
    }

    private static String mapTopicType(String rootTagName) {
        switch (rootTagName) {
            case "concept":
            case "task":
            case "reference":
            case "glossentry":
            case "troubleshooting":
                return rootTagName;
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
