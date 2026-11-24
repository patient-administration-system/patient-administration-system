package uk.nhs.wales.pas.mobile

import androidx.compose.ui.test.junit4.accessibility.enableAccessibilityChecks
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.tryPerformAccessibilityChecks
import io.github.lilydesignsystem.compose.themes.LilyTheme
import io.github.lilydesignsystem.compose.themes.LilyTokens
import java.time.Instant
import org.junit.Rule
import org.junit.Test
import uk.nhs.wales.pas.mobile.core.AppointmentsModel
import uk.nhs.wales.pas.mobile.core.DemoAppointmentSource

/**
 * The accessibility checks (Accessibility Test Framework, through `ui-test-junit4-accessibility`) on the appointments screen,
 * on a device or emulator, in light and dark themes (NFR-36, MOB-6). No check is suppressed. The largest font scale and Welsh
 * are set by the runner (`adb shell settings put system font_scale 2.0`, `adb shell cmd locale set-app-locales`); see the
 * app's README.
 */
class AccessibilityTest {
    @get:Rule
    val compose = createComposeRule().apply { enableAccessibilityChecks() }

    private val now = Instant.parse("2026-11-23T12:00:00Z")

    private fun check(tokens: LilyTokens) {
        compose.setContent {
            LilyTheme(tokens) { AppointmentsScreen(AppointmentsModel(DemoAppointmentSource { now }, isDemo = true) { now }) }
        }
        compose.waitForIdle()
        compose.onNodeWithText("Upcoming").assertIsDisplayed()
        compose.onRoot().tryPerformAccessibilityChecks()
    }

    @Test fun lightThemePassesTheAccessibilityChecks() = check(LilyTokens.light)

    @Test fun darkThemePassesTheAccessibilityChecks() = check(LilyTokens.dark)
}
