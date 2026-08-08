package org.fgsec.halogen.components

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/// Single source of color truth — no hardcoded colors outside this file.
/// Dark-first: the canonical look is the iOS app's near-black blue ink.

// Core ink (matches res/values/colors.xml window_background).
private val Ink = Color(0xFF0F1115)

private val DarkColors = darkColorScheme(
    primary = Color(0xFF0A84FF),
    onPrimary = Color.White,
    primaryContainer = Color(0xFF0A3D6E),
    onPrimaryContainer = Color(0xFFCCE4FF),
    secondary = Color(0xFF8E9AAB),
    onSecondary = Color(0xFF11151B),
    secondaryContainer = Color(0xFF232833),
    onSecondaryContainer = Color(0xFFD8DDE6),
    tertiary = Color(0xFF64D2FF),
    onTertiary = Color(0xFF00344A),
    background = Ink,
    onBackground = Color(0xFFF2F3F5),
    surface = Ink,
    onSurface = Color(0xFFF2F3F5),
    surfaceVariant = Color(0xFF1A1E26),
    onSurfaceVariant = Color(0xFF9AA0AB),
    surfaceContainerLowest = Color(0xFF0B0D10),
    surfaceContainerLow = Color(0xFF14171D),
    surfaceContainer = Color(0xFF181C23),
    surfaceContainerHigh = Color(0xFF1F242D),
    surfaceContainerHighest = Color(0xFF262C37),
    error = Color(0xFFFF453A),
    onError = Color.White,
    errorContainer = Color(0xFF5C1614),
    onErrorContainer = Color(0xFFFFD9D6),
    outline = Color(0xFF3A404C),
    outlineVariant = Color(0xFF272C35),
    inverseSurface = Color(0xFFE8EAEE),
    inverseOnSurface = Color(0xFF191C22),
    inversePrimary = Color(0xFF0060C2),
)

private val LightColors = lightColorScheme(
    primary = Color(0xFF007AFF),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFD6E8FF),
    onPrimaryContainer = Color(0xFF00294D),
    secondary = Color(0xFF5D6673),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFE4E8EF),
    onSecondaryContainer = Color(0xFF262C35),
    tertiary = Color(0xFF0071A4),
    onTertiary = Color.White,
    background = Color.White,
    onBackground = Color(0xFF111318),
    surface = Color.White,
    onSurface = Color(0xFF111318),
    surfaceVariant = Color(0xFFF2F2F7),
    onSurfaceVariant = Color(0xFF6E7480),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color(0xFFF7F7FA),
    surfaceContainer = Color(0xFFF2F2F7),
    surfaceContainerHigh = Color(0xFFECECF1),
    surfaceContainerHighest = Color(0xFFE5E5EA),
    error = Color(0xFFFF3B30),
    onError = Color.White,
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
    outline = Color(0xFFC6C6C8),
    outlineVariant = Color(0xFFE5E5EA),
    inverseSurface = Color(0xFF2E3138),
    inverseOnSurface = Color(0xFFF0F1F4),
    inversePrimary = Color(0xFF8FC2FF),
)

/// Semantic colors with no ColorScheme slot (iOS fills, toast surfaces).
@Immutable
data class HalogenExtraColors(
    /// iOS quaternary fill — artwork/placeholder tiles.
    val placeholderFill: Color,
    /// iOS secondarySystemBackground — non-error toast capsules.
    val toastBackground: Color,
    /// iOS systemGreen — success badges (poll-job Completed).
    val success: Color,
    /// iOS systemOrange — warning badges (WARN log lines).
    val warning: Color,
)

private val DarkExtras = HalogenExtraColors(
    placeholderFill = Color(0x14FFFFFF),
    toastBackground = Color(0xFF1F242D),
    success = Color(0xFF30D158),
    warning = Color(0xFFFF9F0A),
)

private val LightExtras = HalogenExtraColors(
    placeholderFill = Color(0x14000000),
    toastBackground = Color(0xFFF2F2F7),
    success = Color(0xFF34C759),
    warning = Color(0xFFFF9500),
)

val LocalHalogenExtras = staticCompositionLocalOf { DarkExtras }

/// Accessor for the extra palette inside HalogenTheme content.
val halogenExtras: HalogenExtraColors
    @Composable get() = LocalHalogenExtras.current

// Typography tuned to the iOS text-style ladder (body 17 / callout 16 /
// footnote 13 / caption 12) on Material 3 slots.
private val BaseTypography = Typography()
val HalogenTypography = BaseTypography.copy(
    titleLarge = BaseTypography.titleLarge.copy(
        fontSize = 22.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = BaseTypography.titleMedium.copy(
        fontSize = 17.sp, fontWeight = FontWeight.SemiBold),
    bodyLarge = BaseTypography.bodyLarge.copy(fontSize = 17.sp),
    bodyMedium = BaseTypography.bodyMedium.copy(fontSize = 16.sp),
    bodySmall = BaseTypography.bodySmall.copy(fontSize = 13.sp, lineHeight = 18.sp),
    labelMedium = BaseTypography.labelMedium.copy(fontSize = 15.sp),
    labelSmall = BaseTypography.labelSmall.copy(fontSize = 12.sp, lineHeight = 16.sp),
)

@Composable
fun HalogenTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    CompositionLocalProvider(
        LocalHalogenExtras provides if (darkTheme) DarkExtras else LightExtras
    ) {
        MaterialTheme(
            colorScheme = if (darkTheme) DarkColors else LightColors,
            typography = HalogenTypography,
            content = content,
        )
    }
}
