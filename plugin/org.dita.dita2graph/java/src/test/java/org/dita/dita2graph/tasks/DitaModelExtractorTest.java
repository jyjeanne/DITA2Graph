package org.dita.dita2graph.tasks;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Exercises {@link DitaModelExtractor} against a fixture shaped exactly
 * like DITA-OT 4.4's real resolved output for {@code sample-docs/}
 * (docs/dev/phase-0-findings.md finding 5/6) -- job.xml's structure,
 * attribute names, and the map/topic XML shape below were copied from an
 * actual {@code .job.xml} and resolved topic files produced by a live
 * {@code dita --format html5} run, not guessed.
 */
class DitaModelExtractorTest {

    @Test
    void extractsMapAndTopicsWithRelationsMatchingSampleDocs(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML);
        write(tempDir, "user-guide.ditamap", MAP_XML);
        write(tempDir, "topics/configuration.dita", CONFIGURATION_XML);
        write(tempDir, "topics/installing-product-prereqs.dita", PREREQS_XML);
        write(tempDir, "topics/installing-product.dita", INSTALLING_PRODUCT_XML);

        List<String> warnings = new ArrayList<>();
        DitaModelExtractor extractor = new DitaModelExtractor(
                tempDir.toFile(), false, warnings::add, msg -> { });
        List<Object> nodes = extractor.extract();

        assertEquals(4, nodes.size(), "map + 3 topics");
        assertTrue(warnings.isEmpty(), "no unresolved links expected: " + warnings);

