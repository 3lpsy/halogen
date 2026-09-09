package org.fgsec.halogen.storage

import kotlinx.serialization.json.*

internal fun cachePayload(raw: JsonElement): JsonElement =
    if ((raw as? JsonObject)?.get("halogen_cache") == JsonPrimitive(1)) raw.getValue("payload") else raw

internal fun cacheReceipts(raw: JsonElement?): Set<String> =
    if ((raw as? JsonObject)?.get("halogen_cache") == JsonPrimitive(1))
        (raw["applied_journal_ids"] as? JsonArray).orEmpty().map { it.jsonPrimitive.content }.toSet()
    else emptySet()

internal fun cacheEnvelope(payload: JsonElement, receipts: Set<String>): JsonElement =
    if (receipts.isEmpty()) payload else buildJsonObject {
        put("halogen_cache", 1)
        put("payload", payload)
        put("applied_journal_ids", JsonArray(receipts.map(::JsonPrimitive)))
    }
