package org.fgsec.halogen.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.unit.dp
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.SetSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import org.fgsec.halogen.features.latest.EpisodeFilter
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.wire.DownloadStatus
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaybackStatus

/// Sort field vocabulary for episode lists (server `order[order_by]` names).
/// `Position` is the queue's pivot order (local, queue-only).
@Serializable
enum class EpisodeOrderField(val rawValue: String) {
    @SerialName("published_at") Published("published_at"),
    @SerialName("created_at") Added("created_at"),
    @SerialName("title") Title("title"),
    @SerialName("duration_secs") Duration("duration_secs"),
    @SerialName("position") Position("position"),
    /// History only: play-recency order (the web's non-selectable UpdatedAt
    /// sentinel, made selectable) — local, never sent to the server. A
    /// dedicated case so an EXPLICIT "Published desc" choice is
    /// distinguishable from the recency default.
    @SerialName("recency") Recency("recency");

    val label: String
        get() = when (this) {
            Published -> "Published"
            Added -> "Added"
            Title -> "Title"
            Duration -> "Duration"
            Position -> "Position"
            Recency -> "Recent"
        }
}

/// LocalStore keys for the per-page persisted ListQuery — byte-identical to
/// the iOS keys. The owning feature models load/save these, as on iOS.
object ListQueryKeys {
    const val latest = "listquery-latest"
    const val queue = "listquery-queue"
    const val history = "listquery-history"
    const val downloads = "listquery-downloads"
    const val podcastEpisodes = "listquery-podcast-episodes"
    const val playlist = "listquery-playlist"
}

