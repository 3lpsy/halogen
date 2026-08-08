package org.fgsec.halogen.components

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.rounded.AccountCircle
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowCircleDown
import androidx.compose.material.icons.rounded.ArrowCircleRight
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.CheckCircleOutline
import androidx.compose.material.icons.rounded.ChevronLeft
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Circle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.CloudDownload
import androidx.compose.material.icons.rounded.CloudOff
import androidx.compose.material.icons.rounded.Contrast
import androidx.compose.material.icons.rounded.Dangerous
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.DownloadForOffline
import androidx.compose.material.icons.rounded.Downloading
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.FilterList
import androidx.compose.material.icons.rounded.FolderOff
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.GridView
import androidx.compose.material.icons.rounded.HelpOutline
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.List
import androidx.compose.material.icons.rounded.Logout
import androidx.compose.material.icons.rounded.ManageAccounts
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.NoteAdd
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PauseCircleOutline
import androidx.compose.material.icons.rounded.Pending
import androidx.compose.material.icons.rounded.People
import androidx.compose.material.icons.rounded.PersonAdd
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.PlayCircleOutline
import androidx.compose.material.icons.rounded.PlaylistAdd
import androidx.compose.material.icons.rounded.PlaylistAddCheck
import androidx.compose.material.icons.rounded.PlaylistRemove
import androidx.compose.material.icons.rounded.Podcasts
import androidx.compose.material.icons.rounded.QueueMusic
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.RemoveCircleOutline
import androidx.compose.material.icons.rounded.Replay
import androidx.compose.material.icons.rounded.RotateLeft
import androidx.compose.material.icons.rounded.RotateRight
import androidx.compose.material.icons.rounded.SaveAlt
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.SdCardAlert
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Smartphone
import androidx.compose.material.icons.rounded.SwapVert
import androidx.compose.material.icons.rounded.UnfoldMore
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material.icons.rounded.SwapVerticalCircle
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material.icons.rounded.SyncProblem
import androidx.compose.material.icons.rounded.VerticalAlignTop
import androidx.compose.material.icons.rounded.Verified
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.material.icons.rounded.PhonelinkErase
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.ui.graphics.vector.ImageVector

