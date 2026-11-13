package io.github.jyjeanne.dita2graph.tasks;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.stream.Collectors;

import static org.junit.jupiter.api.Assertions.assertEquals;

/**
 * The normalized model as this plugin hands it to dita2graph-core, with
 * every field of every node kind set, against
 * {@code core/dita2graph-core/tests/data/java-model.json}. The core's
 * {@code tests/java_model.rs} reads that file back into its own model and
 * writes it out again: a field one side names differently, or the Rust
 * model drops, fails there.
 *
 * <p>After an intended change, regenerate the file with
 * {@code UPDATE_GOLDEN=1 ./gradlew test}, then run the core's test.
 */
class ModelJsonContractTest {

    private static final Path GOLDEN = Path.of("../../../core/dita2graph-core/tests/data/java-model.json");

    @Test
    void theModelMatchesTheFileTheCoreReads() throws IOException {
        List<Object> nodes = model();
        // One node per line, so a diff of the file shows which one changed.
        String actual = nodes.stream()
                .map(node -> node instanceof MapNode map ? map.toJson() : ((TopicNode) node).toJson())
                .collect(Collectors.joining(",\n", "[\n", "\n]\n"));

        if (System.getenv("UPDATE_GOLDEN") != null) {
            Files.writeString(GOLDEN, actual, StandardCharsets.UTF_8);
        }
        // A Windows checkout may have turned the file's LF into CRLF.
        String expected = Files.readString(GOLDEN, StandardCharsets.UTF_8).replace("\r\n", "\n");
        assertEquals(expected, actual, GOLDEN + " differs; if the change is intended, rerun with UPDATE_GOLDEN=1");
        // What the task writes is the same array on one line.
        assertEquals(actual.replace("\n", ""), ExtractTask.modelJson(nodes));
    }

    /** A map and topics that set every field the JSON has, optional ones included. */
    private static List<Object> model() {
        MapNode map = new MapNode();
        map.id = "user-guide";
        map.title = "User Guide";
        map.sourceFile = "user-guide.ditamap";
        map.keyDefs.add(keyDef("install", null, "installing", null, null, null, "user-guide.ditamap"));
        map.keyDefs.add(keyDef("brew", "tools", null, "https://brew.sh", null, "Homebrew", "keys.ditamap"));
        map.keyDefs.add(keyDef("logo", null, null, null, "images/logo.png", null, "keys.ditamap"));
        map.keyDefs.add(keyDef("product-name", null, null, null, null, "Widget Pro", "keys.ditamap"));
        map.metadata = new Metadata();
        map.metadata.created = "2026-01-15";
        map.metadata.products.add(new String[] {"Widget Pro", "4.2"});
        map.links.add(new Link("contains", "installing"));
        map.links.add(new Link("contains", "settings"));

        TopicNode task = new TopicNode();
        task.id = "installing";
        task.topicType = "task";
        task.title = "Installing \"Widget\" — café";
        task.shortdesc = "Install the product.";
        task.body = "Run the installer.\tThen restart.\nDone.";
        task.resolvedBody = "Run the installer. [reused from shared.dita#warning] Back up first. [end reused]";
        task.ditaClass = "- topic/topic task/task";
        task.audience.add("admin");
        task.product.add("enterprise");
        task.keys.add("install");
        task.uicontrols.add("Settings");
        task.cmdUicontrols.add("Save");
        task.keyrefs.add("tools.brew");
        task.externalLinks.put("https://brew.sh", "tools.brew");
        task.externalLinks.put("https://example.org/faq", null);
        task.codeRefs.add("samples/install.sh");
        task.sourceFile = "topics/installing.dita";
        task.authoredFile = "topics/install-source.dita";
        task.metadata = new Metadata();
        task.metadata.created = "2026-01-15";
        task.metadata.modified = "2026-03-01";
        task.metadata.golive = "2026-04-01";
        task.metadata.expiry = "2027-01-01";
        task.metadata.permissions = "internal";
        task.metadata.authors.add(new String[] {"Ada Writer", "creator"});
        task.metadata.authors.add(new String[] {"Bob Reviewer", null});
        task.metadata.products.add(new String[] {"Widget Pro", "4.2"});
        task.metadata.products.add(new String[] {"Widget Lite", null});
        task.metadata.audiences.add(new String[] {"administrator", "installing", "expert"});
        task.metadata.keywords.add("install");
        task.metadata.indexTerms.add(List.of("installing", "on Linux"));
        task.links.add(new Link("requires", "prerequisites"));
        task.links.add(new Link("references", "settings"));
        task.links.add(new Link("generated-from", "shared", List.of("warning", "li@37")));
        task.links.add(new Link("copy-of", "install-source"));

        // Only the fields every topic has: the optional ones are left out.
        TopicNode settings = new TopicNode();
        settings.id = "settings";
        settings.topicType = "reference";
        settings.title = "Settings";
        settings.sourceFile = "topics/settings.dita";

        TopicNode shared = new TopicNode();
        shared.id = "shared";
        shared.topicType = "concept";
        shared.title = "Shared warnings";
        shared.processingRole = "resource-only";
        shared.sourceFile = "topics/shared.dita";

        TopicNode heading = new TopicNode();
        heading.id = "topichead--reference";
        heading.topicType = "topichead";
        heading.title = "Reference";
        heading.sourceFile = "user-guide.ditamap#topichead--reference";
        heading.links.add(new Link("contains", "settings"));

        return List.of(map, task, settings, shared, heading,
                topic("prerequisites", "topics/prerequisites.dita"),
                topic("install-source", "topics/install-source.dita"));
    }

    private static TopicNode topic(String id, String sourceFile) {
        TopicNode topic = new TopicNode();
        topic.id = id;
        topic.title = id;
        topic.sourceFile = sourceFile;
        return topic;
    }

    private static KeyDef keyDef(String name, String scope, String target, String url, String file,
                                 String text, String sourceFile) {
        KeyDef key = new KeyDef();
        key.name = name;
        key.scope = scope;
        key.target = target;
        key.url = url;
        key.file = file;
        key.text = text;
        key.sourceFile = sourceFile;
        return key;
    }
}