/// One episode list's controls state: search + filter chips + order.
/// Persisted per page (the web's per-view-key sticky list config). Chips are
/// multi-select: OR within a facet, AND across facets (web query.rs).
@Serializable(with = ListQuerySerializer::class)
data class ListQuery(
    val search: String = "",
    val filters: Set<EpisodeFilter> = emptySet(),
    val orderField: EpisodeOrderField = EpisodeOrderField.Published,
    val direction: OrderDirection = OrderDirection.Desc,
) {
    /// Selected chips of each facet.
    val downloadChips: Set<EpisodeFilter>
        get() = filters.intersect(EpisodeFilter.downloadFacet.toSet())
    val playedChips: Set<EpisodeFilter>
        get() = filters.intersect(EpisodeFilter.playedFacet.toSet())

    /// Whether the server page must be re-filtered locally: a facet with
    /// MULTIPLE chips can't be expressed as a single wire token, so the fetch
    /// goes un-narrowed for that facet and `matchesChips` trims each page.
    val needsLocalChipFilter: Boolean
        get() = downloadChips.size > 1 || playedChips.size > 1

    /// Server-side query fragment (search + single-chip facets + order).
    /// Queue/History apply the same query locally instead.
    val queryItems: List<Pair<String, String>>
        get() {
            val items = mutableListOf<Pair<String, String>>()
            val trimmed = search.trim()
            if (trimmed.isNotEmpty()) {
                items += "filter[search]" to trimmed
            }
            downloadChips.singleOrNull()?.wireToken?.let {
                items += "filter[download_status]" to it
            }
            playedChips.singleOrNull()?.wireToken?.let {
                items += "filter[playback_status]" to it
            }
            if (orderField != EpisodeOrderField.Position && orderField != EpisodeOrderField.Recency) {
                items += "order[order_by]" to orderField.rawValue
                items += "order[direction]" to
                    if (direction == OrderDirection.Asc) "Asc" else "Desc"
            }
            return items
        }

    /// Chip predicate over one row: OR within a facet, AND across facets (web
    /// apply_filter_sort). `isOnDevice` null = OnDevice chip no-op (caller switches
    /// sets wholesale); `status` reads the playbacks overlay so local marks flip chips.
    fun matchesChips(
        episode: EpisodeData,
        isOnDevice: ((Int) -> Boolean)? = null,
        status: ((EpisodeData) -> PlaybackStatus)? = null,
    ): Boolean {
        if (filters.contains(EpisodeFilter.OnDevice) && isOnDevice != null &&
            !isOnDevice(episode.id)
        ) {
            return false
        }
        val dl = downloadChips
        if (dl.isNotEmpty()) {
            val ok = dl.any { chip ->
                when (chip) {
                    EpisodeFilter.Downloaded ->
                        episode.download_status == DownloadStatus.Downloaded
                    EpisodeFilter.Downloading ->
                        episode.download_status == DownloadStatus.Downloading
                    else -> false
                }
            }
            if (!ok) return false
        }
        val played = playedChips
        if (played.isNotEmpty()) {
            val resolved = status?.invoke(episode)
                ?: episode.playback_status ?: PlaybackStatus.Unplayed
            val ok = played.any { chip ->
                when (chip) {
                    EpisodeFilter.Unplayed -> resolved == PlaybackStatus.Unplayed
                    EpisodeFilter.InProgress -> resolved == PlaybackStatus.Played
                    EpisodeFilter.Finished -> resolved == PlaybackStatus.Finished
                    else -> false
                }
            }
            if (!ok) return false
        }
        return true
    }

    /// Local projection of the same query (the queue's in-memory list).
    fun apply(
        episodes: List<EpisodeData>,
        isOnDevice: ((Int) -> Boolean)? = null,
        status: ((EpisodeData) -> PlaybackStatus)? = null,
    ): List<EpisodeData> {
        var out = episodes
        val trimmed = search.trim().lowercase()
        if (trimmed.isNotEmpty()) {
            out = out.filter {
                it.title.lowercase().contains(trimmed) ||
                    (it.description?.lowercase()?.contains(trimmed) ?: false) ||
                    (it.podcast?.title?.lowercase()?.contains(trimmed) ?: false)
            }
        }
        out = out.filter { matchesChips(it, isOnDevice, status) }
        return when (orderField) {
            // History sorts recency itself (overlay-wins dates live there);
            // as a plain local sort this is a no-op passthrough.
            EpisodeOrderField.Recency -> out
            // Base order IS position (asc) — desc just flips it (web parity).
            EpisodeOrderField.Position ->
                if (direction == OrderDirection.Desc) out.reversed() else out
            EpisodeOrderField.Published ->
                if (direction == OrderDirection.Asc) out.sortedBy { publishedMillis(it) }
                else out.sortedByDescending { publishedMillis(it) }
            EpisodeOrderField.Added ->
                if (direction == OrderDirection.Asc) out.sortedBy { createdMillis(it) }
                else out.sortedByDescending { createdMillis(it) }
            EpisodeOrderField.Title ->
                if (direction == OrderDirection.Asc) out.sortedBy { it.title.lowercase() }
                else out.sortedByDescending { it.title.lowercase() }
            EpisodeOrderField.Duration ->
                if (direction == OrderDirection.Asc) out.sortedBy { it.duration_secs ?: 0 }
                else out.sortedByDescending { it.duration_secs ?: 0 }
        }
    }

    private fun publishedMillis(episode: EpisodeData): Long =
        episode.published_at
            ?.let { runCatching { WireJson.parseInstant(it).toEpochMilli() }.getOrNull() }
            ?: Long.MIN_VALUE

    private fun createdMillis(episode: EpisodeData): Long =
        runCatching { WireJson.parseInstant(episode.created_at).toEpochMilli() }
            .getOrDefault(Long.MIN_VALUE)
}

/// Lenient per-key decode (the iOS init(from:) shape): pre-multiselect
/// snapshots or a vocabulary change reset the affected field to its default —
/// never fail the whole restore.
object ListQuerySerializer : KSerializer<ListQuery> {
    private val filtersSerializer = SetSerializer(EpisodeFilter.serializer())

    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("org.fgsec.halogen.components.ListQuery")

