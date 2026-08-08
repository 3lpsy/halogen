package org.fgsec.halogen.components

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.layout.onSizeChanged
import kotlinx.coroutines.launch
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipeAction
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.core.SwipeTint
import org.fgsec.halogen.core.performSwipeAction
import org.fgsec.halogen.wire.EpisodeData
import kotlin.math.abs
import kotlin.math.roundToInt

/// One revealed swipe button (screen-local destructive extras and the
/// configured-page actions both reduce to this).
data class RowSwipeAction(
    val label: String,
    val systemImage: String,
    val destructive: Boolean = false,
    val perform: () -> Unit,
)

/// Alias shape used by Downloads/History call sites.
data class ExtraSwipe(
    val label: String,
    val systemImage: String,
    val destructive: Boolean = false,
    val onClick: () -> Unit,
)

/// Semantic tint → theme color (no hardcoded colors outside Theme.kt). Resolved
/// eagerly for every tint so non-composable helpers can look colors up.
@Composable
private fun swipeTintColors(): Map<SwipeTint, Color> {
    val scheme = MaterialTheme.colorScheme
    val extras = halogenExtras
    return mapOf(
        SwipeTint.Gray to scheme.outline,
        SwipeTint.Indigo to scheme.primary,
        SwipeTint.Blue to scheme.primary,
        SwipeTint.Green to extras.success,
        SwipeTint.Red to scheme.error,
        SwipeTint.Purple to scheme.tertiary,
        SwipeTint.Orange to extras.warning,
    )
}

/// Generic reveal-on-drag swipe row: drag reveals tinted buttons (tap to
/// perform); a full LEADING swipe fires the first leading action, trailing
/// always requires a tap — iOS swipeActions semantics on Compose primitives.
@Composable
fun SwipeRevealRow(
    leading: List<Pair<RowSwipeAction, Color>>,
    trailing: List<Pair<RowSwipeAction, Color>>,
    content: @Composable () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val offset = remember { Animatable(0f) }
    var rowWidth by remember { mutableStateOf(0) }
    val density = LocalDensity.current
    val buttonWidthPx = with(density) { 88.dp.toPx() }
    val leadingMax = leading.size * buttonWidthPx
    val trailingMax = trailing.size * buttonWidthPx

    fun close() = scope.launch { offset.animateTo(0f) }

    Box(
        modifier = Modifier.onSizeChanged { rowWidth = it.width },
    ) {
        // Buttons behind the row content, revealed by the drag.
        Row(modifier = Modifier.matchParentSize()) {
            leading.forEach { (action, color) ->
                SwipeButton(action, color, visible = offset.value > 0f) { close() }
            }
            Box(Modifier.weight(1f))
            trailing.forEach { (action, color) ->
                SwipeButton(action, color, visible = offset.value < 0f) { close() }
            }
        }
        Box(
            modifier = Modifier
                .offset { IntOffset(offset.value.roundToInt(), 0) }
                .background(MaterialTheme.colorScheme.background)
                .pointerInput(leading.size, trailing.size) {
                    if (leading.isEmpty() && trailing.isEmpty()) return@pointerInput
                    detectHorizontalDragGestures(
                        onDragEnd = {
                            val v = offset.value
                            // iOS parity: only the leading edge full-swipes
                            // (its FIRST action); trailing needs a tap.
                            val full = rowWidth * 0.6f
                            scope.launch {
                                when {
                                    v > full && leading.isNotEmpty() -> {
                                        leading.first().first.perform(); offset.animateTo(0f)
                                    }
                                    v > leadingMax / 2 -> offset.animateTo(leadingMax)
                                    -v > trailingMax / 2 -> offset.animateTo(-trailingMax)
                                    else -> offset.animateTo(0f)
                                }
                            }
                        },
                        onHorizontalDrag = { change, dragAmount ->
                            change.consume()
                            // Leading drags may cross the row (full swipe);
                            // trailing stops at its revealed buttons.
                            val leadingLimit =
                                if (leading.isEmpty()) 1f
                                else maxOf(rowWidth.toFloat(), leadingMax)
                            val target = (offset.value + dragAmount)
                                .coerceIn(-maxOf(trailingMax, 1f), leadingLimit)
                            scope.launch { offset.snapTo(target) }
                        },
                    )
                },
        ) { content() }
    }
}

@Composable
private fun SwipeButton(
    action: RowSwipeAction,
    color: Color,
    visible: Boolean,
    close: () -> Unit,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier
            .width(88.dp)
            .fillMaxHeight()
            .background(if (visible) color else Color.Transparent)
            .clickable(enabled = visible) {
                action.perform()
                close()
            }
            .padding(vertical = 8.dp),
    ) {
        if (visible) {
            Icon(halogenIcon(action.systemImage), contentDescription = action.label, tint = Color.White)
            Text(action.label, style = MaterialTheme.typography.labelSmall, color = Color.White, maxLines = 1)
        }
    }
}

/// Screen-local trailing actions only (playlist rows' Delete, …).
@Composable
fun SwipeActionsRow(trailing: List<RowSwipeAction>, content: @Composable () -> Unit) {
    val resolved = trailing.map {
        it to if (it.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary
    }
    SwipeRevealRow(leading = emptyList(), trailing = resolved, content = content)
}

/// The user-configured page swipes (Settings → Configure swipes) around a row.
@Composable
fun ConfiguredSwipes(
    page: SwipePage,
    episode: EpisodeData,
    core: HalogenCore,
    extraTrailing: ExtraSwipe? = null,
    context: EpisodeMenuContext = EpisodeMenuContext.Browse,
    content: @Composable () -> Unit,
) {
    val prefs = core.models?.swipes?.prefs?.page(page)
    val tints = swipeTintColors()
    val errorColor = MaterialTheme.colorScheme.error
    val primaryColor = MaterialTheme.colorScheme.primary
    fun resolve(action: SwipeAction): Pair<RowSwipeAction, Color>? {
        if (action == SwipeAction.None) return null
        return RowSwipeAction(
            label = action.label,
            systemImage = action.systemImage,
            destructive = action.tint == SwipeTint.Red,
            perform = { performSwipeAction(action, episode, core, context) },
        ) to (tints[action.tint] ?: primaryColor)
    }
    val leading = listOfNotNull(prefs?.leading?.let(::resolve))
    val trailing = buildList {
        prefs?.trailing?.let(::resolve)?.let(::add)
        extraTrailing?.let {
            add(
                RowSwipeAction(it.label, it.systemImage, it.destructive, it.onClick) to
                    if (it.destructive) errorColor else primaryColor
            )
        }
    }
    SwipeRevealRow(leading = leading, trailing = trailing, content = content)
}

/// Queue/playlist variant that threads the menu context (RemoveFromList etc.).
@Composable
fun ConfiguredSwipeRow(
    page: SwipePage,
    episode: EpisodeData,
    core: HalogenCore,
    context: EpisodeMenuContext,
    extraTrailing: RowSwipeAction? = null,
    content: @Composable () -> Unit,
) {
    ConfiguredSwipes(
        page = page, episode = episode, core = core,
        extraTrailing = extraTrailing?.let { ExtraSwipe(it.label, it.systemImage, it.destructive, it.perform) },
        context = context, content = content,
    )
}