        MapNode map = (MapNode) nodes.get(0);
        assertEquals("user-guide", map.id);
        assertEquals("User Guide", map.title);
        assertEquals(2, map.links.size(), "map contains configuration and installing-product");
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("configuration")));
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("installing-product")));

        TopicNode installingProduct = findTopic(nodes, "installing-product");
        assertEquals("task", installingProduct.topicType);
        assertEquals("Installing Product", installingProduct.title);
        assertEquals("Steps to install the product in a production environment.", installingProduct.shortdesc);
        assertTrue(installingProduct.body.contains("Download the installer."),
                "taskbody text should be extracted: " + installingProduct.body);
        assertEquals(List.of("admin"), installingProduct.audience);
        assertEquals(List.of("enterprise"), installingProduct.product);
        assertEquals(List.of("install-task"), installingProduct.keys);
        // The xref in the task's <prereq> is a prerequisite ("requires");
        // the keyref'd one in <context> is a plain "references" whose key
        // is recorded.
        assertTrue(installingProduct.links.stream()
                .anyMatch(l -> l.relation.equals("references") && l.target.equals("configuration")));
        assertTrue(installingProduct.links.stream()
                .anyMatch(l -> l.relation.equals("requires") && l.target.equals("installing-product-prereqs")));
        assertEquals(Set.of("config-concept"), installingProduct.keyrefs);
        assertEquals(List.of("config-concept", "install-task"),
                map.keyDefs.stream().map(k -> k.name).toList());
        assertEquals(List.of("configuration", "installing-product"),
                map.keyDefs.stream().map(k -> k.target).toList());

        TopicNode configuration = findTopic(nodes, "configuration");
        assertEquals("concept", configuration.topicType);
        assertEquals(List.of("config-concept"), configuration.keys);
        assertNull(configuration.shortdesc);
        assertEquals("Configuration overview content goes here.", configuration.body);

        TopicNode prereqs = findTopic(nodes, "installing-product-prereqs");
        assertEquals("topic", prereqs.topicType);
        assertTrue(prereqs.links.stream()
                .anyMatch(l -> l.relation.equals("references") && l.target.equals("configuration")));
        // Generic <topic> uses <body> (not <conbody>/<taskbody>); getTextContent()
        // includes the <xref> element's own link text inline with the
        // surrounding prose, since DOM text extraction doesn't distinguish
        // link text from plain text.
        assertEquals("See Configuration Overview.", prereqs.body);
    }

    @Test
    void nestedTopicrefTopicheadAndTopicgroupAreAllWalked(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML_NESTED);
        write(tempDir, "user-guide.ditamap", MAP_XML_NESTED);
        // chapter/section carry a <related-links> block shaped exactly
        // like DITA-OT 4.4's own auto-generated parent/child navigation
        // links for a nested topicref (confirmed against a live run,
        // not guessed) -- proves isInsideRelatedLinks actually excludes
        // them, not just that the fixture never had any to begin with.
        write(tempDir, "topics/chapter.dita", CHAPTER_XML_WITH_RELATED_LINKS);
        write(tempDir, "topics/section.dita", SECTION_XML_WITH_RELATED_LINKS);
        write(tempDir, "topics/grouped.dita", topicXml("grouped", "Grouped"));
        write(tempDir, "topics/another.dita", topicXml("another", "Another"));

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();
        assertTrue(warnings.isEmpty(), "no unresolved links expected: " + warnings);
        assertEquals(6, nodes.size(), "map + 4 topics + the topichead");

        MapNode map = (MapNode) nodes.get(0);
        // "chapter" is a top-level topicref -> map contains it directly.
        // The <topichead navtitle="Group"> becomes a heading node that
        // contains "grouped"; "another", inside a <topicgroup> (no title
        // to show), skips through to the map, same as if the wrapper
        // weren't there. "section" is NOT here: it's nested *inside* the
        // "chapter" topicref, so it belongs to chapter, not the map.
        assertEquals(3, map.links.size(), "map: " + map.links);
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("chapter")));
        assertTrue(map.links.stream()
                .anyMatch(l -> l.relation.equals("contains") && l.target.equals("topichead--group")));
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("another")));

        TopicNode group = findTopic(nodes, "topichead--group");
        assertEquals("topichead", group.topicType);
        assertEquals("Group", group.title);
        assertEquals("user-guide.ditamap#topichead--group", group.sourceFile);
        assertEquals(List.of("grouped"), group.links.stream().map(l -> l.target).toList());

        TopicNode chapter = findTopic(nodes, "chapter");
        // Exactly the "contains" edge to section -- NOT also a spurious
        // "references"/"requires" edge from the <related-links> child
        // link DITA-OT generated for the same relationship.
        assertEquals(1, chapter.links.size(), "chapter: " + chapter.links);
        assertTrue(chapter.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("section")));

        TopicNode section = findTopic(nodes, "section");
        // section's own <related-links> has a "parent" link back to
        // chapter; that must not become a references/requires edge
        // either -- section has no real links of its own here.
        assertTrue(section.links.isEmpty(), "section: " + section.links);
    }

    @Test
    void maxDepthLimitsHowManyLevelsOfContainmentAreCaptured(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML_NESTED);
        write(tempDir, "user-guide.ditamap", MAP_XML_NESTED);
        write(tempDir, "topics/chapter.dita", CHAPTER_XML_WITH_RELATED_LINKS);
        write(tempDir, "topics/section.dita", SECTION_XML_WITH_RELATED_LINKS);
        write(tempDir, "topics/grouped.dita", topicXml("grouped", "Grouped"));
        write(tempDir, "topics/another.dita", topicXml("another", "Another"));

        // depth=1: top-level entries (chapter, the "Group" topichead,
        // another -- a topicgroup doesn't consume a level) are still
        // captured, but "section" (nested one level inside "chapter",
        // i.e. level 2) and "grouped" (under the heading) are not --
        // their topic nodes still exist (Pass 2 extracts every resolved
        // topic regardless), they just have no incoming contains edge.
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, 1, msg -> { }, msg -> { }).extract();
        assertEquals(6, nodes.size(), "the topic itself is still extracted, just not contained");

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(3, map.links.size(), "map: " + map.links);
        assertTrue(map.links.stream().anyMatch(l -> l.target.equals("chapter")));
        assertTrue(map.links.stream().anyMatch(l -> l.target.equals("topichead--group")));
        assertTrue(map.links.stream().anyMatch(l -> l.target.equals("another")));

        TopicNode chapterAtDepth1 = findTopic(nodes, "chapter");
        assertTrue(chapterAtDepth1.links.isEmpty(),
                "section is one level too deep for depth=1: " + chapterAtDepth1.links);
        assertTrue(findTopic(nodes, "topichead--group").links.isEmpty());

        // depth=2 (or "unlimited", the default) restores the nested edges.
        List<Object> unlimitedNodes =
                new DitaModelExtractor(tempDir.toFile(), false, 2, msg -> { }, msg -> { }).extract();
        TopicNode chapterAtDepth2 = findTopic(unlimitedNodes, "chapter");
        assertTrue(chapterAtDepth2.links.stream().anyMatch(l -> l.target.equals("section")));
        assertTrue(findTopic(unlimitedNodes, "topichead--group").links.stream()
                .anyMatch(l -> l.target.equals("grouped")));
    }

    @Test
    void bookmapChaptersGlossrefsAndNavtitleOnlyChaptersAreWalked(@TempDir Path tempDir) throws Exception {
        // Shaped after DITA-OT 4.4's resolved UserManual.ditamap of the
        // Oxygen XML user guide: most chapters come out renamed to
        // <topicref>, but a chapter pointing straight at a topic and a
        // navtitle-only chapter keep their name; only @class says what
        // they are. The glossary entries are <glossref>s, the ditavalref
        // is not a topic, and the title is booktitle/mainbooktitle.
        write(tempDir, ".job.xml", jobXml("topics/intro.dita", "topics/add-ons.dita",
                "glossary/anchor.dita", "topics/copyright.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<bookmap id=\"manual\" class=\"- map/map bookmap/bookmap \">"
                + "<booktitle class=\"- topic/title bookmap/booktitle \">"
                + "<mainbooktitle class=\"- topic/ph bookmap/mainbooktitle \"><ph class=\"- topic/ph \">Product</ph>"
                + "  28.0  </mainbooktitle>"
                + "<booktitlealt class=\"- topic/ph bookmap/booktitlealt \">User Guide</booktitlealt></booktitle>"
                + "<frontmatter class=\"- map/topicref bookmap/frontmatter \">"
                + "<booklists class=\"- map/topicref bookmap/booklists \"><toc class=\"- map/topicref bookmap/toc \"/>"
                + "</booklists></frontmatter>"
                + "<topicref href=\"topics/intro.dita\" navtitle=\"Introduction\" class=\"- map/topicref bookmap/chapter \"/>"
                + "<chapter href=\"topics/add-ons.dita\" navtitle=\"Add-ons\" class=\"- map/topicref bookmap/chapter \"/>"
                + "<chapter navtitle=\"Glossary\" xtrf=\"file:/src/user-guide.ditamap\" "
                + "class=\"- map/topicref bookmap/chapter \">"
                + "<glossref href=\"glossary/anchor.dita\" keys=\"anchor\" class=\"+ map/topicref glossref-d/glossref \"/>"
                + "</chapter>"
                + "<backmatter class=\"- map/topicref bookmap/backmatter \">"
                + "<notices href=\"topics/copyright.dita\" class=\"- map/topicref bookmap/notices \"/></backmatter>"
                + "<ditavalref href=\"editor.ditaval\" format=\"ditaval\" class=\"+ map/topicref ditavalref-d/ditavalref \"/>"
                + "</bookmap>");
        write(tempDir, "topics/intro.dita", topicXml("intro", "Introduction"));
        write(tempDir, "topics/add-ons.dita", topicXml("add-ons", "Add-ons"));
        write(tempDir, "glossary/anchor.dita", topicXml("anchor", "Anchor"));
        write(tempDir, "topics/copyright.dita", topicXml("copyright", "Copyright"));

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();
        assertTrue(warnings.isEmpty(), "no unresolved targets expected: " + warnings);

        MapNode map = (MapNode) nodes.get(0);
        assertEquals("Product 28.0", map.title);
        assertEquals(List.of("intro", "add-ons", "topichead--glossary", "copyright"),
                map.links.stream().map(l -> l.target).toList());

        TopicNode glossary = findTopic(nodes, "topichead--glossary");
        assertEquals("topichead", glossary.topicType);
        assertEquals("Glossary", glossary.title);
        assertEquals("user-guide.ditamap#topichead--glossary", glossary.sourceFile);
        assertEquals(List.of("anchor"), glossary.links.stream().map(l -> l.target).toList());
        assertEquals(List.of("anchor"), findTopic(nodes, "anchor").keys);
    }

    private static String topicXml(String id, String title) {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?><topic id=\"" + id + "\">"
                + "<title>" + title + "</title><body><p>" + title + " content.</p></body></topic>";
    }

    @Test
    void draftTopicsAreExcludedUnlessIncludeDraftsIsSet(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML_DRAFT_ONLY);
        write(tempDir, "user-guide.ditamap", MAP_XML_DRAFT_ONLY);
        write(tempDir, "topics/configuration.dita", DRAFT_CONFIGURATION_XML);

        List<String> infos = new ArrayList<>();
        List<Object> excluded = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, infos::add).extract();
        assertEquals(1, excluded.size(), "draft topic excluded, only the map remains");
        assertTrue(infos.stream().anyMatch(m -> m.contains("DITA2GRAPH020I")));

        List<Object> included = new DitaModelExtractor(tempDir.toFile(), true, msg -> { }, msg -> { }).extract();
        assertEquals(2, included.size(), "draft topic included with includeDrafts=true");
    }

    @Test
    void navrefIsDetectedAndWarnedAboutRatherThanSilentlyIgnored(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML);
        write(tempDir, "user-guide.ditamap", MAP_XML_WITH_NAVREF);
        write(tempDir, "topics/configuration.dita", CONFIGURATION_XML);
        write(tempDir, "topics/installing-product-prereqs.dita", PREREQS_XML);
        write(tempDir, "topics/installing-product.dita", INSTALLING_PRODUCT_XML);

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();

        assertEquals(4, nodes.size(), "map + 3 topics -- navref contributes nothing, but doesn't break the rest");
        assertTrue(
                warnings.stream().anyMatch(m -> m.contains("DITA2GRAPH060W") && m.contains("nav.ditamap")),
                "expected a DITA2GRAPH060W warning naming the unresolved navref target: " + warnings);
    }

    @Test
    void externalTopicrefIsSilentlyExcludedFromContainmentNotWarnedAbout(@TempDir Path tempDir) throws Exception {
        // A keyref-resolved <keydef href="https://..." scope="external">
        // -- DITA-OT resolves the keyref onto the topicref itself during
        // preprocessing, so by the time this class sees it, it's a
        // topicref with a real https:// href and scope="external", no
        // different from an authored one. Common real-world pattern
        // (confirmed against dita-ot/docs' own conference-talk maps: a
        // "related talk" pointing at an external video page) that
        // previously produced a DITA2GRAPH010W "unresolved topicref
        // target" per external link -- there was never a local topic for
        // it to be, so that was noise, not a real gap.
        String jobXml = "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/configuration.dita\" uri=\"topics/configuration.dita\" "
                + "path=\"topics/configuration.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
                + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>";
        String mapXml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/configuration.dita\"/>"
                + "<topicref href=\"https://example.com/talk.html\" scope=\"external\" format=\"html\"/>"
                + "</map>";
        write(tempDir, ".job.xml", jobXml);
        write(tempDir, "user-guide.ditamap", mapXml);
        write(tempDir, "topics/configuration.dita", CONFIGURATION_XML);

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();

        assertEquals(2, nodes.size(), "map + 1 real topic -- the external topicref creates no phantom node");
        assertTrue(warnings.isEmpty(), "external topicref should be silently excluded, not warned about: " + warnings);

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(1, map.links.size(), "only the real topicref should produce a contains edge");
        assertEquals("configuration", map.links.get(0).target);
    }

    @Test
    void repeatedCrossReferencesToTheSameTargetProduceOneEdgeNotOnePerMention(@TempDir Path tempDir)
            throws Exception {
        // Confirmed against a real, large third-party corpus
        // (dita-ot/docs): prose that mentions the same keyref-based
        // cross-reference several times in one topic (a glossary-style
        // term referenced from multiple sentences) produced one edge per
        // mention, so the same target ended up listed under one topic's
        // "# Requires"/"# References" heading up to four times over --
        // real, visible bundle noise (`okf_validator`'s "redundant link"
        // warning) a small hand-built fixture with one mention per
        // target never surfaces. Two keyref'd mentions of "configuration"
        // plus two bare mentions of "installing-product-prereqs" here,
        // matching that real shape.
        String jobXml = "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/configuration.dita\" uri=\"topics/configuration.dita\" "
                + "path=\"topics/configuration.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/topics/prereqs.dita\" uri=\"topics/prereqs.dita\" "
                + "path=\"topics/prereqs.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/topics/reuser.dita\" uri=\"topics/reuser.dita\" "
                + "path=\"topics/reuser.dita\" format=\"dita\" has-keyref=\"true\" target=\"true\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
                + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>";
        String mapXml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/configuration.dita\"/>"
                + "<topicref href=\"topics/prereqs.dita\"/>"
                + "<topicref href=\"topics/reuser.dita\"/>"
                + "</map>";
        write(tempDir, ".job.xml", jobXml);
        write(tempDir, "user-guide.ditamap", mapXml);
        write(tempDir, "topics/configuration.dita", CONFIGURATION_XML);
        write(tempDir, "topics/prereqs.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"installing-product-prereqs\"><title>Prereqs</title>"
                + "<body><p>Prereqs.</p></body></topic>");
        write(tempDir, "topics/reuser.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"reuser\"><title>Reuser Topic</title><conbody>"
                + "<p>See <xref href=\"configuration.dita\" keyref=\"config\">Configuration</xref> for setup, "
                + "and again see <xref href=\"configuration.dita\" keyref=\"config\">Configuration</xref> "
                + "for details.</p>"
                + "<p>Also see <xref href=\"prereqs.dita\">Prereqs</xref> and "
                + "<xref href=\"prereqs.dita\">Prereqs</xref> once more.</p>"
                + "</conbody></concept>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode reuser = findTopic(nodes, "reuser");
        long referencesToConfig = reuser.links.stream()
                .filter(l -> l.relation.equals("references") && l.target.equals("configuration"))
                .count();
        long referencesToPrereqs = reuser.links.stream()
                .filter(l -> l.relation.equals("references") && l.target.equals("installing-product-prereqs"))
                .count();
        assertEquals(1, referencesToConfig,
                "two keyref'd mentions of the same target should collapse to one edge: " + reuser.links);
        assertEquals(Set.of("config"), reuser.keyrefs);
        assertEquals(1, referencesToPrereqs,
                "two bare mentions of the same target should collapse to one references edge: " + reuser.links);
    }

    @Test
    void duplicateTopicIdIsDisambiguatedInsteadOfSilentlyOverwritingTheEarlierTopic(@TempDir Path tempDir)
            throws Exception {
        // Confirmed against a real, large third-party corpus
        // (dita-ot/docs): 33 separate topics all sharing the literal
        // root id "ID" -- an unfilled authoring-template placeholder.
        // Without disambiguation, both would map to the same OKF
        // concept file path (okf/topics/ID.md) and the second one
        // written would silently overwrite the first's content --
        // confirmed directly by running the un-fixed extractor against
        // that corpus (33 nodes named "ID" in graph.json, but only one
        // topic's actual content survived in the bundle). Two different
        // source files sharing id="ID" here, matching that real shape
        // exactly (not a synthetic simplification of it).
        String jobXml = "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/first.dita\" uri=\"topics/first.dita\" "
                + "path=\"topics/first.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/topics/second.dita\" uri=\"topics/second.dita\" "
                + "path=\"topics/second.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
                + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>";
        String mapXml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/first.dita\"/>"
                + "<topicref href=\"topics/second.dita\"/>"
                + "</map>";
        write(tempDir, ".job.xml", jobXml);
        write(tempDir, "user-guide.ditamap", mapXml);
        write(tempDir, "topics/first.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"ID\"><title>First Topic</title>"
                + "<conbody><p>First topic's own content.</p></conbody></concept>");
        write(tempDir, "topics/second.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"ID\"><title>Second Topic</title>"
                + "<conbody><p>Second topic's own content.</p></conbody></concept>");

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();

        assertEquals(3, nodes.size(), "map + both topics -- neither one should be silently dropped");
        TopicNode first = findTopic(nodes, "ID");
        assertEquals("First Topic", first.title, "the earlier file keeps the plain, un-disambiguated id");
        TopicNode second = findTopic(nodes, "ID--topics-second.dita");
        assertEquals("Second Topic", second.title);
        assertNotEquals(first.id, second.id, "both topics must end up with distinct graph node ids");

        assertTrue(
                warnings.stream().anyMatch(m -> m.contains("DITA2GRAPH070W") && m.contains("topics/second.dita")
                        && m.contains("topics/first.dita")),
                "expected a DITA2GRAPH070W warning naming both files sharing the id: " + warnings);
    }

    @Test
    void windowsSeparatorsInJobXmlPathsAreNormalized(@TempDir Path tempDir) throws Exception {
        // A real DITA-OT 4.4.1 run on Windows writes job.xml paths with
        // backslashes ("topics\configuration.dita"). Before normalization,
        // none of them matched resolve()'s '/'-separated link targets (456
        // false DITA2GRAPH010W warnings against dita-ot/docs) and a
        // disambiguated id kept its backslashes.
        write(tempDir, ".job.xml", JOB_XML.replace("path=\"topics/", "path=\"topics\\"));
        write(tempDir, "user-guide.ditamap", MAP_XML);
        write(tempDir, "topics/configuration.dita", CONFIGURATION_XML);
        write(tempDir, "topics/installing-product-prereqs.dita", PREREQS_XML);
        write(tempDir, "topics/installing-product.dita", INSTALLING_PRODUCT_XML);

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();

        assertTrue(warnings.isEmpty(), "no unresolved links expected: " + warnings);
        MapNode map = (MapNode) nodes.get(0);
        assertEquals(2, map.links.size(), "map contains configuration and installing-product");
        TopicNode installingProduct = findTopic(nodes, "installing-product");
        assertEquals("topics/installing-product.dita", installingProduct.sourceFile);
        assertEquals(List.of("install-task"), installingProduct.keys);
        assertTrue(installingProduct.links.stream()
                .anyMatch(l -> l.relation.equals("requires") && l.target.equals("installing-product-prereqs")));
    }

    @Test
    void duplicateTopicIdFromWindowsPathIsDisambiguatedWithoutBackslashes(@TempDir Path tempDir)
            throws Exception {
        String jobXml = "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/first.dita\" uri=\"topics/first.dita\" "
                + "path=\"topics\\first.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/topics/second.dita\" uri=\"topics/second.dita\" "
                + "path=\"topics\\second.dita\" format=\"dita\" target=\"true\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
                + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>";
        write(tempDir, ".job.xml", jobXml);
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/first.dita\"/><topicref href=\"topics/second.dita\"/></map>");
        write(tempDir, "topics/first.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"ID\"><title>First Topic</title><conbody><p>First.</p></conbody></concept>");
        write(tempDir, "topics/second.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"ID\"><title>Second Topic</title><conbody><p>Second.</p></conbody></concept>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        assertEquals("Second Topic", findTopic(nodes, "ID--topics-second.dita").title);
    }

    @Test
    void generatedFromIsExtractedFromXtrfMismatches(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML_GENERATED_FROM);
        write(tempDir, "user-guide.ditamap", MAP_XML_GENERATED_FROM);
        write(tempDir, "topics/source.dita", SOURCE_XML);
        write(tempDir, "topics/reuser.dita", REUSER_XML);

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode source = findTopic(nodes, "source");
        assertTrue(source.links.isEmpty(), "source's own content should have no generated-from edges: " + source.links);
        // The source topic keeps its own text in full -- it's the
        // canonical location this content lives, not a reuser of it.
        assertEquals("Reusable content.", source.body);

        TopicNode reuser = findTopic(nodes, "reuser");
        // Exactly one generated-from edge to "source" -- not two, even
        // though two of reuser's elements carry source.dita's xtrf
        // (finding 15's per-topic dedup, not one edge per reused element).
        assertEquals(1, reuser.links.size(), "reuser: " + reuser.links);
        assertTrue(reuser.links.stream()
                .anyMatch(l -> l.relation.equals("generated-from") && l.target.equals("source")));
        // Canonical-node deduplication (docs/dev/canonical-node-dedup-spec.md):
        // reuser's own body text keeps its unreused paragraph but excludes
        // both conref/conkeyref-pulled ones -- that text already lives in
        // source's own body above, reachable via the generated-from edge
        // just asserted, so duplicating it here too would inflate both the
        // OKF bundle and RAG chunk text with copies of identical content.
        assertEquals("Own content not reused.", reuser.body,
                "reuser's body should exclude both conref/conkeyref-pulled paragraphs: " + reuser.body);
    }

    @Test
    void bodyTextExcludesOnlyTheReusedSubtreeNotSiblingOwnContent(@TempDir Path tempDir) throws Exception {
        // A single element (<step>) with one authored child (<cmd>) and
        // one conref'd child (<info>) -- proves exclusion happens per
        // reused subtree, not per whole topic: the authored sibling
        // survives even though it's nested alongside reused content, not
        // a top-level sibling of it (docs/dev/canonical-node-dedup-spec.md's
        // "reused element nested inside the reusing topic's own structure"
        // edge case).
        write(tempDir, ".job.xml", JOB_XML_GENERATED_FROM);
        write(tempDir, "user-guide.ditamap", MAP_XML_GENERATED_FROM);
        write(tempDir, "topics/source.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"source\"><title>Source Topic</title>"
                + "<conbody><p xtrf=\"file:/src/topics/source.dita\">Reusable content.</p></conbody>"
                + "</concept>");
        write(tempDir, "topics/reuser.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<task id=\"reuser\"><title>Reuser Topic</title>"
                + "<taskbody><steps><step xtrf=\"file:/src/topics/reuser.dita\">"
                + "<cmd xtrf=\"file:/src/topics/reuser.dita\">Click Save.</cmd>"
                + "<info xtrf=\"file:/src/topics/source.dita\">Reusable content.</info>"
                + "</step></steps></taskbody></task>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode reuser = findTopic(nodes, "reuser");
        assertEquals("Click Save.", reuser.body,
                "authored <cmd> text should survive; conref'd <info> text should not: " + reuser.body);
    }

    @Test
    void inlineReuseMidSentenceIsPreservedNotExcluded(@TempDir Path tempDir) throws Exception {
        // A conref'd <ph> in the middle of an authored sentence -- unlike
        // a whole reused <p>/<step>, excluding this would leave a
        // grammatically mangled sentence behind, not cleanly drop a
        // self-contained chunk of content
        // (docs/dev/canonical-node-dedup-spec.md's stated v1 policy:
        // "leave it in rather than mangle prose"). Still recorded as
        // generated-from (it's still genuinely reused content), just not
        // excluded from body text the way a block-level reuse is.
        write(tempDir, ".job.xml", JOB_XML_GENERATED_FROM);
        write(tempDir, "user-guide.ditamap", MAP_XML_GENERATED_FROM);
        write(tempDir, "topics/source.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"source\"><title>Source Topic</title>"
                + "<conbody><p xtrf=\"file:/src/topics/source.dita\">Save</p></conbody>"
                + "</concept>");
        write(tempDir, "topics/reuser.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"reuser\"><title>Reuser Topic</title>"
                + "<conbody><p xtrf=\"file:/src/topics/reuser.dita\">Click the "
                + "<ph xtrf=\"file:/src/topics/source.dita\">Save</ph> button.</p></conbody>"
                + "</concept>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode reuser = findTopic(nodes, "reuser");
        assertEquals("Click the Save button.", reuser.body,
                "inline conref'd <ph> text should survive intact, not be dropped mid-sentence: " + reuser.body);
        // Still genuinely reused content -- generated-from must still
        // fire even though the text itself wasn't excluded.
        assertTrue(reuser.links.stream()
                .anyMatch(l -> l.relation.equals("generated-from") && l.target.equals("source")));
    }

    @Test
    void generatedFromIsDetectedOutsideTheBodyToo(@TempDir Path tempDir) throws Exception {
        // The combined body-text/generated-from walk starts at the topic
        // root, not the body element -- a conref'd <shortdesc> (outside
        // the body entirely) must still produce a generated-from edge,
        // proving the merge didn't narrow generated-from's scope down to
        // the body subtree the way body-text extraction is deliberately
        // scoped.
        write(tempDir, ".job.xml", JOB_XML_GENERATED_FROM);
        write(tempDir, "user-guide.ditamap", MAP_XML_GENERATED_FROM);
        write(tempDir, "topics/source.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"source\"><title>Source Topic</title>"
                + "<shortdesc xtrf=\"file:/src/topics/source.dita\">Shared summary.</shortdesc>"
                + "<conbody/></concept>");
        write(tempDir, "topics/reuser.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"reuser\"><title>Reuser Topic</title>"
                + "<shortdesc xtrf=\"file:/src/topics/source.dita\">Shared summary.</shortdesc>"
                + "<conbody><p xtrf=\"file:/src/topics/reuser.dita\">Own content.</p></conbody>"
                + "</concept>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode reuser = findTopic(nodes, "reuser");
        assertTrue(reuser.links.stream()
                        .anyMatch(l -> l.relation.equals("generated-from") && l.target.equals("source")),
                "reused <shortdesc>, outside the body, should still produce a generated-from edge: " + reuser.links);
        assertEquals("Own content.", reuser.body);
    }

    @Test
    void uicontrolsAreExtractedFromTheWholeBodyAndCmdUicontrolsOnlyFromCmd(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", JOB_XML_UICONTROL);
        write(tempDir, "user-guide.ditamap", MAP_XML_UICONTROL);
        write(tempDir, "topics/save-task.dita", SAVE_TASK_XML);

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode task = findTopic(nodes, "save-task");
        assertEquals(List.of("Save"), task.cmdUicontrols, "only the <cmd>-scoped uicontrol: " + task.cmdUicontrols);
        // The whole body also picks up "Mentioned Elsewhere", found
        // outside <cmd> (in <context>, which comes first in document
        // order), which cmdUicontrols must not.
        assertEquals(List.of("Mentioned Elsewhere", "Save"), task.uicontrols, "task: " + task.uicontrols);
    }

    @Test
    void generatedFromNamesEachReusedFragmentAndResolvedBodyKeepsThemBetweenMarkers(@TempDir Path tempDir)
            throws Exception {
        // Shaped after DITA-OT 4.4.1's resolved output (dita-ot/docs,
        // topics/plugins-installing.dita): a conref'd element loses its
        // @id in the reusing topic but keeps the xtrc it has in the source
        // topic's own resolved copy, which still carries the @id.
        write(tempDir, ".job.xml", JOB_XML_GENERATED_FROM);
        write(tempDir, "user-guide.ditamap", MAP_XML_GENERATED_FROM);
        write(tempDir, "topics/source.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"source\" xtrf=\"file:/src/topics/source.dita\" xtrc=\"concept:1;2:30\">"
                + "<title>Source Topic</title><conbody>"
                + "<p id=\"warning\" xtrf=\"file:/src/topics/source.dita\" xtrc=\"p:1;5:20\">Reusable content.</p>"
                + "<p xtrf=\"file:/src/topics/source.dita\" xtrc=\"p:2;6:8\">Paragraph without an id.</p>"
                + "</conbody></concept>");
        write(tempDir, "topics/reuser.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<task id=\"reuser\"><title>Reuser Topic</title><taskbody>"
                + "<context xtrf=\"file:/src/topics/reuser.dita\">"
                + "<p xtrf=\"file:/src/topics/source.dita\" xtrc=\"p:1;5:20\">Reusable content.</p>"
                + "<p xtrf=\"file:/src/topics/reuser.dita\">Own content.</p>"
                + "<p xtrf=\"file:/src/topics/source.dita\" xtrc=\"p:2;6:8\">Paragraph without an id.</p>"
                + "</context></taskbody></task>");

        List<String> infos = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, infos::add).extract();

        TopicNode reuser = findTopic(nodes, "reuser");
        Link generatedFrom = reuser.links.stream()
                .filter(l -> l.relation.equals("generated-from"))
                .findFirst()
                .orElseThrow();
        assertEquals("source", generatedFrom.target);
        // The id found in the source topic, else element@line from the xtrc.
        assertEquals(List.of("warning", "p@6"), generatedFrom.fragments);
        // The author of the source file hears about the element without an
        // @id, whose name changes with its line; not about the one with it.
        assertEquals(List.of("DITA2GRAPH073I: topics/source.dita: 1 reused element(s) without an @id (p@6), "
                        + "named by their line: give them an @id so their fragment names stay stable"),
                infos.stream().filter(m -> m.startsWith("DITA2GRAPH073I")).toList());
        assertEquals("Own content.", reuser.body, "the deduplicated body is unchanged");
        assertEquals("[reused from topics/source.dita#warning] Reusable content. [end reused] Own content. "
                        + "[reused from topics/source.dita#p@6] Paragraph without an id. [end reused]",
                reuser.resolvedBody);
        assertNull(findTopic(nodes, "source").resolvedBody, "a topic that reuses nothing gets no resolved body");
    }

    @Test
    void relationshipTableLinksBecomeReferencesButHierarchyLinksDoNot(@TempDir Path tempDir) throws Exception {
        // DITA-OT 4.4.1's maplink step writes reltable links with
        // role="friend" and hierarchy navigation with parent/child/
        // sibling roles into the same <related-links> block (confirmed
        // against dita-ot/docs' resolved topics/build-using-dita-command.dita).
        write(tempDir, ".job.xml", jobXml("topics/a.dita", "topics/b.dita", "topics/c.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/a.dita\"/><topicref href=\"topics/b.dita\"/><topicref href=\"topics/c.dita\"/>"
                + "</map>");
        write(tempDir, "topics/a.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"a\"><title>A</title>"
                + "<conbody><p>See <xref href=\"c.dita\" keyref=\"c-key\">C</xref>.</p></conbody>"
                + "<related-links>"
                + "<linkpool xtrf=\"file:/src/reltable.ditamap\"><link href=\"b.dita\" role=\"friend\" scope=\"local\" format=\"dita\"/></linkpool>"
                + "<linkpool xtrf=\"file:/src/reltable.ditamap\"><link href=\"c.dita\" role=\"friend\" scope=\"local\" format=\"dita\"/></linkpool>"
                + "<linkpool><link href=\"b.dita\" role=\"sibling\" scope=\"local\" format=\"dita\"/></linkpool>"
                + "<linkpool><link href=\"https://example.com/talk\" role=\"friend\" scope=\"external\" format=\"html\"/></linkpool>"
                + "</related-links></concept>");
        write(tempDir, "topics/b.dita", topicXml("b", "B"));
        write(tempDir, "topics/c.dita", topicXml("c", "C"));

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();
        assertTrue(warnings.isEmpty(), "no unresolved links expected: " + warnings);

        TopicNode a = findTopic(nodes, "a");
        // references c from the prose; references b from the reltable. The
        // reltable link to c adds nothing (the prose already links c), the
        // sibling link is navigation, and the external one leaves the corpus.
        assertEquals(2, a.links.size(), "a: " + a.links.stream().map(l -> l.relation + "->" + l.target).toList());
        assertTrue(a.links.stream().anyMatch(l -> l.relation.equals("references") && l.target.equals("c")));
        // ... but is an external link of a, through no key.
        assertEquals(Set.of("https://example.com/talk"), a.externalLinks.keySet());
        assertNull(a.externalLinks.get("https://example.com/talk"));
        assertTrue(a.links.stream().anyMatch(l -> l.relation.equals("references") && l.target.equals("b")));
    }

    @Test
    void resourceOnlyTopicrefGetsNoContainsEdgeButStaysANode(@TempDir Path tempDir) throws Exception {
        // DITA-OT cascades processing-role="resource-only" onto every
        // topicref it covers (confirmed in dita-ot/docs' resolved
        // userguide.ditamap), so the topicref's own attribute is enough.
        write(tempDir, ".job.xml", jobXml("topics/a.dita", "topics/warehouse.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/a.dita\"/>"
                + "<topicgroup processing-role=\"resource-only\">"
                + "<topicref href=\"topics/warehouse.dita\" processing-role=\"resource-only\"/>"
                // a is published above: also naming it as a resource
                // does not take it out of the table of contents.
                + "<topicref href=\"topics/a.dita\" processing-role=\"resource-only\"/>"
                + "</topicgroup></map>");
        write(tempDir, "topics/a.dita", topicXml("a", "A"));
        write(tempDir, "topics/warehouse.dita", topicXml("warehouse", "Conref warehouse"));

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(List.of("a"), map.links.stream().map(l -> l.target).toList());
        TopicNode warehouse = findTopic(nodes, "warehouse");
        assertEquals("resource-only", warehouse.processingRole);
        assertTrue(warehouse.toJson().contains("\"processingRole\":\"resource-only\""), warehouse.toJson());
        TopicNode a = findTopic(nodes, "a");
        assertNull(a.processingRole);
        assertTrue(!a.toJson().contains("processingRole"), a.toJson());
    }

    @Test
    void rootClassAttributeIsKeptAsDitaClass(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/faq.dita", "topics/plain.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/faq.dita\"/><topicref href=\"topics/plain.dita\"/></map>");
        // A specialization the topic type folds into "concept".
        write(tempDir, "topics/faq.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"faq\" class=\"- topic/topic concept/concept  faq/faq \">"
                + "<title>FAQ</title><conbody><p>Answers.</p></conbody></concept>");
        write(tempDir, "topics/plain.dita", topicXml("plain", "Plain"));

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode faq = findTopic(nodes, "faq");
        assertEquals("concept", faq.topicType);
        assertEquals("- topic/topic concept/concept faq/faq", faq.ditaClass);
        assertTrue(faq.toJson().contains("\"ditaClass\":\"- topic/topic concept/concept faq/faq\""), faq.toJson());
        // No @class (a hand-written fixture): nothing to record.
        assertNull(findTopic(nodes, "plain").ditaClass);
    }

    @Test
    void copyToCopyIsTheTocEntryAndDerivesFromItsSource(@TempDir Path tempDir) throws Exception {
        // Shaped after DITA-OT 4.4.1's kept temp directory for a copy-to
        // fixture: the resolved topicref keeps href on the source and
        // copy-to on the copy; job.xml lists the copy as its own file
        // with the source's src, here *before* the source (the order the
        // Oxygen XML user guide produced, where the copy took the id).
        String src = "file:/src/topics/shared.dita";
        write(tempDir, ".job.xml", "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/chapter-one.dita\" path=\"topics/chapter-one.dita\" format=\"dita\"></file>"
                + "<file src=\"file:/src/topics/chapter-two.dita\" path=\"topics/chapter-two.dita\" format=\"dita\"></file>"
                + "<file src=\"" + src + "\" path=\"topics\\shared-2.dita\" format=\"dita\"></file>"
                + "<file src=\"" + src + "\" path=\"topics\\shared.dita\" format=\"dita\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>");
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/chapter-one.dita\"><topicref href=\"topics/shared.dita\"/></topicref>"
                + "<topicref href=\"topics/chapter-two.dita\">"
                + "<topicref href=\"topics/shared.dita\" copy-to=\"topics/shared-2.dita\"/></topicref>"
                + "</map>");
        write(tempDir, "topics/chapter-one.dita", topicXml("chapter-one", "Chapter one"));
        write(tempDir, "topics/chapter-two.dita", topicXml("chapter-two", "Chapter two"));
        String shared = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"shared\" xtrf=\"" + src + "\" xtrc=\"topic:1;3:30\">"
                + "<title xtrf=\"" + src + "\" xtrc=\"title:1;3:38\">Shared</title>"
                + "<body xtrf=\"" + src + "\" xtrc=\"body:1;3:59\">"
                + "<p id=\"shared-p\" xtrf=\"" + src + "\" xtrc=\"p:1;3:68\">Shared content.</p></body></topic>";
        write(tempDir, "topics/shared.dita", shared);
        write(tempDir, "topics/shared-2.dita", shared);

        List<String> warnings = new ArrayList<>();
        List<String> infos = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, infos::add).extract();
        assertTrue(warnings.isEmpty(), "a copy is an expected duplicate, not a warning: " + warnings);
        assertTrue(infos.stream().anyMatch(m -> m.contains("DITA2GRAPH071I")), "infos: " + infos);

        // The source keeps its id even though job.xml lists the copy first.
        TopicNode source = findTopic(nodes, "shared");
        assertEquals("topics/shared.dita", source.sourceFile);
        assertNull(source.authoredFile);
        TopicNode copy = findTopic(nodes, "shared--topics-shared-2.dita");
        // The copy keeps its own path, its concept's identity; the file the
        // author edits comes beside it, as the copy-to target exists only
        // in DITA-OT's output.
        assertEquals("topics/shared-2.dita", copy.sourceFile);
        assertEquals("topics/shared.dita", copy.authoredFile);

        // Each TOC entry contains its own file: the source once, the copy once.
        assertEquals(List.of("shared"), findTopic(nodes, "chapter-one").links.stream()
                .filter(l -> l.relation.equals("contains")).map(l -> l.target).toList());
        assertEquals(List.of("shared--topics-shared-2.dita"), findTopic(nodes, "chapter-two").links.stream()
                .filter(l -> l.relation.equals("contains")).map(l -> l.target).toList());

        // The copy derives from the source, never the other way round.
        assertTrue(copy.links.stream().anyMatch(l -> l.relation.equals("generated-from") && l.target.equals("shared")),
                "copy: " + copy.links);
        assertTrue(source.links.stream().noneMatch(l -> l.relation.equals("generated-from")),
                "source: " + source.links);
        assertTrue(copy.links.stream().anyMatch(l -> l.relation.equals("copy-of") && l.target.equals("shared")),
                "copy: " + copy.links);
        assertTrue(source.links.stream().noneMatch(l -> l.relation.equals("copy-of")), "source: " + source.links);
    }

    @Test
    void metadataComesFromTheResolvedPrologAndIndexTermsFromTheWholeTopic(@TempDir Path tempDir) throws Exception {
        // Shaped after DITA-OT 4.4.1's resolved output for a map whose
        // topicmeta and topicref topicmeta carry metadata: DITA-OT has
        // already cascaded them into topic a's prolog (the map's critdates
        // and prodinfo, the topicref's audience, permissions and keyword).
        write(tempDir, ".job.xml", "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/guide.ditamap\" path=\"guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "<file src=\"file:/src/topics/a.dita\" path=\"topics/a.dita\" format=\"dita\"></file>"
                + "<file src=\"file:/src/topics/b.dita\" path=\"topics/b.dita\" format=\"dita\"></file>"
                + "</files></job>");
        String prodinfo = "<prodinfo class=\"- topic/prodinfo \"><prodname class=\"- topic/prodname \">Widget Pro</prodname>"
                + "<vrmlist class=\"- topic/vrmlist \"><vrm class=\"- topic/vrm \" version=\"4\" release=\"2\"/></vrmlist></prodinfo>";
        write(tempDir, "guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"guide\" class=\"- map/map \"><title>Guide</title>"
                + "<topicmeta class=\"- map/topicmeta \"><critdates class=\"- topic/critdates \">"
                + "<created class=\"- topic/created \" date=\"2024-01-15\"/></critdates>" + prodinfo + "</topicmeta>"
                + "<topicref class=\"- map/topicref \" href=\"topics/a.dita\"/>"
                + "<topicref class=\"- map/topicref \" href=\"topics/b.dita\"/></map>");
        write(tempDir, "topics/a.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<task id=\"a\" class=\"- topic/topic task/task \"><title class=\"- topic/title \">Install</title>"
                + "<prolog class=\"- topic/prolog \">"
                + "<author class=\"- topic/author \" type=\"creator\">Ada</author>"
                + "<critdates class=\"- topic/critdates \"><created class=\"- topic/created \" date=\"2024-01-15\"/>"
                + "<revised class=\"- topic/revised \" modified=\"2025-01-01\" golive=\"2025-01-10\"/>"
                + "<revised class=\"- topic/revised \" modified=\"2025-06-01\" golive=\"2025-06-15\" expiry=\"2027-01-01\"/>"
                + "</critdates><permissions class=\"- topic/permissions \" view=\"internal\"/>"
                + "<metadata class=\"- topic/metadata \">"
                + "<audience class=\"- topic/audience \" type=\"other\" othertype=\"integrator\" job=\"installing\" experiencelevel=\"expert\"/>"
                + "<keywords class=\"- topic/keywords \"><keyword class=\"- topic/keyword \">from-map</keyword></keywords>"
                + "<keywords class=\"- topic/keywords \"><keyword class=\"- topic/keyword \">setup</keyword>"
                + "<cmdname class=\"+ topic/keyword sw-d/cmdname \">dita</cmdname>"
                + "<indexterm class=\"- topic/indexterm \">installing"
                + "<indexterm class=\"- topic/indexterm \">on <xmlelement class=\"+ topic/keyword markup-d/xmlelement \">Linux</xmlelement>"
                + "<index-sort-as class=\"+ topic/index-base indexing-d/index-sort-as \">linux</index-sort-as></indexterm>"
                + "</indexterm>"
                + "<indexterm class=\"- topic/indexterm \">setup<index-see class=\"+ topic/index-base indexing-d/index-see \">installing</index-see></indexterm>"
                + "</keywords>" + prodinfo + "</metadata></prolog>"
                + "<taskbody class=\"- topic/body task/taskbody \"><p class=\"- topic/p \">Run it."
                + "<indexterm class=\"- topic/indexterm \">commands<indexterm class=\"- topic/indexterm \">run</indexterm></indexterm>"
                + "</p></taskbody></task>");
        write(tempDir, "topics/b.dita", topicXml("b", "About"));

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, w -> { }, i -> { }).extract();

        Metadata a = findTopic(nodes, "a").metadata;
        assertEquals("2024-01-15", a.created);
        assertEquals("2025-06-01", a.modified, "the latest revision");
        assertEquals("2025-06-15", a.golive);
        assertEquals("2027-01-01", a.expiry);
        assertEquals("internal", a.permissions);
        assertEquals(List.of("Ada|creator"), a.authors.stream().map(x -> x[0] + "|" + x[1]).toList());
        assertEquals(List.of("integrator|installing|expert"),
                a.audiences.stream().map(x -> String.join("|", x)).toList(), "type=other reads othertype");
        assertEquals(List.of("from-map", "setup", "dita"), List.copyOf(a.keywords), "a keyword specialization counts");
        assertEquals(List.of("Widget Pro|4.2"), a.products.stream().map(x -> x[0] + "|" + x[1]).toList());
        // Each entry from its top level; a see reference and a sort key are
        // not words of the term.
        assertEquals(List.of(List.of("installing", "on Linux"), List.of("setup"), List.of("commands", "run")),
                List.copyOf(a.indexTerms));
        assertTrue(findTopic(nodes, "a").toJson().contains(
                "\"indexTerms\":[[\"installing\",\"on Linux\"],[\"setup\"],[\"commands\",\"run\"]]"));

        assertFalse(findTopic(nodes, "b").toJson().contains("metadata"), "no metadata, no field");
        MapNode map = (MapNode) nodes.get(0);
        assertEquals("2024-01-15", map.metadata.created);
        assertTrue(map.toJson().contains("\"products\":[{\"name\":\"Widget Pro\",\"version\":\"4.2\"}]"), map.toJson());
    }

    @Test
    void branchFilteringCopiesKeepTheFileTheAuthorEdits(@TempDir Path tempDir) throws Exception {
        // Shaped after DITA-OT 4.4.1's kept temp directory for the DITA-OT
        // documentation: two <ditavalref>s with a <dvrResourcePrefix> write
        // topics/first-build-using-dita-command.dita and
        // topics/build-using-dita-command.dita, each its own job.xml file
        // with the src of topics/using-dita-command.dita, which the map
        // also references unfiltered. The last topic is reached only
        // through a filtered copy, and its authored name has a space,
        // percent-encoded in src.
        String base = "file:/C:/docs%20root/";
        String src = base + "topics/using-dita-command.dita";
        String spaced = base + "topics/my%20topic.dita";
        write(tempDir, ".job.xml", "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"" + base + "guide.ditamap\" path=\"guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "<file src=\"" + src + "\" path=\"topics\\using-dita-command.dita\" format=\"dita\"></file>"
                + "<file src=\"" + src + "\" path=\"topics\\first-build-using-dita-command.dita\" format=\"dita\"></file>"
                + "<file src=\"" + src + "\" path=\"topics\\build-using-dita-command.dita\" format=\"dita\"></file>"
                + "<file src=\"" + spaced + "\" path=\"topics\\novice-topic.dita\" format=\"dita\"></file>"
                + "</files></job>");
        write(tempDir, "guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"guide\"><title>Guide</title>"
                + "<topicref href=\"topics/using-dita-command.dita\"/>"
                + "<topicref href=\"topics/first-build-using-dita-command.dita\"/>"
                + "<topicref href=\"topics/build-using-dita-command.dita\"/>"
                + "<topicref href=\"topics/novice-topic.dita\"/>"
                + "</map>");
        String command = topicXml("using-command-line-tool", "Using the dita command");
        write(tempDir, "topics/using-dita-command.dita", command);
        write(tempDir, "topics/first-build-using-dita-command.dita", command);
        write(tempDir, "topics/build-using-dita-command.dita", command);
        write(tempDir, "topics/novice-topic.dita", topicXml("my-topic", "My topic"));

        List<String> warnings = new ArrayList<>();
        List<String> infos = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, infos::add).extract();

        // Expected copies, not topics that happen to share an id.
        assertTrue(warnings.stream().noneMatch(w -> w.startsWith("DITA2GRAPH070W")), "warnings: " + warnings);
        assertEquals(2, infos.stream().filter(i -> i.startsWith("DITA2GRAPH071I")).count(), "infos: " + infos);

        TopicNode unfiltered = findTopic(nodes, "using-command-line-tool");
        assertEquals("topics/using-dita-command.dita", unfiltered.sourceFile);
        assertNull(unfiltered.authoredFile, "not a copy");
        TopicNode novice = findTopic(nodes, "using-command-line-tool--topics-first-build-using-dita-command.dita");
        assertEquals("topics/first-build-using-dita-command.dita", novice.sourceFile);
        assertEquals("topics/using-dita-command.dita", novice.authoredFile);
        TopicNode expert = findTopic(nodes, "using-command-line-tool--topics-build-using-dita-command.dita");
        assertEquals("topics/build-using-dita-command.dita", expert.sourceFile);
        assertEquals("topics/using-dita-command.dita", expert.authoredFile);
        TopicNode myTopic = findTopic(nodes, "my-topic");
        assertEquals("topics/novice-topic.dita", myTopic.sourceFile);
        assertEquals("topics/my topic.dita", myTopic.authoredFile, "src is percent-decoded");
        assertTrue(expert.toJson().contains("\"authoredFile\":\"topics/using-dita-command.dita\""), expert.toJson());
        assertFalse(unfiltered.toJson().contains("authoredFile"), unfiltered.toJson());

        // Each filtered copy is a copy of the unfiltered topic; a topic the
        // map publishes only filtered has no source to be a copy of.
        for (TopicNode copy : List.of(novice, expert)) {
            assertEquals(List.of("using-command-line-tool"), copy.links.stream()
                    .filter(l -> l.relation.equals("copy-of")).map(l -> l.target).toList(), copy.id);
        }
        assertTrue(myTopic.links.stream().noneMatch(l -> l.relation.equals("copy-of")), "my-topic: " + myTopic.links);
        assertTrue(unfiltered.links.stream().noneMatch(l -> l.relation.equals("copy-of")));
    }

    @Test
    void keyDefinitionsExternalLinksAndPrerequisitesFollowTheMarkup(@TempDir Path tempDir) throws Exception {
        // Shaped after dita-ot/docs' resolved userguide.ditamap (DITA-OT
        // 4.4.1, kept temp directory): every keydef of a submap survives,
        // with @keys, its resolved href and the submap's xtrf.
        String keysSrc = "file:/src/resources/keys.ditamap";
        write(tempDir, ".job.xml", "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"file:/src/topics/install.dita\" path=\"topics/install.dita\" format=\"dita\"></file>"
                + "<file src=\"file:/src/topics/setup.dita\" path=\"topics/setup.dita\" format=\"dita\"></file>"
                + "<file src=\"file:/src/topics/about.dita\" path=\"topics/about.dita\" format=\"dita\"></file>"
                + "<file src=\"" + keysSrc + "\" path=\"resources/keys.ditamap\" format=\"ditamap\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>");
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\" class=\"- map/map \"><title>User Guide</title>"
                + "<topicref class=\"- map/topicref \" href=\"topics/install.dita\" keys=\"install\"/>"
                + "<topicref class=\"- map/topicref \" href=\"topics/setup.dita\" keys=\"setup setup-alias\"/>"
                + "<topicref class=\"- map/topicref \" href=\"topics/about.dita\"/>"
                + "<topicgroup class=\"+ map/topicref mapgroup-d/topicgroup \" xtrf=\"" + keysSrc + "\">"
                + "<keydef class=\"+ map/topicref mapgroup-d/keydef \" keys=\"brew\" href=\"https://brew.sh\" "
                + "scope=\"external\" format=\"html\" processing-role=\"resource-only\" xtrf=\"" + keysSrc + "\">"
                + "<topicmeta class=\"- map/topicmeta \"><linktext class=\"- map/linktext \">Homebrew</linktext></topicmeta></keydef>"
                + "<keydef class=\"+ map/topicref mapgroup-d/keydef \" keys=\"product\" processing-role=\"resource-only\" "
                + "xtrf=\"" + keysSrc + "\"><topicmeta class=\"- map/topicmeta \"><keywords class=\"- topic/keywords \">"
                + "<keyword class=\"- topic/keyword \">Acme   Builder</keyword></keywords></topicmeta></keydef>"
                + "<keydef class=\"+ map/topicref mapgroup-d/keydef \" keys=\"logo\" href=\"images/logo.png\" format=\"png\" "
                + "processing-role=\"resource-only\" xtrf=\"" + keysSrc + "\"/>"
                // A second definition of "install": the first one wins.
                + "<keydef class=\"+ map/topicref mapgroup-d/keydef \" keys=\"install\" href=\"topics/about.dita\" "
                + "processing-role=\"resource-only\" xtrf=\"" + keysSrc + "\"/>"
                + "</topicgroup>"
                + "<topicgroup class=\"+ map/topicref mapgroup-d/topicgroup \" keyscope=\"legacy\">"
                + "<keydef class=\"+ map/topicref mapgroup-d/keydef \" keys=\"install\" href=\"topics/about.dita\" "
                + "processing-role=\"resource-only\"/></topicgroup>"
                + "</map>");
        write(tempDir, "topics/install.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<task id=\"install\" class=\"- topic/topic task/task \"><title>Install</title>"
                + "<taskbody class=\"- topic/body task/taskbody \">"
                + "<prereq class=\"- topic/section task/prereq \"><p>Install <xref href=\"https://brew.sh\" keyref=\"brew\" "
                + "scope=\"external\" format=\"html\">Homebrew</xref> and read <xref href=\"setup.dita\">Setup</xref>.</p></prereq>"
                + "<context class=\"- topic/section task/context \"><p><keyword keyref=\"product\">Acme Builder</keyword> "
                + "is described in <xref href=\"about.dita\" keyref=\"about\">About</xref>.</p></context>"
                + "</taskbody>"
                + "<related-links class=\"- topic/related-links \">"
                + "<link href=\"about.dita\" importance=\"required\" class=\"- topic/link \"/>"
                + "<link href=\"setup.dita\" role=\"parent\" class=\"- topic/link \" keyref=\"setup\"/>"
                + "</related-links></task>");
        write(tempDir, "topics/setup.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"setup\" class=\"- topic/topic concept/concept \"><title>Setup</title>"
                + "<conbody class=\"- topic/body concept/conbody \">"
                + "<p>A concept has no prerequisites: <xref href=\"install.dita\" importance=\"required\">Install</xref>.</p>"
                + "</conbody></concept>");
        write(tempDir, "topics/about.dita", topicXml("about", "About"));

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();
        assertTrue(warnings.isEmpty(), "warnings: " + warnings);

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(List.of("install", "setup", "setup-alias", "brew", "product", "logo", "install"),
                map.keyDefs.stream().map(k -> k.name).toList());
        KeyDef install = map.keyDefs.get(0);
        assertEquals("install", install.target, "the first definition of install wins");
        assertEquals("user-guide.ditamap", install.sourceFile);
        assertNull(install.text, "a key that names a topic keeps no text");
        assertEquals("setup", map.keyDefs.get(2).target);
        KeyDef brew = map.keyDefs.get(3);
        assertEquals("https://brew.sh", brew.url);
        assertEquals("Homebrew", brew.text);
        assertEquals("resources/keys.ditamap", brew.sourceFile, "the submap that defines it, from xtrf");
        KeyDef product = map.keyDefs.get(4);
        assertNull(product.target);
        assertNull(product.url);
        assertEquals("Acme Builder", product.text);
        assertEquals("images/logo.png", map.keyDefs.get(5).file);
        KeyDef scoped = map.keyDefs.get(6);
        assertEquals("legacy", scoped.scope);
        assertEquals("about", scoped.target);
        assertTrue(map.toJson().contains("{\"name\":\"brew\",\"url\":\"https://brew.sh\",\"text\":\"Homebrew\","
                + "\"sourceFile\":\"resources/keys.ditamap\"}"), map.toJson());

        TopicNode task = findTopic(nodes, "install");
        // prereq link and a task's required link: requires; the rest references.
        assertEquals(List.of("requires->setup", "requires->about"),
                task.links.stream().map(l -> l.relation + "->" + l.target).toList());
        // The parent link is generated navigation: its keyref is not a use.
        assertEquals(Set.of("brew", "product", "about"), task.keyrefs);
        assertEquals(Map.of("https://brew.sh", "brew"), task.externalLinks);
        assertTrue(task.toJson().contains("\"externalLinks\":[{\"url\":\"https://brew.sh\",\"key\":\"brew\"}]"),
                task.toJson());
        // importance="required" outside a task is no prerequisite.
        assertEquals(List.of("references->install"), findTopic(nodes, "setup").links.stream()
                .map(l -> l.relation + "->" + l.target).toList());
    }

    @Test
    void coderefAndConkeyrefAreReadFromTheSourceWhenTheResolvedTopicConfirmsThem(@TempDir Path tempDir)
            throws Exception {
        // DITA-OT replaces <coderef> with the file's text and resolves
        // conkeyref away (dita-ot/docs, kept temp directory): job.xml lists
        // the sample files (format "coderef") but not who includes them.
        Path srcDir = tempDir.resolve("src");
        String sampleSrc = srcDir.resolve("topics/sample.dita").toFile().toURI().toString();
        String warehouseSrc = srcDir.resolve("topics/warehouse.dita").toFile().toURI().toString();
        String brokenSrc = srcDir.resolve("topics/broken.dita").toFile().toURI().toString();
        String buildSrc = srcDir.resolve("samples/build.xml").toFile().toURI().toString();
        String filteredSrc = srcDir.resolve("samples/filtered.xml").toFile().toURI().toString();
        Path temp = tempDir.resolve("temp");
        write(temp, ".job.xml", "<?xml version=\"1.0\" ?><job><files>"
                + "<file src=\"" + sampleSrc + "\" path=\"topics/sample.dita\" format=\"dita\"></file>"
                + "<file src=\"" + warehouseSrc + "\" path=\"topics/warehouse.dita\" format=\"dita\"></file>"
                + "<file src=\"" + brokenSrc + "\" path=\"topics/broken.dita\" format=\"dita\"></file>"
                + "<file src=\"" + buildSrc + "\" path=\"samples/build.xml\" format=\"coderef\"></file>"
                + "<file src=\"" + filteredSrc + "\" path=\"samples/filtered.xml\" format=\"coderef\"></file>"
                + "<file src=\"file:/src/user-guide.ditamap\" path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
                + "</files></job>");
        write(temp, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/sample.dita\"/><topicref href=\"topics/broken.dita\"/>"
                + "<keydef keys=\"warehouse\" href=\"topics/warehouse.dita\" processing-role=\"resource-only\"/>"
                + "<keydef keys=\"other\" href=\"topics/broken.dita\" processing-role=\"resource-only\"/>"
                + "</map>");
        // The source: two coderefs (DITAVAL removed the second codeblock)
        // and two conkeyrefs (only the first one's content got reused).
        write(srcDir, "topics/sample.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"sample\"><title>Sample</title><body>"
                + "<codeblock><coderef href=\"../samples/build.xml#line-range(1,5)\"/></codeblock>"
                + "<codeblock audience=\"internal\"><coderef href=\"../samples/filtered.xml\"/></codeblock>"
                + "<p conkeyref=\"warehouse/shared-p\"/><p conkeyref=\"other/gone\"/>"
                + "</body></topic>");
        write(srcDir, "topics/broken.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"broken\"><title>Broken</title><body><p>&undefined;<coderef href=\"x\"/></p></body></topic>");
        write(temp, "topics/sample.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"sample\" xtrf=\"" + sampleSrc + "\" xtrc=\"topic:1;1:1\"><title>Sample</title>"
                + "<body xtrf=\"" + sampleSrc + "\" xtrc=\"body:1;1:30\">"
                + "<codeblock xtrf=\"" + sampleSrc + "\" xtrc=\"codeblock:1;1:36\">&lt;project/&gt;</codeblock>"
                + "<p xtrf=\"" + warehouseSrc + "\" xtrc=\"p:1;1:60\">Shared text.</p>"
                + "</body></topic>");
        write(temp, "topics/warehouse.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"warehouse\" xtrf=\"" + warehouseSrc + "\" xtrc=\"topic:1;1:1\"><title>Warehouse</title>"
                + "<body xtrf=\"" + warehouseSrc + "\" xtrc=\"body:1;1:30\">"
                + "<p id=\"shared-p\" xtrf=\"" + warehouseSrc + "\" xtrc=\"p:1;1:60\">Shared text.</p></body></topic>");
        // A codeblock: the source is read only when a coderef could have
        // filled one (or the topic reuses content).
        write(temp, "topics/broken.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"broken\"><title>Broken</title><body><codeblock>x</codeblock></body></topic>");

        List<String> infos = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(temp.toFile(), false, msg -> { }, infos::add).extract();

        TopicNode sample = findTopic(nodes, "sample");
        assertEquals(Set.of("samples/build.xml"), sample.codeRefs);
        assertEquals(Set.of("warehouse"), sample.keyrefs, "the conkeyref whose content was reused");
        assertTrue(sample.toJson().contains("\"codeRefs\":[\"samples/build.xml\"]"), sample.toJson());
        assertTrue(findTopic(nodes, "broken").codeRefs.isEmpty());
        assertTrue(infos.stream().anyMatch(m -> m.startsWith("DITA2GRAPH072I") && m.contains("topics/broken.dita")),
                "infos: " + infos);
    }

    @Test
    void topicTypeComesFromClassSoSpecializationsAndTroubleshootingAreRecognized(@TempDir Path tempDir)
            throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/fix.dita", "topics/group.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/fix.dita\"/><topicref href=\"topics/group.dita\"/></map>");
        write(tempDir, "topics/fix.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<troubleshooting id=\"fix\" class=\"- topic/topic troubleshooting/troubleshooting \">"
                + "<title>Fix it</title><troublebody class=\"- topic/body troubleshooting/troublebody \">"
                + "<p>Restart the server.</p></troublebody></troubleshooting>");
        write(tempDir, "topics/group.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<glossgroup id=\"group\" class=\"- topic/topic concept/concept glossgroup/glossgroup \">"
                + "<title>Terms</title></glossgroup>");

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();

        TopicNode fix = findTopic(nodes, "fix");
        assertEquals("troubleshooting", fix.topicType);
        assertEquals("Restart the server.", fix.body);
        assertEquals("concept", findTopic(nodes, "group").topicType, "a specialization takes its base type");
        assertTrue(warnings.isEmpty(), "both types are recognized: " + warnings);
    }

    @Test
    void nestedTopicsOfACompositeFileJoinTheRootTopicsText(@TempDir Path tempDir) throws Exception {
        // Shaped after dita-ot/docs' release-notes/index.dita: a root
        // <reference> with no body, one nested <reference> per release.
        write(tempDir, ".job.xml", jobXml("release-notes/index.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"release-notes/index.dita\"/></map>");
        write(tempDir, "release-notes/index.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<reference id=\"notes\" class=\"- topic/topic reference/reference \">"
                + "<title class=\"- topic/title \">Release notes</title>"
                + "<reference id=\"v2\" class=\"- topic/topic reference/reference \">"
                + "<title class=\"- topic/title \">Release 2</title>"
                + "<refbody class=\"- topic/body reference/refbody \"><section class=\"- topic/section \">"
                + "<p class=\"- topic/p \">outputFile.base is deprecated.</p></section></refbody>"
                + "<reference id=\"v2-fixes\" class=\"- topic/topic reference/reference \">"
                + "<title class=\"- topic/title \">Fixes</title>"
                + "<refbody class=\"- topic/body reference/refbody \"><p class=\"- topic/p \">Bug 42.</p></refbody>"
                + "</reference></reference>"
                + "<reference id=\"v1\" class=\"- topic/topic reference/reference \">"
                + "<title class=\"- topic/title \">Release 1</title>"
                + "<shortdesc class=\"- topic/shortdesc \">The first one.</shortdesc>"
                + "<refbody class=\"- topic/body reference/refbody \"><p class=\"- topic/p \">Hello.</p></refbody>"
                + "</reference></reference>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        TopicNode notes = findTopic(nodes, "notes");
        assertEquals("Release notes", notes.title, "the root keeps its own title");
        assertEquals("Release 2 outputFile.base is deprecated. Fixes Bug 42. Release 1 The first one. Hello.",
                notes.body);
    }

    @Test
    void keyrefsResolveInTheKeyScopeOfTheTopicrefParentScopeFirst(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/old.dita", "topics/new.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<keydef keys=\"shared\"><topicmeta><keywords><keyword>Root</keyword></keywords></topicmeta></keydef>"
                + "<topicref href=\"topics/new.dita\"/>"
                + "<topicgroup keyscope=\"legacy\">"
                + "<keydef keys=\"product\"><topicmeta><keywords><keyword>Classic</keyword></keywords></topicmeta></keydef>"
                + "<keydef keys=\"shared\"><topicmeta><keywords><keyword>Legacy</keyword></keywords></topicmeta></keydef>"
                + "<topicref href=\"topics/old.dita\"/></topicgroup></map>");
        String keywords = "<keyword keyref=\"product\"/><keyword keyref=\"shared\"/><keyword keyref=\"legacy.product\"/>";
        write(tempDir, "topics/old.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"old\"><title>Old</title><body><p>" + keywords + "</p></body></topic>");
        write(tempDir, "topics/new.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"new\"><title>New</title><body><p>" + keywords + "</p></body></topic>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        // In the legacy scope, product is the scope's own key, and shared
        // is the root's: a parent scope's definition takes precedence.
        assertEquals(Set.of("legacy.product", "shared"), findTopic(nodes, "old").keyrefs);
        // In the root scope, a bare product is defined nowhere.
        assertEquals(Set.of("product", "shared", "legacy.product"), findTopic(nodes, "new").keyrefs);
    }

    @Test
    void aGlossentrysTextIsItsDefinitionAndItsGlossBody(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/api.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title><topicref href=\"topics/api.dita\"/></map>");
        write(tempDir, "topics/api.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<glossentry id=\"api\" class=\"- topic/topic concept/concept glossentry/glossentry \">"
                + "<glossterm class=\"- topic/title concept/title glossentry/glossterm \">API</glossterm>"
                + "<glossdef class=\"- topic/abstract concept/abstract glossentry/glossdef \">Application programming interface.</glossdef>"
                + "<glossBody class=\"- topic/body concept/conbody glossentry/glossBody \">"
                + "<glossUsage class=\"- topic/note concept/note glossentry/glossUsage \">Spell it out first.</glossUsage>"
                + "</glossBody></glossentry>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        assertEquals("Application programming interface. Spell it out first.", findTopic(nodes, "api").body);
    }

    @Test
    void nothingPointsAtASkippedDraft(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/a.dita", "topics/draft.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/a.dita\"/><topicref href=\"topics/draft.dita\" keys=\"wip\"/></map>");
        write(tempDir, "topics/a.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"a\"><title>A</title><body><p><xref href=\"draft.dita\">WIP</xref></p></body></topic>");
        write(tempDir, "topics/draft.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<topic id=\"draft\" status=\"draft\"><title>Draft</title><body><p>Soon.</p></body></topic>");

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(List.of("a"), map.links.stream().map(l -> l.target).toList());
        assertTrue(findTopic(nodes, "a").links.isEmpty(), "a: " + findTopic(nodes, "a").links);
        assertEquals("wip", map.keyDefs.get(0).name);
        assertNull(map.keyDefs.get(0).target, "the key's topic is not exported");
    }

    @Test
    void aKeydefIsResourceOnlyByDefaultEvenWithoutClass(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/a.dita", "topics/x.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/a.dita\"/>"
                + "<keydef keys=\"x\" href=\"topics/x.dita\"/>"
                + "<keydef keys=\"v\"><topicmeta><navtitle>Variable</navtitle></topicmeta></keydef></map>");
        write(tempDir, "topics/a.dita", topicXml("a", "A"));
        write(tempDir, "topics/x.dita", topicXml("x", "X"));

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, msg -> { }, msg -> { }).extract();

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(List.of("a"), map.links.stream().map(l -> l.target).toList(), "a keydef is no TOC entry");
        assertEquals("resource-only", findTopic(nodes, "x").processingRole);
        assertTrue(nodes.stream().noneMatch(n -> n instanceof TopicNode && "topichead".equals(((TopicNode) n).topicType)),
                "a text keydef is no heading");
        assertEquals(List.of("x", "v"), map.keyDefs.stream().map(k -> k.name).toList());
    }

    @Test
    void keysAreRecordedBeyondTheDepthLimitAndInRelationshipTables(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/a.dita", "topics/b.dita", "topics/c.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/a.dita\"><topicref href=\"topics/b.dita\" keys=\"deep\"/></topicref>"
                + "<reltable><relrow><relcell><topicref href=\"topics/c.dita\" keys=\"related\"/></relcell></relrow></reltable>"
                + "</map>");
        for (String id : List.of("a", "b", "c")) {
            write(tempDir, "topics/" + id + ".dita", topicXml(id, id.toUpperCase()));
        }

        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, 1, msg -> { }, msg -> { }).extract();

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(List.of("deep", "related"), map.keyDefs.stream().map(k -> k.name).toList());
        assertEquals(List.of("b", "c"), map.keyDefs.stream().map(k -> k.target).toList());
        assertTrue(findTopic(nodes, "a").links.isEmpty(), "depth 1: a contains nothing");
    }

    @Test
    void aRelationshipTableLinkThroughAKeyUsesThatKeyAndAGeneratedDescriptionAddsNothing(@TempDir Path tempDir)
            throws Exception {
        // DITA-OT 4.4.1 drops the reltable topicref's @keyref from the link
        // it generates, but keeps the topicref's xtrf/xtrc, and copies the
        // target's shortdesc, with the target's own xref and keyword, into
        // the link (dita-ot/docs: topics/ant.dita, key ant-manual).
        String reltable = "file:/src/reltable.ditamap";
        write(tempDir, ".job.xml", jobXml("topics/a.dita", "topics/b.dita", "topics/c.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title>"
                + "<topicref href=\"topics/a.dita\"/><topicref href=\"topics/b.dita\" keys=\"b-key\"/>"
                + "<topicref href=\"topics/c.dita\" keys=\"c-key\"/>"
                + "<keydef keys=\"talk\" href=\"https://example.com/talk\" scope=\"external\" format=\"html\"/>"
                + "<reltable><relrow>"
                + "<relcell><topicref href=\"topics/a.dita\" xtrf=\"" + reltable + "\" xtrc=\"topicref:1;5:10\"/></relcell>"
                + "<relcell>"
                + "<topicref keyref=\"talk\" href=\"https://example.com/talk\" scope=\"external\" format=\"html\""
                + " xtrf=\"" + reltable + "\" xtrc=\"topicref:2;6:10\"/>"
                + "<topicref keyref=\"b-key\" href=\"topics/b.dita\" xtrf=\"" + reltable + "\" xtrc=\"topicref:3;7:10\"/>"
                + "</relcell></relrow></reltable></map>");
        write(tempDir, "topics/a.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"a\"><title>A</title><conbody><p>A content.</p></conbody>"
                + "<related-links><linkpool xtrf=\"" + reltable + "\" xtrc=\"topicref:1;5:10\">"
                + "<link class=\"- topic/link \" href=\"https://example.com/talk\" role=\"friend\" scope=\"external\""
                + " format=\"html\" xtrf=\"" + reltable + "\" xtrc=\"topicref:2;6:10\">"
                + "<linktext class=\"- topic/linktext \">Talk</linktext></link>"
                + "<link class=\"- topic/link \" href=\"b.dita\" role=\"friend\" scope=\"local\" format=\"dita\""
                + " xtrf=\"" + reltable + "\" xtrc=\"topicref:3;7:10\">"
                + "<linktext class=\"- topic/linktext \">B</linktext>"
                + "<desc class=\"- topic/desc \">About B, see <xref class=\"- topic/xref \" href=\"c.dita\""
                + " keyref=\"c-key\">C</xref> and <keyword class=\"- topic/keyword \" keyref=\"product\"/>.</desc>"
                + "</link></linkpool></related-links></concept>");
        // b links the same URL in its prose, without the key; the reltable
        // link back to a went through no key.
        write(tempDir, "topics/b.dita", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<concept id=\"b\"><title>B</title><conbody><p>See <xref href=\"https://example.com/talk\""
                + " scope=\"external\" format=\"html\">the talk</xref>.</p></conbody>"
                + "<related-links><linkpool xtrf=\"" + reltable + "\" xtrc=\"topicref:3;7:10\">"
                + "<link class=\"- topic/link \" href=\"a.dita\" role=\"friend\" scope=\"local\" format=\"dita\""
                + " xtrf=\"" + reltable + "\" xtrc=\"topicref:1;5:10\"/>"
                + "</linkpool></related-links></concept>");
        write(tempDir, "topics/c.dita", topicXml("c", "C"));

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();
        assertTrue(warnings.isEmpty(), "warnings: " + warnings);

        TopicNode a = findTopic(nodes, "a");
        assertEquals(Map.of("https://example.com/talk", "talk"), a.externalLinks);
        assertEquals(Set.of("talk", "b-key"), a.keyrefs, "the description's c-key and product are b's");
        assertEquals(List.of("references->b"), a.links.stream().map(l -> l.relation + "->" + l.target).toList());

        TopicNode b = findTopic(nodes, "b");
        assertNull(b.externalLinks.get("https://example.com/talk"), "b hard-codes the URL");
        assertTrue(b.keyrefs.isEmpty(), "b: " + b.keyrefs);
        assertEquals(List.of("references->a"), b.links.stream().map(l -> l.relation + "->" + l.target).toList());
    }

    @Test
    void aTopicNamedLikeTheMapGetsAnIdOfItsOwn(@TempDir Path tempDir) throws Exception {
        write(tempDir, ".job.xml", jobXml("topics/guide.dita"));
        write(tempDir, "user-guide.ditamap", "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<map id=\"user-guide\"><title>User Guide</title><topicref href=\"topics/guide.dita\"/></map>");
        write(tempDir, "topics/guide.dita", topicXml("user-guide", "About this guide"));

        List<String> warnings = new ArrayList<>();
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, warnings::add, msg -> { }).extract();

        assertEquals("About this guide", findTopic(nodes, "user-guide--topics-guide.dita").title);
        assertTrue(warnings.stream().anyMatch(w -> w.startsWith("DITA2GRAPH070W")), "warnings: " + warnings);
    }

    private static String jobXml(String... topicPaths) {
        StringBuilder sb = new StringBuilder("<?xml version=\"1.0\" ?><job><files>");
        for (String path : topicPaths) {
            sb.append("<file src=\"file:/src/").append(path).append("\" uri=\"").append(path)
                    .append("\" path=\"").append(path).append("\" format=\"dita\" target=\"true\"></file>");
        }
        sb.append("<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
                + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>");
        return sb.append("</files></job>").toString();
    }

    private static TopicNode findTopic(List<Object> nodes, String id) {
        return nodes.stream()
                .filter(TopicNode.class::isInstance)
                .map(TopicNode.class::cast)
                .filter(t -> t.id.equals(id))
                .findFirst()
                .orElseThrow(() -> new AssertionError("no topic with id " + id));
    }

    private static void write(Path dir, String relative, String content) throws IOException {
        Path path = dir.resolve(relative);
        Files.createDirectories(path.getParent());
        Files.writeString(path, content);
    }

    private static final String JOB_XML = "<?xml version=\"1.0\" ?><job>"
            + "<files>"
            + "<file src=\"file:/src/topics/installing-product.dita\" uri=\"topics/installing-product.dita\" "
            + "path=\"topics/installing-product.dita\" format=\"dita\" has-keyref=\"true\" has-link=\"true\" target=\"true\"></file>"
            + "<file src=\"file:/src/topics/configuration.dita\" uri=\"topics/configuration.dita\" "
            + "path=\"topics/configuration.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/topics/installing-product-prereqs.dita\" uri=\"topics/installing-product-prereqs.dita\" "
            + "path=\"topics/installing-product-prereqs.dita\" format=\"dita\" has-keyref=\"true\" has-link=\"true\" target=\"true\"></file>"
            + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
            + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
            + "</files></job>";

    private static final String MAP_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<map id=\"user-guide\">"
            + "<title>User Guide</title>"
            + "<topicref href=\"topics/configuration.dita\" keys=\"config-concept\" type=\"concept\"/>"
            + "<topicref href=\"topics/installing-product.dita\" keys=\"install-task\" type=\"task\"/>"
            + "</map>";

    // Shaped after a live DITA-OT 4.4 run's actual resolved output for a
    // <navref> (confirmed directly, docs/dev/phase-0-findings.md finding
    // 16): it survives completely unresolved, verbatim, since DITA-OT
    // never processes it for this transtype.
    private static final String MAP_XML_WITH_NAVREF = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<map id=\"user-guide\">"
            + "<title>User Guide</title>"
            + "<topicref href=\"topics/configuration.dita\" keys=\"config-concept\" type=\"concept\"/>"
            + "<topicref href=\"topics/installing-product.dita\" keys=\"install-task\" type=\"task\"/>"
            + "<navref mapref=\"nav.ditamap\"/>"
            + "</map>";

    private static final String CONFIGURATION_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<concept id=\"configuration\">"
            + "<title>Configuration Overview</title>"
            + "<conbody><p>Configuration overview content goes here.</p></conbody>"
            + "</concept>";

    private static final String PREREQS_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<topic id=\"installing-product-prereqs\">"
            + "<title>Installing Product: Prerequisites</title>"
            + "<body><p>See <xref href=\"configuration.dita\" keyref=\"config-concept\" type=\"concept\">Configuration Overview</xref>.</p></body>"
            + "</topic>";

    private static final String INSTALLING_PRODUCT_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<task id=\"installing-product\" audience=\"admin\" product=\"enterprise\">"
            + "<title>Installing Product</title>"
            + "<shortdesc>Steps to install the product in a production environment.</shortdesc>"
            + "<taskbody>"
            + "<prereq><xref href=\"installing-product-prereqs.dita\" type=\"topic\"/></prereq>"
            + "<context><p>This task requires <xref href=\"configuration.dita\" keyref=\"config-concept\" type=\"concept\">Configuration Overview</xref> to be completed first.</p></context>"
            + "<steps><step><cmd>Download the installer.</cmd></step></steps>"
            + "</taskbody>"
            + "</task>";

    // Shaped after a real DITA-OT 4.4 resolved topic for a nested
    // topicref parent (captured directly from a live run against a
    // scratch project, not guessed): a <related-links> block with a
    // "child" role link to the nested topicref's target.
    private static final String CHAPTER_XML_WITH_RELATED_LINKS = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<topic id=\"chapter\"><title>Chapter</title>"
            + "<body><p>Chapter content.</p></body>"
            + "<related-links><linkpool mapkeyref=\"user-guide\"><linkpool>"
            + "<link format=\"dita\" href=\"section.dita\" role=\"child\" scope=\"local\" type=\"topic\">"
            + "<linktext>Section</linktext></link>"
            + "</linkpool></linkpool></related-links>"
            + "</topic>";

    // Same shape, "parent" role, for the nested child's own resolved output.
    private static final String SECTION_XML_WITH_RELATED_LINKS = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<topic id=\"section\"><title>Section</title>"
            + "<body><p>Section content.</p></body>"
            + "<related-links><linkpool mapkeyref=\"user-guide\">"
            + "<link format=\"dita\" href=\"chapter.dita\" role=\"parent\" scope=\"local\" type=\"topic\">"
            + "<linktext>Chapter</linktext></link>"
            + "</linkpool></related-links>"
            + "</topic>";

    private static final String JOB_XML_NESTED = "<?xml version=\"1.0\" ?><job><files>"
            + "<file src=\"file:/src/topics/chapter.dita\" uri=\"topics/chapter.dita\" "
            + "path=\"topics/chapter.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/topics/section.dita\" uri=\"topics/section.dita\" "
            + "path=\"topics/section.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/topics/grouped.dita\" uri=\"topics/grouped.dita\" "
            + "path=\"topics/grouped.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/topics/another.dita\" uri=\"topics/another.dita\" "
            + "path=\"topics/another.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
            + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
            + "</files></job>";

    // A top-level <topicref> ("chapter") with a nested child <topicref>
    // ("section") -- section should attach to chapter, not the map. A
    // <topichead> and a <topicgroup>, each wrapping one <topicref> with
    // no topic of their own -- the titled topichead becomes a heading
    // node, the topicgroup skips through to the map.
    private static final String MAP_XML_NESTED = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<map id=\"user-guide\"><title>User Guide</title>"
            + "<topicref href=\"topics/chapter.dita\">"
            + "<topicref href=\"topics/section.dita\"/>"
            + "</topicref>"
            + "<topichead navtitle=\"Group\">"
            + "<topicref href=\"topics/grouped.dita\"/>"
            + "</topichead>"
            + "<topicgroup>"
            + "<topicref href=\"topics/another.dita\"/>"
            + "</topicgroup>"
            + "</map>";

    private static final String JOB_XML_DRAFT_ONLY = "<?xml version=\"1.0\" ?><job><files>"
            + "<file src=\"file:/src/topics/configuration.dita\" uri=\"topics/configuration.dita\" "
            + "path=\"topics/configuration.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
            + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
            + "</files></job>";

    private static final String MAP_XML_DRAFT_ONLY = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<map id=\"user-guide\"><title>User Guide</title>"
            + "<topicref href=\"topics/configuration.dita\"/></map>";

    private static final String DRAFT_CONFIGURATION_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<concept id=\"configuration\" status=\"draft\">"
            + "<title>Configuration Overview</title><conbody><p>Draft content.</p></conbody></concept>";

    private static final String JOB_XML_GENERATED_FROM = "<?xml version=\"1.0\" ?><job><files>"
            + "<file src=\"file:/src/topics/source.dita\" uri=\"topics/source.dita\" "
            + "path=\"topics/source.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/topics/reuser.dita\" uri=\"topics/reuser.dita\" "
            + "path=\"topics/reuser.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
            + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
            + "</files></job>";

    private static final String MAP_XML_GENERATED_FROM = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<map id=\"user-guide\"><title>User Guide</title>"
            + "<topicref href=\"topics/source.dita\"/>"
            + "<topicref href=\"topics/reuser.dita\"/>"
            + "</map>";

    // source.dita's own paragraph carries an xtrf matching its own src --
    // real DITA-OT output tags every element this way, reused or not
    // (confirmed directly, docs/dev/phase-0-findings.md finding 15).
    private static final String SOURCE_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<concept id=\"source\">"
            + "<title>Source Topic</title>"
            + "<conbody><p xtrf=\"file:/src/topics/source.dita\">Reusable content.</p></conbody>"
            + "</concept>";

    // Two elements carry source.dita's xtrf (simulating two separate
    // conref/conkeyref pulls from the same source topic) -- must
    // collapse to exactly one generated-from edge, not two. A third
    // element carries reuser's own xtrf (its own, non-reused content)
    // and must not produce a self-referential edge.
    private static final String REUSER_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<concept id=\"reuser\">"
            + "<title>Reuser Topic</title>"
            + "<conbody>"
            + "<p xtrf=\"file:/src/topics/source.dita\">This paragraph is reused elsewhere via conref.</p>"
            + "<p xtrf=\"file:/src/topics/source.dita\">Another reused paragraph via conkeyref.</p>"
            + "<p xtrf=\"file:/src/topics/reuser.dita\">Own content not reused.</p>"
            + "</conbody>"
            + "</concept>";

    private static final String JOB_XML_UICONTROL = "<?xml version=\"1.0\" ?><job><files>"
            + "<file src=\"file:/src/topics/save-task.dita\" uri=\"topics/save-task.dita\" "
            + "path=\"topics/save-task.dita\" format=\"dita\" target=\"true\"></file>"
            + "<file src=\"file:/src/user-guide.ditamap\" uri=\"user-guide.ditamap\" "
            + "path=\"user-guide.ditamap\" format=\"ditamap\" input=\"true\"></file>"
            + "</files></job>";

    private static final String MAP_XML_UICONTROL = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<map id=\"user-guide\"><title>User Guide</title>"
            + "<topicref href=\"topics/save-task.dita\"/></map>";

    private static final String SAVE_TASK_XML = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
            + "<task id=\"save-task\">"
            + "<title>Save Task</title>"
            + "<taskbody>"
            + "<context><p>See <uicontrol>Mentioned Elsewhere</uicontrol> in the toolbar.</p></context>"
            + "<steps><step><cmd>Click <uicontrol>Save</uicontrol> to store your changes.</cmd></step></steps>"
            + "</taskbody>"
            + "</task>";
}
