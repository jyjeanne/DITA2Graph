package org.dita.dita2graph.tasks;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
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
        // The keyref'd cross-reference in <context> resolves to "requires";
        // the bare xref in <prereq> (no keyref) resolves to "references".
        assertTrue(installingProduct.links.stream()
                .anyMatch(l -> l.relation.equals("requires") && l.target.equals("configuration")));
        assertTrue(installingProduct.links.stream()
                .anyMatch(l -> l.relation.equals("references") && l.target.equals("installing-product-prereqs")));

        TopicNode configuration = findTopic(nodes, "configuration");
        assertEquals("concept", configuration.topicType);
        assertEquals(List.of("config-concept"), configuration.keys);
        assertNull(configuration.shortdesc);
        assertEquals("Configuration overview content goes here.", configuration.body);

        TopicNode prereqs = findTopic(nodes, "installing-product-prereqs");
        assertEquals("topic", prereqs.topicType);
        assertTrue(prereqs.links.stream()
                .anyMatch(l -> l.relation.equals("requires") && l.target.equals("configuration")));
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
        assertEquals(5, nodes.size(), "map + 4 topics");

        MapNode map = (MapNode) nodes.get(0);
        // "chapter" is a top-level topicref -> map contains it directly.
        // "grouped" is nested inside a <topichead> (no topic of its own)
        // and "another" inside a <topicgroup> -- both skip through to
        // the map, same as if the wrapper weren't there. "section" is
        // NOT here: it's nested *inside* the "chapter" topicref, so it
        // belongs to chapter, not the map.
        assertEquals(3, map.links.size(), "map: " + map.links);
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("chapter")));
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("grouped")));
        assertTrue(map.links.stream().anyMatch(l -> l.relation.equals("contains") && l.target.equals("another")));

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

        // depth=1: top-level topicrefs (chapter, grouped, another --
        // topichead/topicgroup don't consume a level) are still
        // captured, but "section" (nested one level inside "chapter",
        // i.e. level 2) is not -- its topic node still exists (Pass 2
        // extracts every resolved topic regardless), it just has no
        // incoming contains edge.
        List<Object> nodes = new DitaModelExtractor(tempDir.toFile(), false, 1, msg -> { }, msg -> { }).extract();
        assertEquals(5, nodes.size(), "the topic itself is still extracted, just not contained");

        MapNode map = (MapNode) nodes.get(0);
        assertEquals(3, map.links.size(), "map: " + map.links);
        assertTrue(map.links.stream().anyMatch(l -> l.target.equals("chapter")));
        assertTrue(map.links.stream().anyMatch(l -> l.target.equals("grouped")));
        assertTrue(map.links.stream().anyMatch(l -> l.target.equals("another")));

        TopicNode chapterAtDepth1 = findTopic(nodes, "chapter");
        assertTrue(chapterAtDepth1.links.isEmpty(),
                "section is one level too deep for depth=1: " + chapterAtDepth1.links);

        // depth=2 (or "unlimited", the default) restores the nested edge.
        List<Object> unlimitedNodes =
                new DitaModelExtractor(tempDir.toFile(), false, 2, msg -> { }, msg -> { }).extract();
        TopicNode chapterAtDepth2 = findTopic(unlimitedNodes, "chapter");
        assertTrue(chapterAtDepth2.links.stream().anyMatch(l -> l.target.equals("section")));
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
        long requiresToConfig = reuser.links.stream()
                .filter(l -> l.relation.equals("requires") && l.target.equals("configuration"))
                .count();
        long referencesToPrereqs = reuser.links.stream()
                .filter(l -> l.relation.equals("references") && l.target.equals("installing-product-prereqs"))
                .count();
        assertEquals(1, requiresToConfig,
                "two keyref'd mentions of the same target should collapse to one requires edge: " + reuser.links);
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
    // no topic of their own -- both should skip through to the map.
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
