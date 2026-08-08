package org.fgsec.halogen.networking

import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException
import java.util.Locale
import kotlinx.serialization.json.Json

/// JSON coding configured for the wire contract: chrono serializes RFC3339
/// with fractional seconds (whole seconds when the fraction is exactly zero).
/// Wire types keep dates as `String`; parse/format through the helpers here.
object WireJson {
    val json: Json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
    }

    private val fractional: DateTimeFormatter =
        DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSSXXX", Locale.US)
            .withZone(ZoneOffset.UTC)

    /// Parse an RFC3339 timestamp, fractional seconds and offsets included.
    fun parseInstant(raw: String): Instant =
        try {
            java.time.OffsetDateTime.parse(raw).toInstant()
        } catch (e: DateTimeParseException) {
            try {
                Instant.parse(raw)
            } catch (_: DateTimeParseException) {
                throw IllegalArgumentException("Unparseable RFC3339 date: $raw", e)
            }
        }

    /// Format as RFC3339 UTC with millisecond fraction (matches the iOS encoder).
    fun formatInstant(instant: Instant): String = fractional.format(instant)
}
