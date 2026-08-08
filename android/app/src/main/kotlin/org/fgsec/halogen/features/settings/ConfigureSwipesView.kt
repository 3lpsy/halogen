package org.fgsec.halogen.features.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.core.SwipePrefsModel

/// Configure per-page episode swipe actions (the web's
/// `/settings/configure-swipes`, scoped to the browsing lists — structural
/// pages keep their built-in swipes).
@Composable
fun ConfigureSwipesView(swipes: SwipePrefsModel, onBack: () -> Unit) {
    SettingsScaffold(title = "Configure swipes", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            for (page in SwipePage.entries) {
                SettingsGroup(header = page.label) {
                    // Per-page vocabulary: remove-from-list only where the
                    // list IS a playlist (SwipePage.allowedActions).
                    PickerRow(
                        "Leading (full swipe)",
                        selected = swipes.prefs.page(page).leading,
                        options = page.allowedActions,
                        optionLabel = { it.label },
                    ) { new ->
                        swipes.update(page, leading = new, trailing = swipes.prefs.page(page).trailing)
                    }
                    PickerRow(
                        "Trailing",
                        selected = swipes.prefs.page(page).trailing,
                        options = page.allowedActions,
                        optionLabel = { it.label },
                    ) { new ->
                        swipes.update(page, leading = swipes.prefs.page(page).leading, trailing = new)
                    }
                }
            }
        }
    }
}