    override fun serialize(encoder: Encoder, value: ListQuery) {
        val out = encoder as JsonEncoder
        out.encodeJsonElement(
            buildJsonObject {
                put("search", out.json.encodeToJsonElement(String.serializer(), value.search))
                put("filters", out.json.encodeToJsonElement(filtersSerializer, value.filters))
                put(
                    "orderField",
                    out.json.encodeToJsonElement(EpisodeOrderField.serializer(), value.orderField))
                put(
                    "direction",
                    out.json.encodeToJsonElement(OrderDirection.serializer(), value.direction))
            })
    }

    override fun deserialize(decoder: Decoder): ListQuery {
        val input = decoder as JsonDecoder
        val obj = input.decodeJsonElement() as? JsonObject ?: return ListQuery()
        fun <T> field(key: String, serializer: KSerializer<T>, default: T): T =
            obj[key]
                ?.let { runCatching { input.json.decodeFromJsonElement(serializer, it) }.getOrNull() }
                ?: default
        return ListQuery(
            search = field("search", String.serializer(), ""),
            filters = field("filters", filtersSerializer, emptySet()),
            orderField = field("orderField", EpisodeOrderField.serializer(), EpisodeOrderField.Published),
            direction = field("direction", OrderDirection.serializer(), OrderDirection.Desc),
        )
    }
}

/// The persistent sub-navbar on every episode list: search field + Filter
/// dropdown + Order dropdown (the web's list controls, menu-style).
@Composable
fun ListControlsBar(
    query: ListQuery,
    onQueryChange: (ListQuery) -> Unit,
    /// The queue offers Position ordering; other pages don't.
    allowsPosition: Boolean = false,
    /// History offers the Recent (play-recency) ordering; other pages don't.
    allowsRecency: Boolean = false,
    /// Downloads fixes its facet via the segmented control — hide the filter
    /// menu there (every other list shows it, History included; web parity).
    showFilter: Boolean = true,
    /// Embedded accounts have no separate device set — the OnDevice chip is
    /// dropped there (web: `embedded` prop on ListControls).
    allowsOnDevice: Boolean = true,
) {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainerLow)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SearchField(
                value = query.search,
                onValueChange = { onQueryChange(query.copy(search = it)) },
                placeholder = "Search",
                modifier = Modifier.weight(1f),
            )
            if (showFilter) {
                FilterMenuButton(
                    query = query, onQueryChange = onQueryChange,
                    allowsOnDevice = allowsOnDevice,
                )
            }
            OrderMenuButton(
                query = query, onQueryChange = onQueryChange,
                allowsPosition = allowsPosition, allowsRecency = allowsRecency,
            )
        }
        HorizontalDivider()
    }
}

/// Multi-select chips (web ListControls): tapping toggles membership; the
/// menu stays coherent because facets OR internally / AND across.
@Composable
private fun FilterMenuButton(
    query: ListQuery,
    onQueryChange: (ListQuery) -> Unit,
    allowsOnDevice: Boolean,
) {
    var open by remember { mutableStateOf(false) }
    val availableChips = EpisodeFilter.entries.filter { allowsOnDevice || it != EpisodeFilter.OnDevice }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(
                halogenIcon(
                    if (query.filters.isEmpty()) "line.3.horizontal.decrease.circle"
                    else "line.3.horizontal.decrease.circle.fill"),
                contentDescription = "Filter",
                tint = if (query.filters.isEmpty()) MaterialTheme.colorScheme.onSurfaceVariant
                else MaterialTheme.colorScheme.primary,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (chip in availableChips) {
                val selected = query.filters.contains(chip)
                DropdownMenuItem(
                    text = { Text(chip.label) },
                    leadingIcon = if (selected) {
                        { Icon(halogenIcon("checkmark"), contentDescription = null) }
                    } else null,
                    onClick = {
                        open = false
                        onQueryChange(
                            query.copy(
                                filters = if (selected) query.filters - chip
                                else query.filters + chip))
                    },
                )
            }
            if (query.filters.isNotEmpty()) {
                HorizontalDivider()
                DropdownMenuItem(
                    text = { Text("Clear filters") },
                    onClick = {
                        open = false
                        onQueryChange(query.copy(filters = emptySet()))
                    },
                )
            }
        }
    }
}

