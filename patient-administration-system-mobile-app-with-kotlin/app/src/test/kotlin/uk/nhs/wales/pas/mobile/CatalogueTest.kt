package uk.nhs.wales.pas.mobile

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.nhs.wales.pas.mobile.core.Messages

/** The string catalogues, read as files: the same rules as the web UI's i18n test and the iOS app's catalogue tests. */
class CatalogueTest {
    private val res = File("src/main/res")

    private fun strings(dir: String): Map<String, String> {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(File(res, "$dir/strings.xml"))
        val nodes = doc.getElementsByTagName("string")
        return (0 until nodes.length).associate { i ->
            val node = nodes.item(i)
            node.attributes.getNamedItem("name").nodeValue to node.textContent.replace("\\'", "'")
        }
    }

    @Test
    fun everyLocaleHasExactlyTheEnglishKeys() {
        val english = strings("values")
        assertTrue(english.size > 20)
        for (dir in listOf("values-cy")) {
            val catalogue = strings(dir)
            assertEquals(dir, english.keys, catalogue.keys)
            assertTrue(dir, catalogue.values.all { it.isNotBlank() })
        }
    }

    @Test
    fun everyKeyTheCodeUsesHasAString() {
        val english = strings("values")
        for (key in Messages.keys) {
            assertTrue(key, Messages.resourceName(key) in english)
            stringFor(key) // throws if unmapped
        }
    }

    /** Shared keys keep the web UI's wording, and the Android and iOS apps say the same thing. */
    @Test
    fun wordingMatchesTheWebUiAndTheIosApp() {
        val repo = File("../..").canonicalFile
        val web = File(repo, "patient-administration-system-ui-with-svelte/src/lib/messages")
        val ios = File(repo, "patient-administration-system-mobile-app-with-swift/Sources/PASMobileCore/Resources")
        val line = Regex("^\"([^\"]+)\" = \"(.*)\";$")
        for ((dir, webFile, lproj) in listOf(Triple("values", "en-001.json", "en"), Triple("values-cy", "cy-001.json", "cy"))) {
            val android = strings(dir)
            val iosStrings = File(ios, "$lproj.lproj/Localizable.strings").readLines()
                .mapNotNull { line.matchEntire(it.trim())?.destructured }.associate { (k, v) -> k to v }
            assertEquals(dir, iosStrings.keys.map(Messages::resourceName).toSet(), android.keys)
            val webJson = File(web, webFile).readText()
            for ((key, value) in iosStrings) {
                assertEquals("$dir $key", value, android[Messages.resourceName(key)])
                if (!key.startsWith("mobile.")) {
                    val pattern = Regex("\"" + Regex.escape(key) + "\"\\s*:\\s*\"((?:[^\"\\\\]|\\\\.)*)\"")
                    pattern.find(webJson)?.let { assertEquals("$dir $key (web)", it.groupValues[1], value) }
                }
            }
        }
    }
}
