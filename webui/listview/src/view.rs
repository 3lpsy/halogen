//! Episode-list view state. The data DTOs (`EpisodeData`, `PlaybackData`, …) live in `halogen_wire` (wasm-safe, no `db`
//! feature); `EpisodeData` is re-exported here so the component and the rest of the UI share one definition. Everything
//! else below is *view state*, sort/filter/swipe UI controls, which is intentionally UI-only.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use halogen_wire::EpisodeData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    /// Manual order — the playlist pivot's `position`. Only meaningful for
    /// `Playlist` sources; rendered as the raw membership Vec order.
    Custom,
    PublishedAt,
    Title,
    CreatedAt,
    UpdatedAt,
    Duration,
}

impl SortField {
    /// Stable serialization token (URL query + sticky storage). NOT the display
    /// label — labels can change without breaking saved/shared state.
    pub fn token(self) -> &'static str {
        match self {
            SortField::Custom => "custom",
            SortField::PublishedAt => "published",
            SortField::Title => "title",
            SortField::CreatedAt => "added",
            SortField::UpdatedAt => "updated",
            SortField::Duration => "duration",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "custom" => SortField::Custom,
            "published" => SortField::PublishedAt,
            "title" => SortField::Title,
            "added" => SortField::CreatedAt,
            "updated" => SortField::UpdatedAt,
            "duration" => SortField::Duration,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Asc,
    Desc,
}

impl OrderDirection {
    pub fn token(self) -> &'static str {
        match self {
            OrderDirection::Asc => "asc",
            OrderDirection::Desc => "desc",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "asc" => OrderDirection::Asc,
            "desc" => OrderDirection::Desc,
            _ => return None,
        })
    }
}

/// Filter rows, sort with the supplied ascending comparator, and reverse for descending. Collection lists share this
/// path; episode lists instead sort IDs with a render-time reverse flag.
pub fn search_sort<T>(
    mut rows: Vec<T>,
    direction: OrderDirection,
    keep: impl Fn(&T) -> bool,
    cmp: impl Fn(&T, &T) -> std::cmp::Ordering,
) -> Vec<T> {
    rows.retain(|r| keep(r));
    rows.sort_by(|a, b| {
        let ord = cmp(a, b);
        match direction {
            OrderDirection::Asc => ord,
            OrderDirection::Desc => ord.reverse(),
        }
    });
    rows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
    pub field: SortField,
    pub direction: OrderDirection,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            field: SortField::PublishedAt,
            direction: OrderDirection::Desc,
        }
    }
}

/// A single multi-select filter chip. Within a facet (download vs played) the
/// selected chips are OR-ed; across facets they're AND-ed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeFilter {
    /// Downloaded to THIS device (client-download set). A client-only facet —
    /// applied locally, not server-side.
    OnDevice,
    Downloaded,
    Downloading,
    /// Never started — `PlaybackStatus::Unplayed`.
    Unplayed,
    /// Started but not finished — `PlaybackStatus::Played`.
    Played,
    /// Listened to the end — `PlaybackStatus::Finished`.
    Finished,
}

impl EpisodeFilter {
    /// Stable serialization token (URL query + sticky storage).
    pub fn token(self) -> &'static str {
        match self {
            EpisodeFilter::OnDevice => "ondevice",
            EpisodeFilter::Downloaded => "downloaded",
            EpisodeFilter::Downloading => "downloading",
            EpisodeFilter::Unplayed => "unplayed",
            EpisodeFilter::Played => "played",
            EpisodeFilter::Finished => "finished",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "ondevice" => EpisodeFilter::OnDevice,
            "downloaded" => EpisodeFilter::Downloaded,
            "downloading" => EpisodeFilter::Downloading,
            "unplayed" => EpisodeFilter::Unplayed,
            "played" => EpisodeFilter::Played,
            "finished" => EpisodeFilter::Finished,
            _ => return None,
        })
    }
}

// `PartialEq` so it's a sound inner type for the `filter: Signal<FilterSpec>` prop
// (Dioxus value-compares signal-typed props; a non-`PartialEq` inner assumes
// "changed" and can churn). Matches sibling `SortSpec`/`ListViewState`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterSpec {
    pub search: Option<String>,
    pub podcast_id: Option<i32>,
    pub published_after: Option<DateTime<Utc>>,
    /// Active filter chips (empty = no constraint).
    pub filters: Vec<EpisodeFilter>,
}

