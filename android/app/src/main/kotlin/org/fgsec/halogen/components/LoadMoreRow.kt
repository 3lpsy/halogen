package org.fgsec.halogen.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

/// The infinite-scroll sentinel: a spinner that fires `loadMore` on appear,
/// or — after a failed page — a tappable retry. (A swallowed loadMore error
/// used to leave a spinner that never resolves at the fold.)
@Composable
fun LoadMoreRow(
    failed: Boolean,
    loadMore: suspend () -> Unit,
) {
    val scope = rememberCoroutineScope()

    Row(
        Modifier.fillMaxWidth().padding(vertical = 12.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (failed) {
            TextButton(onClick = { scope.launch { loadMore() } }) {
                Icon(
                    halogenIcon("arrow.clockwise"),
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    "Couldn't load more — retry",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(start = 6.dp),
                )
            }
        } else {
            CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.5.dp)
            LaunchedEffect(Unit) { loadMore() }
        }
    }
}
