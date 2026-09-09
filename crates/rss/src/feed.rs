//! Parsing RSS feeds into [`RemoteFeedData`] / [`RemoteEpisodeData`].

use std::str::FromStr;

use chrono::Utc;
use rss::extension::Extension;
use rss::{Channel, Enclosure, Guid, Item};

use super::types::{RemoteChapter, RemoteEpisodeData, RemoteFeedData};

/// Parses RSS XML into channel art + episodes.
pub fn parse_feed(xml: &str) -> anyhow::Result<RemoteFeedData> {
    let channel: Channel = Channel::from_str(xml)?;
    let art_url = channel
        .itunes_ext()
        .and_then(|ext| ext.image().map(|s: &str| s.to_string()))
        .or_else(|| channel.image().map(|img| img.url().to_string()));
    let channel_title = Some(channel.title().trim().to_string()).filter(|t| !t.is_empty());
    Ok(RemoteFeedData {
        art_url,
        channel_title,
        episodes: channel.items().iter().map(to_remote_episode).collect(),
    })
}

/// Parses RSS XML content into a list of episodes. Test-only convenience over
/// [`parse_feed`] — production code wants the channel art too, so it calls
/// `parse_feed` directly.
#[cfg(test)]
pub(crate) fn parse_rss(xml: &str) -> anyhow::Result<Vec<RemoteEpisodeData>> {
    parse_feed(xml).map(|f| f.episodes)
}

fn to_remote_episode(item: &Item) -> RemoteEpisodeData {
    let description = item
        .description()
        .map(|s: &str| s.to_string())
        .or_else(|| item.content().map(|s: &str| s.to_string()))
        .or_else(|| Some("".to_string()));

    let content_url = item
        .enclosure()
        .map(|e: &Enclosure| e.url().to_string())
        .or_else(|| item.link().map(|s: &str| s.to_string()));

    // Art comes from the itunes image, falling back to an IMAGE enclosure only.
    // Never the audio enclosure: `art_url` feeds `<img>` tags (via the server
    // art cache), and a past `audio/*` fallback here made browsers download
    // whole MP3s from origin CDNs just to fail rendering them as images.
    let art_url = item
        .itunes_ext()
        .and_then(|ext| ext.image().map(|s: &str| s.to_string()))
        .or_else(|| {
            item.enclosure()
                .filter(|e: &&Enclosure| e.mime_type().starts_with("image/"))
                .map(|e: &Enclosure| e.url().to_string())
        });

    let published_at = item
        .pub_date()
        .and_then(|s: &str| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let duration_secs = item
        .itunes_ext()
        .and_then(|ext| ext.duration())
        .and_then(parse_npt_seconds);

    let guid = item.guid().map(|g: &Guid| g.value().to_string());

    // Chapters: prefer inline `psc:chapters` (free); only when there are none do
    // we record a `podcast:chapters` URL for the best-effort fetch during ingest.
    let chapters = parse_psc_chapters(item);
    let chapters_url = if chapters.is_empty() {
        parse_podcast_chapters_url(item)
    } else {
        None
    };

    RemoteEpisodeData {
        title: item.title().map(|s| s.to_string()).unwrap_or_default(),
        description,
        content_url: content_url.unwrap_or_default(),
        art_url,
        published_at,
        duration_secs,
        guid,
        chapters,
        chapters_url,
    }
}

/// Parse inline `psc:chapters` (Podlove Simple Chapters) off a feed item. `rss` exposes namespaced elements
/// via `item.extensions()` keyed by prefix → local name. `<psc:chapters>` lands at
/// `extensions()["psc"]["chapters"]`; each `<psc:chapter start=… title=…/>` is a child carrying those attrs. We
/// iterate child values rather than assume the child key, then read `start`/`title`.
fn parse_psc_chapters(item: &Item) -> Vec<RemoteChapter> {
    let Some(psc) = item.extensions().get("psc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for wrapper in psc.values().flatten() {
        for chapter in wrapper.children().values().flatten() {
            if let Some(parsed) = psc_chapter(chapter) {
                out.push(parsed);
            }
        }
    }
    out
}

/// Read one `<psc:chapter>` element's `start` (NPT) + `title` attrs. Skips a
/// chapter missing either, or whose `start` doesn't parse.
fn psc_chapter(ext: &Extension) -> Option<RemoteChapter> {
    let attrs = ext.attrs();
    let title = attrs.get("title")?;
    let starts_at_secs = parse_npt_seconds(attrs.get("start")?)?;
    Some(RemoteChapter {
        title: title.clone(),
        starts_at_secs,
    })
}

/// Extract a `podcast:chapters` external JSON URL (Podcasting 2.0) off a feed item: `<podcast:chapters url=…
/// type="application/json+chapters"/>` → `extensions()["podcast"]["chapters"]`. Keyed on the `chapters` element
/// so we never confuse it with other `podcast:` elements (transcript, person, …) that also carry a `url`.
/// Accepts a missing `type`; otherwise requires it to name JSON.
fn parse_podcast_chapters_url(item: &Item) -> Option<String> {
    let chapters = item.extensions().get("podcast")?.get("chapters")?;
    chapters.iter().find_map(|ext| {
        let attrs = ext.attrs();
        let url = attrs.get("url").filter(|u| !u.is_empty())?;
        let type_ok = attrs.get("type").is_none_or(|t| t.contains("json"));
        type_ok.then(|| url.clone())
    })
}

/// Parse a Normal Play Time value (shared by `<itunes:duration>` and the `psc:chapter` `start` attribute) into
/// whole seconds. Accepts a plain seconds count (`"3600"`), `MM:SS` (`"62:03"`), or `HH:MM:SS` (`"1:02:03"`),
/// each with an optional fractional `.mmm` suffix (truncated — we store whole seconds). Returns `None` for
/// empty/garbage so callers can skip the value.
pub(crate) fn parse_npt_seconds(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Drop a fractional-seconds suffix on the final component (`839.5`,
    // `00:13:39.500`); whole seconds is all we keep.
    let s = s.split('.').next().unwrap_or(s);
    if s.is_empty() {
        return None;
    }
    if let Ok(secs) = s.parse::<i64>() {
        return i32::try_from(secs.max(0)).ok();
    }
    let mut total: i64 = 0;
    for part in s.split(':') {
        let n: i64 = part.trim().parse().ok()?;
        if n < 0 {
            return None;
        }
        total = total * 60 + n;
    }
    i32::try_from(total).ok()
}