impl FilterSpec {
    /// Report whether search, chips, dates, or podcast restrictions hide rows. Manual reorder must then be disabled
    /// because rendered indices no longer match membership positions or the descending mirror axis.
    pub fn is_narrowing(&self) -> bool {
        self.search.as_ref().is_some_and(|s| !s.trim().is_empty())
            || self.podcast_id.is_some()
            || self.published_after.is_some()
            || !self.filters.is_empty()
    }
}

/// The persistable slice of a list's view state: sort + filter chips + search.
/// `podcast_id` / `published_after` are intentionally excluded — those come from
/// the route/page, not from saved or shared state, so they're re-applied by the
/// page and merged onto whatever this restores.
#[derive(Debug, Clone, PartialEq)]
pub struct ListViewState {
    pub sort: SortSpec,
    pub filters: Vec<EpisodeFilter>,
    pub search: Option<String>,
}

impl ListViewState {
    /// Serialize to a compact query string (no leading `?`), e.g.
    /// `sort=published.desc&filter=ondevice,unplayed&q=foo`.
    // Only the wasm URL-state path (`write_url_state`) + tests call this; the native
    // bin compiles the no-op URL stubs, so it's legitimately unused there.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn to_query(&self) -> String {
        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        ser.append_pair(
            "sort",
            &format!(
                "{}.{}",
                self.sort.field.token(),
                self.sort.direction.token()
            ),
        );
        if !self.filters.is_empty() {
            let tokens: Vec<&str> = self.filters.iter().map(|f| f.token()).collect();
            ser.append_pair("filter", &tokens.join(","));
        }
        if let Some(q) = self.search.as_ref().filter(|s| !s.is_empty()) {
            ser.append_pair("q", q);
        }
        ser.finish()
    }

    /// Parse a query string (no leading `?`). Lenient: unknown tokens are dropped,
    /// missing `sort` falls back to default. Returns `None` when nothing relevant
    /// is present (so callers can fall through to sticky storage / page defaults).
    // Only the wasm URL-state path (`read_url_state`) + tests call this; the native
    // bin compiles the no-op URL stubs, so it's legitimately unused there.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn from_query(qs: &str) -> Option<Self> {
        let mut sort: Option<SortSpec> = None;
        let mut filters: Vec<EpisodeFilter> = Vec::new();
        let mut search: Option<String> = None;
        let mut saw_key = false;
        for (k, v) in url::form_urlencoded::parse(qs.as_bytes()) {
            match k.as_ref() {
                "sort" => {
                    saw_key = true;
                    let mut parts = v.splitn(2, '.');
                    let field = parts.next().and_then(SortField::from_token);
                    let dir = parts.next().and_then(OrderDirection::from_token);
                    if let (Some(field), Some(direction)) = (field, dir) {
                        sort = Some(SortSpec { field, direction });
                    }
                }
                "filter" => {
                    saw_key = true;
                    filters = v.split(',').filter_map(EpisodeFilter::from_token).collect();
                }
                "q" => {
                    saw_key = true;
                    if !v.is_empty() {
                        search = Some(v.into_owned());
                    }
                }
                _ => {}
            }
        }
        if !saw_key {
            return None;
        }
        Some(Self {
            sort: sort.unwrap_or_default(),
            filters,
            search,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListSource {
    AllEpisodes,
    Playlist {
        id: i32,
    },
    History,
    ClientDownloads,
    /// Episodes the SERVER holds (downloaded or downloading) — the Downloads
    /// page's source in local-runtime mode, where the server's library *is*
    /// this device's library and the device-download concept doesn't exist.
    ServerDownloads,
}

impl ListSource {
    /// Sources whose membership is an id list resolved from LOCAL state (no sparse server paging): playlists, both
    /// downloads sets, history. EXTEND THIS when adding a variant, the list component's mode gate reads it, and a
    /// variant missing here silently takes the server-paged path while the id-list branch's snapshot gates go
    /// inconsistent (the exact bug that shipped with `ServerDownloads` and panicked the Downloads page).
    pub fn is_id_list(&self) -> bool {
        matches!(
            self,
            ListSource::Playlist { .. }
                | ListSource::ClientDownloads
                | ListSource::ServerDownloads
                | ListSource::History
        )
    }
}

/// Multiselect (bulk-action) view state for one `EpisodeList`. Page-local, it lives only while the list is mounted and
/// is never persisted: selection is a transient interaction, not saved/shared state. `selected` survives filtering and
/// sorting: an episode hidden by a filter stays in the set (and reappears selected when the filter is removed).
/// `selected_only` is the "Selected" review chip, it restricts the list to the selected set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MultiSelectState {
    /// Whether multiselect mode is on (checkboxes shown, swipe/PTR/reorder off).
    pub active: bool,
    /// The chosen episode ids.
    pub selected: std::collections::HashSet<i32>,
    /// Show only the selected episodes (the "Selected" chip).
    pub selected_only: bool,
}

/// A contextual action bound to one swipe direction on an episode row. The set is
/// broad and user-configurable per page (see `SwipePrefs`); each row resolves the
/// action against its own live state (played / queued / downloaded) in
/// `EpisodeListItem`. `None` on a `SwipeConfig` side disables that swipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwipeAction {
    Play,
    Stream,
    TogglePlayed,
    MarkPlayed,
    MarkUnplayed,
    ResetProgress,
    AddToQueue,
    RemoveFromQueue,
    ToggleQueue,
    /// Remove from *this list's* playlist (queue or the viewed playlist).
    RemoveFromList,
    AddToPlaylist,
    DownloadToDevice,
    /// Force-fresh device copy (remove + re-pull).
    RedownloadDevice,
    RemoveDownload,
    ToggleDownload,
    DownloadOnServer,
    RemoveFromServer,
    ToggleServerDownload,
}

