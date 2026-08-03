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

        TopicNode prereqs = findTopic(nodes, "installing-product-prereqs");
        assertEquals("topic", prereqs.topicType);
        assertTrue(prereqs.links.stream()
                .anyMatch(l -> l.relation.equals("requires") && l.target.equals("configuration")));
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
}
