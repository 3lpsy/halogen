package org.fgsec.halogen.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import coil3.compose.AsyncImage
import coil3.compose.LocalPlatformContext

/// Square rounded artwork tile with a placeholder. URLs point at the server's
/// art cache; ArtLoader fetches them with the API bearer token.
@Composable
fun Artwork(url: String?, size: Dp) {
    // iOS @ScaledMetric parity: the tile grows with the UI-size pref so a
    // bigger UI doesn't pair big labels with shrunken-looking art.
    val scaled = size * LocalDensity.current.fontScale
    Box(
        Modifier
            .size(scaled)
            .clip(RoundedCornerShape(scaled / 7))
            .background(halogenExtras.placeholderFill),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            halogenIcon("waveform"),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (url != null) {
            AsyncImage(
                model = url,
                contentDescription = null,
                imageLoader = ArtLoader.loader(LocalPlatformContext.current),
                contentScale = ContentScale.Crop,
                modifier = Modifier.matchParentSize(),
            )
        }
    }
}