impl SwipeAction {
    /// Every variant, in menu order (also drives the configure-page dropdowns).
    pub const ALL: [SwipeAction; 18] = [
        SwipeAction::Play,
        SwipeAction::Stream,
        SwipeAction::TogglePlayed,
        SwipeAction::MarkPlayed,
        SwipeAction::MarkUnplayed,
        SwipeAction::ResetProgress,
        SwipeAction::AddToQueue,
        SwipeAction::RemoveFromQueue,
        SwipeAction::ToggleQueue,
        SwipeAction::RemoveFromList,
        SwipeAction::AddToPlaylist,
        SwipeAction::DownloadToDevice,
        SwipeAction::RedownloadDevice,
        SwipeAction::RemoveDownload,
        SwipeAction::ToggleDownload,
        SwipeAction::DownloadOnServer,
        SwipeAction::RemoveFromServer,
        SwipeAction::ToggleServerDownload,
    ];

    /// Stable identifier (serde name) for `<select>` values.
    pub fn as_str(&self) -> &'static str {
        match self {
            SwipeAction::Play => "Play",
            SwipeAction::Stream => "Stream",
            SwipeAction::TogglePlayed => "TogglePlayed",
            SwipeAction::MarkPlayed => "MarkPlayed",
            SwipeAction::MarkUnplayed => "MarkUnplayed",
            SwipeAction::ResetProgress => "ResetProgress",
            SwipeAction::AddToQueue => "AddToQueue",
            SwipeAction::RemoveFromQueue => "RemoveFromQueue",
            SwipeAction::ToggleQueue => "ToggleQueue",
            SwipeAction::RemoveFromList => "RemoveFromList",
            SwipeAction::AddToPlaylist => "AddToPlaylist",
            SwipeAction::DownloadToDevice => "DownloadToDevice",
            SwipeAction::RedownloadDevice => "RedownloadDevice",
            SwipeAction::RemoveDownload => "RemoveDownload",
            SwipeAction::ToggleDownload => "ToggleDownload",
            SwipeAction::DownloadOnServer => "DownloadOnServer",
            SwipeAction::RemoveFromServer => "RemoveFromServer",
            SwipeAction::ToggleServerDownload => "ToggleServerDownload",
        }
    }

    /// Parse a `<select>` value back to an action; `None` for "none"/unknown.
    pub fn from_token(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// Short human label — shown in the swipe reveal strip and the configure page.
    pub fn label(&self) -> &'static str {
        match self {
            SwipeAction::Play => "Play",
            SwipeAction::Stream => "Stream",
            SwipeAction::TogglePlayed => "Toggle played",
            SwipeAction::MarkPlayed => "Mark played",
            SwipeAction::MarkUnplayed => "Mark unplayed",
            SwipeAction::ResetProgress => "Reset progress",
            SwipeAction::AddToQueue => "Add to queue",
            SwipeAction::RemoveFromQueue => "Remove from queue",
            SwipeAction::ToggleQueue => "Toggle queue",
            SwipeAction::RemoveFromList => "Remove from list",
            SwipeAction::AddToPlaylist => "Add to playlist",
            SwipeAction::DownloadToDevice => "Download",
            SwipeAction::RedownloadDevice => "Re-download",
            SwipeAction::RemoveDownload => "Remove download",
            SwipeAction::ToggleDownload => "Toggle download",
            SwipeAction::DownloadOnServer => "Download on server",
            SwipeAction::RemoveFromServer => "Remove from server",
            SwipeAction::ToggleServerDownload => "Toggle server download",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SwipeConfig {
    #[serde(default)]
    pub left: Option<SwipeAction>,
    #[serde(default)]
    pub right: Option<SwipeAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemVariant {
    /// No playback progress bar. Currently unused (every list opts into
    /// `WithProgress`), but kept for lists that may want a bare row.
    #[allow(dead_code)]
    Default,
    WithProgress,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_spec_is_narrowing_detects_any_active_constraint() {
        // Default (no constraint) does not narrow → reorder stays enabled.
        assert!(!FilterSpec::default().is_narrowing());
        // A blank/whitespace search doesn't count.
        assert!(
            !FilterSpec {
                search: Some("   ".to_string()),
                ..Default::default()
            }
            .is_narrowing()
        );
        // Any real search / chip / podcast / date bound narrows the set.
        assert!(
            FilterSpec {
                search: Some("foo".to_string()),
                ..Default::default()
            }
            .is_narrowing()
        );
        assert!(
            FilterSpec {
                filters: vec![EpisodeFilter::Unplayed],
                ..Default::default()
            }
            .is_narrowing()
        );
        assert!(
            FilterSpec {
                podcast_id: Some(7),
                ..Default::default()
            }
            .is_narrowing()
        );
    }

    #[test]
    fn list_view_state_round_trips_through_query() {
        let state = ListViewState {
            sort: SortSpec {
                field: SortField::Title,
                direction: OrderDirection::Asc,
            },
            filters: vec![EpisodeFilter::OnDevice, EpisodeFilter::Unplayed],
            search: Some("rust news".into()),
        };
        let qs = state.to_query();
        let back = ListViewState::from_query(&qs).expect("parses");
        assert_eq!(back, state);
    }

    #[test]
    fn from_query_is_lenient_about_unknown_tokens_and_missing_sort() {
        // Unknown sort field / filter tokens are dropped; missing sort → default.
        let parsed = ListViewState::from_query("sort=bogus.sideways&filter=ondevice,nope&q=hi")
            .expect("still parses (keys present)");
        assert_eq!(parsed.sort, SortSpec::default());
        assert_eq!(parsed.filters, vec![EpisodeFilter::OnDevice]);
        assert_eq!(parsed.search.as_deref(), Some("hi"));
    }

    #[test]
    fn from_query_none_when_no_relevant_keys() {
        assert!(ListViewState::from_query("").is_none());
        assert!(ListViewState::from_query("foo=bar").is_none());
    }

    #[test]
    fn to_query_omits_empty_filter_and_search() {
        let state = ListViewState {
            sort: SortSpec::default(),
            filters: vec![],
            search: None,
        };
        let qs = state.to_query();
        assert!(qs.contains("sort="), "sort always present: {qs}");
        assert!(!qs.contains("filter="), "empty filter omitted: {qs}");
        assert!(!qs.contains("q="), "empty search omitted: {qs}");
    }

    #[test]
    fn search_with_special_chars_survives_round_trip() {
        let state = ListViewState {
            sort: SortSpec::default(),
            filters: vec![],
            search: Some("a & b = c?d".into()),
        };
        let back = ListViewState::from_query(&state.to_query()).expect("parses");
        assert_eq!(back.search, state.search);
    }
}
