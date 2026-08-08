package org.fgsec.halogen

import android.app.Application
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.HalogenCore
import uniffi.halogen_mobile.initCore

class App : Application() {
    /// App-lifetime Main scope: the core (and its session jobs) outlive any
    /// single activity — the media service shares the same process state.
    val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    var core: HalogenCore? = null

    override fun onCreate() {
        super.onCreate()
        // Installs the Rust tracing subscriber (idempotent).
        initCore()
        // The diagnostic ring must exist before any layer logs into it.
        DeviceLog.initialize(this)
    }
}
