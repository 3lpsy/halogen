package org.fgsec.halogen.features.latest

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/// One multi-select episode filter chip — mirrors the web's `EpisodeFilter`
/// vocabulary: within a facet selected chips are OR-ed, across facets AND-ed.
/// `OnDevice` is client-only (this device's download set); the rest map onto
/// wire `FilterParams` tokens when a facet has exactly one selected chip.
@Serializable
enum class EpisodeFilter(val rawValue: String) {
    @SerialName("ondevice") OnDevice("ondevice"),
    @SerialName("downloaded") Downloaded("downloaded"),
    @SerialName("downloading") Downloading("downloading"),
    @SerialName("unplayed") Unplayed("unplayed"),
    @SerialName("played") InProgress("played"),
    @SerialName("finished") Finished("finished");

    val label: String
        get() = when (this) {
            OnDevice -> "On Device"
            Downloaded -> "Downloaded"
            Downloading -> "Downloading"
            Unplayed -> "Unplayed"
            InProgress -> "In Progress"
            Finished -> "Finished"
        }

    /// The wire `FilterParams` token for a single-chip facet selection.
    val wireToken: String?
        get() = when (this) {
            OnDevice -> null
            Downloaded -> "DOWNLOADED"
            Downloading -> "DOWNLOADING"
            Unplayed -> "UNPLAYED"
            InProgress -> "PLAYED"
            Finished -> "FINISHED"
        }

    companion object {
        /// The download facet (server download state).
        val downloadFacet = listOf(Downloaded, Downloading)
        /// The played-state facet (the 3-state playback_status column).
        val playedFacet = listOf(Unplayed, InProgress, Finished)
    }
}
