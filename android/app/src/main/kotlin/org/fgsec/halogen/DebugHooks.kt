package org.fgsec.halogen

import android.content.Intent
import android.os.Bundle

/// DEBUG smoke-test hooks (the iOS launch-environment vocabulary): read from
/// launch-intent extras / instrumentation args on debuggable builds only.
/// HALOGEN_RESET wipes app state before boot; the rest drive flows headlessly.
object DebugHooks {
    var autoconnect: String? = null
        private set
    var tab: String? = null
        private set
    var autoplay: Int? = null
        private set
    var autodownload: Int? = null
        private set
    var audioOverride: String? = null
        private set
    var reset = false
        private set

    fun capture(intent: Intent?, debuggable: Boolean) {
        if (!debuggable) return
        val extras: Bundle = intent?.extras ?: return
        autoconnect = extras.getString("HALOGEN_AUTOCONNECT") ?: autoconnect
        tab = extras.getString("HALOGEN_TAB") ?: tab
        autoplay = extras.getString("HALOGEN_AUTOPLAY")?.toIntOrNull() ?: autoplay
        autodownload = extras.getString("HALOGEN_AUTODOWNLOAD")?.toIntOrNull() ?: autodownload
        audioOverride = extras.getString("HALOGEN_AUDIO_OVERRIDE") ?: audioOverride
        reset = extras.getString("HALOGEN_RESET") == "1" || reset
    }
}
