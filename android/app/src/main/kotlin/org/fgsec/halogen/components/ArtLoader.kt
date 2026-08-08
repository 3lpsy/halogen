package org.fgsec.halogen.components

import android.content.Context
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.disk.DiskCache
import coil3.memory.MemoryCache
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import java.io.File
import okhttp3.OkHttpClient
import okio.Path.Companion.toOkioPath
import org.fgsec.halogen.networking.Http

/// Authenticated artwork fetcher with a two-tier cache: Coil memory cache and
/// per-account files on disk — artwork survives offline relaunches (the
/// server's art cache is the durable origin tier). Fetches carry the API
/// bearer explicitly on the shared OkHttpClient.
object ArtLoader {
    @Volatile private var token: String? = null
    @Volatile private var current: ImageLoader? = null

    /// Reconfigure at login/account switch: fresh token + per-account art dir.
    fun configure(context: Context, token: String?, namespace: String? = null) {
        this.token = token
        current?.memoryCache?.clear()
        val dir = namespace?.let { ns ->
            File(context.filesDir, "halogen-client/$ns/art").apply { mkdirs() }
        }
        current = build(context.applicationContext, dir)
    }

    /// The loader for Artwork tiles; an unauthenticated default until configure.
    fun loader(context: PlatformContext): ImageLoader =
        current ?: build(context, dir = null).also { current = it }

    private fun build(context: PlatformContext, dir: File?): ImageLoader =
        ImageLoader.Builder(context)
            .components {
                add(OkHttpNetworkFetcherFactory(callFactory = { authedClient() }))
            }
            .memoryCache { MemoryCache.Builder().maxSizePercent(context, 0.25).build() }
            .diskCache {
                dir?.let { DiskCache.Builder().directory(it.toOkioPath()).build() }
            }
            .build()

    /// Shared client plus the API bearer (Coil can't reach TokenBox itself).
    private fun authedClient(): OkHttpClient =
        Http.client.newBuilder()
            .addInterceptor { chain ->
                val t = token
                val request =
                    if (t != null)
                        chain.request().newBuilder()
                            .header("Authorization", "Bearer $t")
                            .build()
                    else chain.request()
                chain.proceed(request)
            }
            .build()
}
