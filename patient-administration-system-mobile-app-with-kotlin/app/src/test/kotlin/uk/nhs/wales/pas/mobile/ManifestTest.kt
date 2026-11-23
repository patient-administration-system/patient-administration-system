package uk.nhs.wales.pas.mobile

import java.io.File
import org.junit.Assert.assertFalse
import org.junit.Test

/** NFR-35: until the API itself enforces a patient's own record, the app cannot reach the network at all. */
class ManifestTest {
    @Test
    fun theAppAsksForNoNetworkPermission() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        assertFalse(manifest.contains("android.permission.INTERNET"))
        assertFalse(manifest.contains("ACCESS_NETWORK_STATE"))
    }
}
