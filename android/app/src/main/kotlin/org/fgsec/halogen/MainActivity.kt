package org.fgsec.halogen

import android.Manifest
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import org.fgsec.halogen.core.HalogenCore
import java.io.File

class MainActivity : ComponentActivity() {
    // Denial is not fatal: playback works, only the media notification is
    // suppressed — so the result is ignored (iOS has no equivalent prompt).
    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val debuggable = (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
        DebugHooks.capture(intent, debuggable)
        val app = application as App
        if (DebugHooks.reset && app.core == null) wipeAllState()
        val core = app.core ?: HalogenCore(applicationContext, app.mainScope).also { app.core = it }
        requestNotificationPermission()
        setContent { RootView(core) }
    }

    /// Android 13+ gates the media notification behind a runtime grant.
    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val granted = ContextCompat.checkSelfPermission(
            this, Manifest.permission.POST_NOTIFICATIONS
        ) == PackageManager.PERMISSION_GRANTED
        if (!granted) notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    /// DEBUG HALOGEN_RESET: factory-fresh state before the core ever boots.
    private fun wipeAllState() {
        File(filesDir, "halogen-client").deleteRecursively()
        File(filesDir, "halogen-server").deleteRecursively()
        File(filesDir, "device-log.json").delete()
        for (name in listOf("org.fgsec.halogen.session", "org.fgsec.halogen.embedded-secrets")) {
            deleteSharedPreferences(name)
        }
        getSharedPreferences("halogen", MODE_PRIVATE).edit().clear().apply()
    }
}