@Composable
private fun OrderMenuButton(
    query: ListQuery,
    onQueryChange: (ListQuery) -> Unit,
    allowsPosition: Boolean,
    allowsRecency: Boolean,
) {
    var open by remember { mutableStateOf(false) }
    val orderFields = EpisodeOrderField.entries.filter {
        (allowsPosition || it != EpisodeOrderField.Position) &&
            (allowsRecency || it != EpisodeOrderField.Recency)
    }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(
                halogenIcon("arrow.up.arrow.down.circle"),
                contentDescription = "Order",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (field in orderFields) {
                DropdownMenuItem(
                    text = { Text(field.label) },
                    leadingIcon = if (query.orderField == field) {
                        { Icon(halogenIcon("checkmark"), contentDescription = null) }
                    } else null,
                    onClick = {
                        open = false
                        onQueryChange(query.copy(orderField = field))
                    },
                )
            }
            HorizontalDivider()
            val asc = query.direction == OrderDirection.Asc
            DropdownMenuItem(
                text = { Text(if (asc) "Ascending" else "Descending") },
                leadingIcon = {
                    Icon(
                        halogenIcon(if (asc) "arrow.up" else "arrow.down"),
                        contentDescription = null)
                },
                onClick = {
                    open = false
                    onQueryChange(
                        query.copy(
                            direction = if (asc) OrderDirection.Desc else OrderDirection.Asc))
                },
            )
        }
    }
}

/// The generic sort+search sub-navbar for NON-episode lists (podcasts,
/// playlists) — the web's shared `SortSearchControls` wrapper: a search field
/// plus a field/direction menu over a page-supplied field vocabulary.
@Composable
fun <Field> SortSearchBar(
    search: String,
    onSearchChange: (String) -> Unit,
    field: Field,
    onFieldChange: (Field) -> Unit,
    direction: OrderDirection,
    onDirectionChange: (OrderDirection) -> Unit,
    /// Sort fields in menu order: (value, label).
    fields: List<Pair<Field, String>>,
    placeholder: String = "Search",
) {
    var open by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainerLow)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SearchField(
                value = search,
                onValueChange = onSearchChange,
                placeholder = placeholder,
                modifier = Modifier.weight(1f),
            )
            Box {
                IconButton(onClick = { open = true }) {
                    Icon(
                        halogenIcon("arrow.up.arrow.down.circle"),
                        contentDescription = "Order",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                    for ((value, label) in fields) {
                        DropdownMenuItem(
                            text = { Text(label) },
                            leadingIcon = if (field == value) {
                                { Icon(halogenIcon("checkmark"), contentDescription = null) }
                            } else null,
                            onClick = {
                                open = false
                                onFieldChange(value)
                            },
                        )
                    }
                    HorizontalDivider()
                    val asc = direction == OrderDirection.Asc
                    DropdownMenuItem(
                        text = { Text(if (asc) "Ascending" else "Descending") },
                        leadingIcon = {
                            Icon(
                                halogenIcon(if (asc) "arrow.up" else "arrow.down"),
                                contentDescription = null)
                        },
                        onClick = {
                            open = false
                            onDirectionChange(if (asc) OrderDirection.Desc else OrderDirection.Asc)
                        },
                    )
                }
            }
        }
        HorizontalDivider()
    }
}

/// The rounded inline search field both bars share (iOS secondarySystemFill
/// capsule): magnifier + plain field + clear button when non-empty.
@Composable
private fun SearchField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .clip(RoundedCornerShape(9.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 10.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            halogenIcon("magnifyingglass"),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(18.dp),
        )
        Box(Modifier.weight(1f)) {
            if (value.isEmpty()) {
                Text(
                    placeholder,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyMedium
                    .copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (value.isNotEmpty()) {
            Icon(
                halogenIcon("xmark.circle.fill"),
                contentDescription = "Clear search",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(18.dp).clickable { onValueChange("") },
            )
        }
    }
}
