package uk.nhs.wales.pas.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.lilydesignsystem.compose.ErrorSummary
import io.github.lilydesignsystem.compose.InsetText
import io.github.lilydesignsystem.compose.LilyCard
import io.github.lilydesignsystem.compose.LilyMinimumTouchTarget
import io.github.lilydesignsystem.compose.LocalLilyTextStyles
import io.github.lilydesignsystem.compose.Problem
import io.github.lilydesignsystem.compose.SummaryList
import io.github.lilydesignsystem.compose.SummaryListItem
import io.github.lilydesignsystem.compose.Tag
import io.github.lilydesignsystem.compose.themes.LocalLilyTokens
import java.util.Locale
import kotlinx.coroutines.launch
import uk.nhs.wales.pas.mobile.core.Appointment
import uk.nhs.wales.pas.mobile.core.AppointmentFormat
import uk.nhs.wales.pas.mobile.core.AppointmentsModel
import uk.nhs.wales.pas.mobile.core.LoadState
import uk.nhs.wales.pas.mobile.core.Messages

/** The patient's appointments (FR-227): upcoming first, then past, each as a Lily card with a summary list. */
@Composable
fun AppointmentsScreen(model: AppointmentsModel, modifier: Modifier = Modifier) {
    val state by model.state.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    LaunchedEffect(model) { model.load() }
    val styles = LocalLilyTextStyles.current
    Column(
        modifier = modifier.fillMaxSize().background(LocalLilyTokens.current.base100)
            .verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        BasicText(
            stringResource(R.string.myAppointments_title),
            style = styles.heading.copy(fontSize = 28.sp),
            modifier = Modifier.semantics { heading() },
        )
        when (val s = state) {
            LoadState.Loading -> BasicText(stringResource(R.string.mobile_loading), style = styles.body)
            is LoadState.Failed -> {
                ErrorSummary(
                    title = stringResource(R.string.mobile_error_title),
                    problems = listOf(Problem(stringResource(stringFor(s.problem.messageKey))) {}),
                )
                BasicText(
                    stringResource(R.string.mobile_retry),
                    style = styles.strong,
                    modifier = Modifier.sizeIn(minHeight = LilyMinimumTouchTarget)
                        .clickable(role = Role.Button) { scope.launch { model.load() } }
                        .padding(vertical = 12.dp),
                )
            }
            is LoadState.Loaded -> {
                if (model.isDemo) InsetText { BasicText(stringResource(R.string.mobile_demoNotice), style = styles.body) }
                if (s.list.isEmpty) BasicText(stringResource(R.string.myAppointments_empty), style = styles.body)
                Section(R.string.mobile_upcoming, s.list.upcoming)
                Section(R.string.mobile_past, s.list.past)
            }
        }
    }
}

@Composable
private fun Section(heading: Int, items: List<Appointment>) {
    if (items.isEmpty()) return
    val styles = LocalLilyTextStyles.current
    BasicText(
        stringResource(heading),
        style = styles.heading.copy(fontSize = 22.sp, fontWeight = FontWeight.Bold),
        modifier = Modifier.semantics { heading() },
    )
    items.forEach { Row(it) }
}

@Composable
private fun Row(appointment: Appointment) {
    val styles = LocalLilyTextStyles.current
    val language = LocalConfiguration.current.locales[0].language
    // Dates in the catalogue's language, so the words around a date and the date agree.
    val locale = Locale.Builder().setLanguage(if (language in Messages.locales) language else "en").setRegion("GB").build()
    val whenText = AppointmentFormat.`when`(appointment, locale)
    val statusTerm = stringResource(R.string.mobile_status)
    val status = stringResource(stringFor(appointment.status.messageKey))
    LilyCard(modifier = Modifier.fillMaxWidth(), label = whenText) {
        SummaryList(label = whenText) {
            SummaryListItem(term = stringResource(R.string.mobile_when)) { BasicText(whenText, style = styles.body) }
            SummaryListItem(term = stringResource(R.string.mobile_how)) {
                BasicText(stringResource(stringFor(appointment.modality.messageKey)), style = styles.body)
            }
            SummaryListItem(term = statusTerm) { Tag(text = status, label = statusTerm) }
        }
    }
}
