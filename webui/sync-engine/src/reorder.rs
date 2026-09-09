//! Pure id-reordering used by the optimistic playlist mutators — the testable
//! core of [`SyncService::move_in_playlist_locally`](super::SyncService), in its
//! own module so the clamp logic (which must match the server's, so a FIFO outbox
//! replay reproduces the optimistic order) is unit-tested in isolation.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use halogen_wire::{OrderDirection, PlaylistReorderField, cmp_opt};

/// Reorder `episode_id` to index `to` within `ids` (clamped to range). No-op if
/// the id is absent or already at `to`.
pub(super) fn reorder_ids(ids: &mut Vec<i32>, episode_id: i32, to: i32) {
    let Some(from) = ids.iter().position(|id| *id == episode_id) else {
        return;
    };
    let to = (to.max(0) as usize).min(ids.len().saturating_sub(1));
    if to == from {
        return;
    }
    let id = ids.remove(from);
    ids.insert(to, id);
}

/// The episode fields a smart reorder sorts by, pulled out of the pool so the sort
/// is testable without constructing a whole `EpisodeData`.
pub(super) struct ReorderKeys {
    pub published_at: Option<DateTime<Utc>>,
    pub title: String,
    pub duration_secs: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// Sort by field/direction with missing IDs/values always last and ID tie-breaks. Added order approximates unavailable
/// pivot created_at with episode created_at; later server sync reconciles the difference.
pub(super) fn sort_ids_by(
    ids: &mut [i32],
    field: PlaylistReorderField,
    direction: OrderDirection,
    keys: impl Fn(i32) -> Option<ReorderKeys>,
) {
    let map: HashMap<i32, ReorderKeys> = ids
        .iter()
        .filter_map(|&id| keys(id).map(|k| (id, k)))
        .collect();
    ids.sort_by(|a, b| {
        let ka = map.get(a);
        let kb = map.get(b);
        let ord = match field {
            PlaylistReorderField::Published => cmp_opt(
                ka.and_then(|k| k.published_at),
                kb.and_then(|k| k.published_at),
                &direction,
            ),
            PlaylistReorderField::Duration => cmp_opt(
                ka.and_then(|k| k.duration_secs),
                kb.and_then(|k| k.duration_secs),
                &direction,
            ),
            PlaylistReorderField::Title => cmp_opt(
                ka.map(|k| k.title.to_lowercase()),
                kb.map(|k| k.title.to_lowercase()),
                &direction,
            ),
            PlaylistReorderField::Added => cmp_opt(
                ka.map(|k| k.created_at),
                kb.map(|k| k.created_at),
                &direction,
            ),
        };
        ord.then(a.cmp(b))
    });
}

#[cfg(test)]
mod tests {
    use super::{ReorderKeys, reorder_ids, sort_ids_by};
    use chrono::{DateTime, Utc};
    use halogen_wire::{OrderDirection, PlaylistReorderField};

    #[test]
    fn reorder_moves_to_front_and_back() {
        let mut ids = vec![10, 20, 30];
        reorder_ids(&mut ids, 30, 0);
        assert_eq!(ids, vec![30, 10, 20]);
        reorder_ids(&mut ids, 30, 2);
        assert_eq!(ids, vec![10, 20, 30]);
    }

    #[test]
    fn reorder_clamps_out_of_range_to_last() {
        let mut ids = vec![10, 20, 30];
        reorder_ids(&mut ids, 10, 99);
        assert_eq!(ids, vec![20, 30, 10]);
    }

    #[test]
    fn reorder_noop_on_unchanged_or_absent() {
        let mut ids = vec![10, 20, 30];
        reorder_ids(&mut ids, 20, 1); // already at index 1
        assert_eq!(ids, vec![10, 20, 30]);
        reorder_ids(&mut ids, 999, 0); // absent
        assert_eq!(ids, vec![10, 20, 30]);
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    // 1: published 100, "Charlie", 300s, added 10
    // 2: published 300, "alpha",   100s, added 20
    // 3: published —,   "Bravo",   —,    added 30
    fn keys(id: i32) -> Option<ReorderKeys> {
        Some(match id {
            1 => ReorderKeys {
                published_at: Some(ts(100)),
                title: "Charlie".into(),
                duration_secs: Some(300),
                created_at: ts(10),
            },
            2 => ReorderKeys {
                published_at: Some(ts(300)),
                title: "alpha".into(),
                duration_secs: Some(100),
                created_at: ts(20),
            },
            3 => ReorderKeys {
                published_at: None,
                title: "Bravo".into(),
                duration_secs: None,
                created_at: ts(30),
            },
            _ => return None,
        })
    }

    fn sorted(field: PlaylistReorderField, dir: OrderDirection) -> Vec<i32> {
        let mut ids = vec![3, 1, 2];
        sort_ids_by(&mut ids, field, dir, keys);
        ids
    }

    #[test]
    fn sort_by_published_nulls_last_both_directions() {
        use OrderDirection::*;
        use PlaylistReorderField::Published;
        assert_eq!(sorted(Published, Asc), vec![1, 2, 3]);
        // Desc flips the present values but the null (3) still sorts last.
        assert_eq!(sorted(Published, Desc), vec![2, 1, 3]);
    }

    #[test]
    fn sort_by_title_is_case_insensitive() {
        use OrderDirection::Asc;
        use PlaylistReorderField::Title;
        // alpha, Bravo, Charlie — not ASCII order (which would put the capitals first).
        assert_eq!(sorted(Title, Asc), vec![2, 3, 1]);
    }

    #[test]
    fn sort_by_duration_nulls_last() {
        use OrderDirection::Asc;
        use PlaylistReorderField::Duration;
        assert_eq!(sorted(Duration, Asc), vec![2, 1, 3]);
    }

    #[test]
    fn sort_by_added_uses_created_at() {
        use OrderDirection::*;
        use PlaylistReorderField::Added;
        assert_eq!(sorted(Added, Asc), vec![1, 2, 3]);
        assert_eq!(sorted(Added, Desc), vec![3, 2, 1]);
    }

    #[test]
    fn sort_missing_episode_sorts_last() {
        // Id 99 has no keys → sorts after the resolvable ids regardless of field.
        let mut ids = vec![99, 2, 1];
        sort_ids_by(
            &mut ids,
            PlaylistReorderField::Title,
            OrderDirection::Asc,
            keys,
        );
        assert_eq!(ids, vec![2, 1, 99]);
    }
}