/// SF Symbol name → Material icon table. iOS hardcodes SF Symbol names
/// (BuiltinNav, SwipeAction configs travel as strings); this is the single
/// mapping the Android UI resolves them through.
private val sfTable: Map<String, ImageVector> = mapOf(
    // Playback
    "play.fill" to Icons.Rounded.PlayArrow,
    "play.circle" to Icons.Rounded.PlayCircleOutline,
    "pause.fill" to Icons.Rounded.Pause,
    "pause.circle" to Icons.Rounded.PauseCircleOutline,
    "forward.end.fill" to Icons.Rounded.SkipNext,
    "backward.end.fill" to Icons.Rounded.SkipPrevious,
    "goforward" to Icons.Rounded.RotateRight,
    "gobackward" to Icons.Rounded.RotateLeft,
    "waveform" to Icons.Rounded.GraphicEq,
    "waveform.circle" to Icons.Rounded.GraphicEq,
    "waveform.circle.fill" to Icons.Rounded.GraphicEq,
    "moon.zzz" to Icons.Rounded.Bedtime,
    "dot.radiowaves.left.and.right" to Icons.Rounded.Podcasts,
    "music.note.list" to Icons.Rounded.QueueMusic,
    // Lists / navigation
    "list.bullet" to Icons.Rounded.List,
    "list.bullet.circle" to Icons.Rounded.List,
    "list.bullet.circle.fill" to Icons.Rounded.List,
    "square.grid.2x2" to Icons.Rounded.GridView,
    "magnifyingglass" to Icons.Rounded.Search,
    "gear" to Icons.Rounded.Settings,
    "chevron.right" to Icons.Rounded.ChevronRight,
    "chevron.up.chevron.down" to Icons.Rounded.UnfoldMore,
    "chevron.left" to Icons.Rounded.ChevronLeft,
    "line.3.horizontal.decrease.circle" to Icons.Rounded.FilterList,
    "line.3.horizontal.decrease.circle.fill" to Icons.Rounded.FilterList,
    "arrow.up" to Icons.Rounded.ArrowUpward,
    "arrow.down" to Icons.Rounded.ArrowDownward,
    "arrow.up.arrow.down" to Icons.Rounded.SwapVert,
    "arrow.up.arrow.down.circle" to Icons.Rounded.SwapVerticalCircle,
    "arrow.up.to.line" to Icons.Rounded.VerticalAlignTop,
    "arrow.forward.circle" to Icons.Rounded.ArrowCircleRight,
    // Marks / editing
    "checkmark" to Icons.Rounded.Check,
    "checkmark.circle" to Icons.Rounded.CheckCircleOutline,
    "checkmark.circle.fill" to Icons.Rounded.CheckCircle,
    "checkmark.circle.badge.questionmark" to Icons.Rounded.HelpOutline,
    "checkmark.seal" to Icons.Rounded.Verified,
    "circle" to Icons.Rounded.Circle,
    "circle.lefthalf.filled" to Icons.Rounded.Contrast,
    "xmark" to Icons.Rounded.Close,
    "xmark.circle.fill" to Icons.Rounded.Cancel,
    "xmark.octagon" to Icons.Rounded.Dangerous,
    "plus" to Icons.Rounded.Add,
    "minus.circle" to Icons.Rounded.RemoveCircleOutline,
    "pencil" to Icons.Rounded.Edit,
    "trash" to Icons.Rounded.Delete,
    "ellipsis" to Icons.Rounded.MoreHoriz,
    "ellipsis.circle" to Icons.Rounded.Pending,
    // Downloads / sync
    "arrow.down.circle" to Icons.Rounded.ArrowCircleDown,
    "arrow.down.circle.dotted" to Icons.Rounded.Downloading,
    "arrow.down.to.line.circle" to Icons.Rounded.DownloadForOffline,
    "icloud.and.arrow.down" to Icons.Rounded.CloudDownload,
    "icloud.circle" to Icons.Outlined.Cloud,
    "icloud.slash" to Icons.Rounded.CloudOff,
    "exclamationmark.icloud" to Icons.Rounded.CloudOff,
    "arrow.clockwise" to Icons.Rounded.Refresh,
    "arrow.clockwise.circle" to Icons.Rounded.Refresh,
    "arrow.clockwise.circle.fill" to Icons.Rounded.Refresh,
    "arrow.counterclockwise" to Icons.Rounded.Replay,
    "arrow.triangle.2.circlepath" to Icons.Rounded.Sync,
    "exclamationmark.arrow.triangle.2.circlepath" to Icons.Rounded.SyncProblem,
    // Playlists / documents
    "text.badge.plus" to Icons.Rounded.PlaylistAdd,
    "text.badge.minus" to Icons.Rounded.PlaylistRemove,
    "text.badge.checkmark" to Icons.Rounded.PlaylistAddCheck,
    "doc.text" to Icons.Rounded.Description,
    "doc.badge.plus" to Icons.Rounded.NoteAdd,
    "doc.badge.arrow.up" to Icons.Rounded.UploadFile,
    "square.and.arrow.up" to Icons.Rounded.Share,
    "square.and.arrow.down" to Icons.Rounded.SaveAlt,
    // Time
    "clock" to Icons.Rounded.Schedule,
    "clock.arrow.circlepath" to Icons.Rounded.History,
    // Status / errors
    "wifi" to Icons.Rounded.Wifi,
    "wifi.exclamationmark" to Icons.Rounded.WifiOff,
    "wifi.slash" to Icons.Rounded.WifiOff,
    "exclamationmark.triangle.fill" to Icons.Rounded.Warning,
    "exclamationmark.circle" to Icons.Rounded.ErrorOutline,
    "info.circle" to Icons.Rounded.Info,
    "slash.circle" to Icons.Rounded.Block,
    "externaldrive.badge.xmark" to Icons.Rounded.FolderOff,
    "externaldrive.badge.exclamationmark" to Icons.Rounded.SdCardAlert,
    "externaldrive" to Icons.Rounded.Storage,
    "internaldrive" to Icons.Rounded.Storage,
    "slider.horizontal.3" to Icons.Rounded.Tune,
    "gearshape" to Icons.Rounded.Settings,
    "photo" to Icons.Rounded.Image,
    "eye" to Icons.Rounded.Visibility,
    "eye.slash" to Icons.Rounded.VisibilityOff,
    // Devices / people / server
    "iphone" to Icons.Rounded.Smartphone,
    "iphone.slash" to Icons.Rounded.PhonelinkErase,
    "person.circle" to Icons.Rounded.AccountCircle,
    "person.crop.circle" to Icons.Rounded.AccountCircle,
    "person.2" to Icons.Rounded.People,
    "person.2.badge.gearshape" to Icons.Rounded.ManageAccounts,
    "person.badge.plus" to Icons.Rounded.PersonAdd,
    "rectangle.portrait.and.arrow.right" to Icons.Rounded.Logout,
    "server.rack" to Icons.Rounded.Dns,
)

/// Resolve an SF Symbol name; unknown names get a visible question mark.
fun halogenIcon(sf: String): ImageVector = sfTable[sf] ?: Icons.Rounded.HelpOutline

/// Direct references for the hot paths (avoids stringly lookups in new code).
object HalogenIcons {
    val Play = Icons.Rounded.PlayArrow
    val Pause = Icons.Rounded.Pause
    val SkipNext = Icons.Rounded.SkipNext
    val SkipPrevious = Icons.Rounded.SkipPrevious
    val More = Icons.Rounded.MoreHoriz
    val Delete = Icons.Rounded.Delete
    val Download = Icons.Rounded.ArrowCircleDown
    val Downloaded = Icons.Rounded.CheckCircle
    val Queue = Icons.Rounded.QueueMusic
    val Search = Icons.Rounded.Search
    val Settings = Icons.Rounded.Settings
    val Chevron = Icons.Rounded.ChevronRight
    val Close = Icons.Rounded.Close
    val Add = Icons.Rounded.Add
    val Account = Icons.Rounded.AccountCircle
    val Retry = Icons.Rounded.Refresh
    val Offline = Icons.Rounded.WifiOff
    val ArtworkPlaceholder = Icons.Rounded.GraphicEq
}
