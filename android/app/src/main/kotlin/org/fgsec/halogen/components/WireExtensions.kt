package org.fgsec.halogen.components

import org.fgsec.halogen.wire.PlaylistReorderField

/// Hand-written helpers for generated wire types (the generator emits plain
/// serializable enums; UI pickers need an explicit, stable case order).
/// CaseIterable parity with iOS WireExtensions.swift.
val PlaylistReorderFieldAllCases: List<PlaylistReorderField> = listOf(
    PlaylistReorderField.Published,
    PlaylistReorderField.Title,
    PlaylistReorderField.Duration,
    PlaylistReorderField.Added,
)
