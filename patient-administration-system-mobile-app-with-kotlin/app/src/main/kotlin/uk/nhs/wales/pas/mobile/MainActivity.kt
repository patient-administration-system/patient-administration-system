// The Android patient app (spec/mobile-apps/index.md). It shows the demo source only: the app may not call a real PAS until
// NFR-35 holds (the API must itself refuse a patient another person's record). Edge to edge, light or dark with the system.
package uk.nhs.wales.pas.mobile

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import io.github.lilydesignsystem.compose.themes.LilyTheme
import io.github.lilydesignsystem.compose.themes.LilyTokens
import uk.nhs.wales.pas.mobile.core.AppointmentsModel
import uk.nhs.wales.pas.mobile.core.DemoAppointmentSource

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val model = remember { AppointmentsModel(DemoAppointmentSource(), isDemo = true) }
            LilyTheme(if (isSystemInDarkTheme()) LilyTokens.dark else LilyTokens.light) {
                AppointmentsScreen(model, Modifier.safeDrawingPadding())
            }
        }
    }
}
