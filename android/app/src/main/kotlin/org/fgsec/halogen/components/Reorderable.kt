package org.fgsec.halogen.components

import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.zIndex
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/// Drag-to-reorder for LazyColumn rows via an explicit drag handle (SwiftUI
/// `.onMove` stand-in, no third-party lib): a single `onMove(from, to)` commit
/// fires at drop, so models enqueue exactly one durable op per drag (iOS parity).
@Stable
class ReorderableListState internal constructor(
    private val listState: LazyListState,
    private val scope: CoroutineScope,
    /// Kept current by rememberReorderableListState on every recomposition —
    /// a once-captured lambda would close over stale caller state.
    internal var onMove: (fromIndex: Int, toIndex: Int) -> Unit,
) {
    var draggingIndex: Int? by mutableStateOf(null)
        private set
    var targetIndex: Int? by mutableStateOf(null)
        private set
    private var dragOffset by mutableFloatStateOf(0f)
    private var draggedSize = 0

    /// Begin a drag on the item whose LazyColumn key is `key`.
    fun startDrag(key: Any) {
        val info = listState.layoutInfo.visibleItemsInfo.firstOrNull { it.key == key } ?: return
        draggingIndex = info.index
        targetIndex = info.index
        dragOffset = 0f
        draggedSize = info.size
    }

    fun drag(delta: Float) {
        val from = draggingIndex ?: return
        dragOffset += delta
        val info = visible(from) ?: return
        val center = info.offset + dragOffset + info.size / 2f
        listState.layoutInfo.visibleItemsInfo
            .firstOrNull { it.index != from && center >= it.offset && center < it.offset + it.size }
            ?.let { targetIndex = it.index }
        // Edge auto-scroll; the offset compensation keeps the floating row
        // under the (stationary) finger while the list moves beneath it.
        val top = info.offset + dragOffset
        val bottom = top + info.size
        val overshoot = when {
            top < listState.layoutInfo.viewportStartOffset ->
                top - listState.layoutInfo.viewportStartOffset
            bottom > listState.layoutInfo.viewportEndOffset ->
                bottom - listState.layoutInfo.viewportEndOffset
            else -> 0f
        }
        if (overshoot != 0f) scope.launch { dragOffset += listState.scrollBy(overshoot) }
    }

    fun endDrag() {
        val from = draggingIndex
        val to = targetIndex
        clear()
        if (from != null && to != null && from != to) onMove(from, to)
    }

    fun cancelDrag() = clear()

    /// Visual Y shift for the row currently composed at `index`.
    fun displacement(index: Int): Float {
        val from = draggingIndex ?: return 0f
        val to = targetIndex ?: return 0f
        return when {
            index == from -> dragOffset
            index in (from + 1)..to -> -draggedSize.toFloat()
            index in to until from -> draggedSize.toFloat()
            else -> 0f
        }
    }

    private fun clear() {
        draggingIndex = null
        targetIndex = null
        dragOffset = 0f
        draggedSize = 0
    }

    private fun visible(index: Int) =
        listState.layoutInfo.visibleItemsInfo.firstOrNull { it.index == index }
}

@Composable
fun rememberReorderableListState(
    listState: LazyListState,
    onMove: (fromIndex: Int, toIndex: Int) -> Unit,
): ReorderableListState {
    val scope = rememberCoroutineScope()
    val state = remember(listState) { ReorderableListState(listState, scope, onMove) }
    state.onMove = onMove
    return state
}

/// Apply to each item's outermost container (items must be keyed).
fun Modifier.reorderableItem(state: ReorderableListState, index: Int): Modifier =
    zIndex(if (state.draggingIndex == index) 1f else 0f)
        .graphicsLayer { translationY = state.displacement(index) }

/// Apply to the row's drag handle; `key` must be the item's LazyColumn key.
fun Modifier.reorderHandle(state: ReorderableListState, key: Any): Modifier =
    pointerInput(state, key) {
        detectDragGestures(
            onDragStart = { state.startDrag(key) },
            onDrag = { change, amount ->
                change.consume()
                state.drag(amount.y)
            },
            onDragEnd = { state.endDrag() },
            onDragCancel = { state.cancelDrag() },
        )
    }
