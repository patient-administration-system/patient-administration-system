package uk.nhs.wales.pas.mobile

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import io.github.lilydesignsystem.compose.themes.LilyTheme
import io.github.lilydesignsystem.compose.themes.LilyTokens
import java.time.Instant
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uk.nhs.wales.pas.mobile.core.AppointmentsModel
import uk.nhs.wales.pas.mobile.core.DemoAppointmentSource
import uk.nhs.wales.pas.mobile.core.LoadProblem
import uk.nhs.wales.pas.mobile.core.LoadProblemException

@RunWith(RobolectricTestRunner::class)
class AppointmentsScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val now = Instant.parse("2026-11-23T12:00:00Z")

    private fun show(model: AppointmentsModel) = compose.setContent {
        LilyTheme(LilyTokens.light) { AppointmentsScreen(model) }
    }

    /** FR-227, FR-229: the demo shows its notice, the upcoming and past headings, and each status. */
    @Test
    fun demoShowsUpcomingAndPastInEnglish() {
        show(AppointmentsModel(DemoAppointmentSource { now }, isDemo = true) { now })
        compose.onNodeWithText("These are example appointments", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Upcoming").assertIsDisplayed()
        compose.onNodeWithText("Past").assertExists()
        val booked = compose.onAllNodesWithContentDescription("Booked", substring = true, useUnmergedTree = true).fetchSemanticsNodes().size
        assertTrue("two booked, saw $booked", booked == 2)
        compose.onNodeWithContentDescription("Completed", substring = true, useUnmergedTree = true).assertExists()
        compose.onNodeWithText("Video").assertExists()
    }

    /** The Welsh catalogue and Welsh dates. */
    @Test
    @Config(qualifiers = "cy")
    fun demoShowsWelsh() {
        show(AppointmentsModel(DemoAppointmentSource { now }, isDemo = true) { now })
        compose.onNodeWithText("Fy apwyntiadau").assertIsDisplayed()
        compose.onNodeWithText("I ddod").assertIsDisplayed()
        assertTrue(compose.onAllNodesWithText("Dydd", substring = true).fetchSemanticsNodes().isNotEmpty())
    }

    /** FR-228: a failure shows the app's own words and a retry that loads again. */
    @Test
    fun failureShowsTheAppsWordsAndRetries() {
        var calls = 0
        show(AppointmentsModel({ calls++; throw LoadProblemException(LoadProblem.Unreachable) }, isDemo = false) { now })
        compose.onNodeWithText("There is a problem").assertIsDisplayed()
        compose.onNodeWithText("We could not reach the appointments service", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Try again").performClick()
        compose.waitForIdle()
        assertTrue("retry loads again", calls >= 2)
    }
}
