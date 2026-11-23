package uk.nhs.wales.pas.mobile.core

import java.time.Instant
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** The state of the appointments screen. */
public sealed interface LoadState {
    public data object Loading : LoadState
    public data class Loaded(val list: AppointmentList) : LoadState
    public data class Failed(val problem: LoadProblem) : LoadState
}

/** Loads appointments and holds what the screen shows. Pure state, so it is tested without a view. */
public class AppointmentsModel(
    private val source: AppointmentSource,
    public val isDemo: Boolean,
    private val now: () -> Instant = Instant::now,
) {
    private val mutableState = MutableStateFlow<LoadState>(LoadState.Loading)
    public val state: StateFlow<LoadState> = mutableState.asStateFlow()

    public suspend fun load() {
        mutableState.value = LoadState.Loading
        mutableState.value = try {
            LoadState.Loaded(AppointmentList.of(source.appointments(), now()))
        } catch (e: LoadProblemException) {
            // A person the PAS does not hold has, as far as it knows, no appointments.
            if (e.problem == LoadProblem.NotFound) LoadState.Loaded(AppointmentList.of(emptyList(), now())) else LoadState.Failed(e.problem)
        }
    }
}
