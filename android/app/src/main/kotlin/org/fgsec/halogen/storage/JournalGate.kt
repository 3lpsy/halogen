package org.fgsec.halogen.storage

import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.sync.Mutex

/** Reopened handles for one profile share the cache/journal publication boundary. */
internal object JournalGate {
    private val gates = ConcurrentHashMap<String, Mutex>()
    fun forPath(path: String): Mutex = gates.computeIfAbsent(path) { Mutex() }
}
