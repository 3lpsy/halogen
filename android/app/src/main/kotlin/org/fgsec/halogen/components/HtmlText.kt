package org.fgsec.halogen.components

import android.util.LruCache
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.LinkInteractionListener
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import java.net.URI

/// Feed-HTML rendering — the native mirror of the web's whitelist parser
/// (webui/component-widgets/src/html.rs). Feed descriptions are hostile input, never
/// handed to a real HTML engine: tokenize, keep a small structural-tag whitelist,
/// decode entities, emit AnnotatedString blocks; script/style contents drop whole.
object HtmlText {
    /// Paragraph blocks for the detail page — `HtmlDescription` renders one
    /// Text per block with real spacing between them.
    fun blocks(
        html: String,
        linkColor: Color = Color.Unspecified,
        onLink: ((String) -> Unit)? = null,
    ): List<AnnotatedString> = Parser(linkColor, onLink).parse(html)

    /// Flattened plain text (row previews) through the same parse.
    fun plain(html: String): String =
        blocks(html).joinToString("\n") { it.text }.trim()

    /// One-line preview for list rows, memoized: row bodies re-evaluate on
    /// every overlay/player publish, and a full parse of multi-KB show-notes
    /// per visible row per render burns the main thread. Keyed by content
    /// hash — stable within a launch, which is all a cache needs.
    private val previewCache = LruCache<String, String>(256)
    private val whitespaceRun = Regex("\\s+")

    fun preview(html: String): String {
        val key = "${html.hashCode()}-${html.length}"
        previewCache.get(key)?.let { return it }
        val flat = plain(html).replace(whitespaceRun, " ")
        previewCache.put(key, flat)
        return flat
    }

    /// Whether an href is safe to expose as tappable. Everything else
    /// (`javascript:`, `data:`, relative/unknown schemes) renders as plain
    /// text — web `is_safe_href` parity.
    fun isSafeHref(href: String): Boolean {
        val h = href.trim().lowercase()
        return h.startsWith("http://") || h.startsWith("https://") || h.startsWith("mailto:")
    }

    // MARK: - entities

    /// Single-pass entity decode — one scan, so `&amp;lt;` can't
    /// double-decode into `<` (web `decode_entities` parity, plus the named
    /// typographic set feeds actually use).
    fun decodeEntities(s: String): String {
        if (!s.contains('&')) return s
        val out = StringBuilder(s.length)
        var i = 0
        while (i < s.length) {
            if (s[i] != '&') {
                out.append(s[i])
                i += 1
                continue
            }
            // Entity name runs to ';' within a short window (web MAX_ENTITY_LEN).
            var j = i + 1
            var end = -1
            while (j < s.length && j - i <= 12) {
                if (s[j] == ';') {
                    end = j
                    break
                }
                j += 1
            }
            val decoded = if (end >= 0) decodeEntity(s.substring(i + 1, end)) else null
            if (decoded == null) {
                out.append('&')
                i += 1
                continue
            }
            out.append(decoded)
            i = end + 1
        }
        return out.toString()
    }

    private fun decodeEntity(entity: String): String? = when (entity) {
        "amp" -> "&"
        "lt" -> "<"
        "gt" -> ">"
        "quot" -> "\""
        "apos" -> "'"
        "nbsp" -> " "
        "rsquo" -> "’"
        "lsquo" -> "‘"
        "rdquo" -> "”"
        "ldquo" -> "“"
        "mdash" -> "—"
        "ndash" -> "–"
        "hellip" -> "…"
        else -> {
            if (!entity.startsWith("#")) null
            else {
                val num = entity.drop(1)
                val value =
                    if (num.startsWith("x") || num.startsWith("X"))
                        num.drop(1).toIntOrNull(16)
                    else num.toIntOrNull()
                if (value == null || value < 0 || value > 0x10FFFF || value in 0xD800..0xDFFF)
                    null
                else String(Character.toChars(value))
            }
        }
    }

    // MARK: - tokenizer

    private sealed interface Tok {
        data class Open(val name: String, val href: String?) : Tok
        data class Close(val name: String) : Tok
        data object Br : Tok
        data class Text(val text: String) : Tok
    }

    /// null = malformed (`<` with no `>`) — the caller renders the raw input
    /// literally rather than a half-interpreted result (web ParseError).
    private fun tokenize(input: String): List<Tok>? {
        val toks = mutableListOf<Tok>()
        var i = 0
        var textStart = 0
        while (i < input.length) {
            if (input[i] != '<') {
                i += 1
                continue
            }
            if (i > textStart) {
                toks.add(Tok.Text(input.substring(textStart, i)))
            }
            var j = i + 1
            while (j < input.length && input[j] != '>') j += 1
            if (j >= input.length) return null
            pushTag(toks, input.substring(i + 1, j).trim())
            i = j + 1
            textStart = i
        }
        if (textStart < input.length) {
            toks.add(Tok.Text(input.substring(textStart)))
        }
        return toks
    }

    private fun pushTag(toks: MutableList<Tok>, inner: String) {
        if (inner.startsWith("!")) return  // comments / doctype
        if (inner.startsWith("/")) {
            toks.add(Tok.Close(tagName(inner.drop(1))))
            return
        }
        val name = tagName(inner)
        if (name == "br") {
            toks.add(Tok.Br)
            return
        }
        toks.add(Tok.Open(name, href = if (name == "a") attribute(inner, "href") else null))
    }

    private fun tagName(s: String): String =
        s.lowercase().takeWhile { it.isLetterOrDigit() }

    private fun attribute(tag: String, name: String): String? {
        // Token boundary on BOTH sides (web extract_attr): `(^|\s)name\s*=`
        // stops a name that is a prefix/suffix of a longer one (`data-href`)
        // from matching `href`.
        val match = Regex("(^|\\s)${Regex.escape(name)}\\s*=\\s*", RegexOption.IGNORE_CASE)
            .find(tag) ?: return null
        var rest = tag.substring(match.range.last + 1)
        val quote = rest.firstOrNull() ?: return null
        // Attribute values are entity-encoded like text (`&amp;` in query
        // strings is routine feed HTML) — decode before use (web parity).
        if (quote == '"' || quote == '\'') {
            rest = rest.drop(1)
            val end = rest.indexOf(quote)
            if (end < 0) return null
            return decodeEntities(rest.substring(0, end))
        }
        return decodeEntities(rest.takeWhile { !it.isWhitespace() })
    }

    // MARK: - parser

    /// One styled inline run inside a block.
    private data class Run(
        val text: String,
        val bold: Boolean = false,
        val italic: Boolean = false,
        val code: Boolean = false,
        val link: String? = null,
    )

    private class Parser(
        private val linkColor: Color,
        onLink: ((String) -> Unit)?,
    ) {
        private val blocks = mutableListOf<AnnotatedString>()
        private var para = mutableListOf<Run>()
        private var listItem: MutableList<Run>? = null
        private var heading = false
        private var bold = 0
        private var italic = 0
        private var code = 0
        private var listDepth = 0
        /// Inside script/style — their inner text is never rendered.
        private var skip = 0
        /// An open <a>: capture its text, emit one link run on close.
        private var link: Pair<String, String>? = null

        private val listener: LinkInteractionListener? = onLink?.let { cb ->
            LinkInteractionListener { ann -> (ann as? LinkAnnotation.Url)?.let { cb(it.url) } }
        }

        fun parse(html: String): List<AnnotatedString> {
            val toks = tokenize(html)
            if (toks == null) {
                val raw = html.trim()
                return if (raw.isEmpty()) emptyList() else listOf(AnnotatedString(raw))
            }
            for (tok in toks) handle(tok)
            link?.let {
                link = null
                appendLink(it)
            }
            flushItem()
            flushPara()
            return blocks
        }

        private fun handle(tok: Tok) {
            when (tok) {
                is Tok.Text -> {
                    if (skip > 0) return
                    val decoded = collapseWhitespace(decodeEntities(tok.text))
                    if (decoded.isEmpty()) return
                    val open = link
                    if (open != null) {
                        link = open.copy(second = open.second + decoded)
                    } else {
                        appendText(decoded)
                    }
                }

                is Tok.Br -> {
                    if (skip > 0) return
                    appendToSink(Run("\n"))
                }

                is Tok.Open -> {
                    if (tok.name == "script" || tok.name == "style") {
                        skip += 1
                        return
                    }
                    if (skip > 0) return
                    when (tok.name) {
                        "p", "div", "section", "blockquote" -> flushPara()
                        "h1", "h2", "h3", "h4", "h5", "h6" -> {
                            flushPara()
                            heading = true
                        }
                        "ul", "ol" -> {
                            flushItem()
                            flushPara()
                            listDepth += 1
                        }
                        "li" -> {
                            flushItem()
                            listItem = mutableListOf()
                        }
                        "b", "strong" -> bold += 1
                        "i", "em" -> italic += 1
                        "code" -> code += 1
                        "pre" -> {
                            flushPara()
                            code += 1
                        }
                        "a" -> link = (tok.href ?: "") to ""
                        else -> {}  // unknown tag: dropped, inner text kept
                    }
                }

                is Tok.Close -> {
                    if (tok.name == "script" || tok.name == "style") {
                        skip = maxOf(0, skip - 1)
                        return
                    }
                    if (skip > 0) return
                    when (tok.name) {
                        "p", "div", "section", "blockquote" -> flushPara()
                        "h1", "h2", "h3", "h4", "h5", "h6" -> flushPara()
                        "ul", "ol" -> {
                            flushItem()
                            flushPara()
                            listDepth = maxOf(0, listDepth - 1)
                        }
                        "li" -> flushItem()
                        "b", "strong" -> bold = maxOf(0, bold - 1)
                        "i", "em" -> italic = maxOf(0, italic - 1)
                        "code" -> code = maxOf(0, code - 1)
                        "pre" -> {
                            code = maxOf(0, code - 1)
                            flushPara()
                        }
                        "a" -> link?.let {
                            link = null
                            appendLink(it)
                        }
                        else -> {}
                    }
                }
            }
        }

        // MARK: sinks

        private val sinkTail: Char?
            get() = (listItem ?: para).lastOrNull()?.text?.lastOrNull()

        private fun appendToSink(run: Run) {
            (listItem ?: para).add(run)
        }

        /// Append a text run in the current style, normalizing the space at
        /// the run boundary (collapsed runs keep at most one).
        private fun appendText(decoded: String) {
            var text = decoded
            if (text.startsWith(" ") && (sinkTail == null || sinkTail?.isWhitespace() == true)) {
                text = text.drop(1)
            }
            if (text.isEmpty()) return
            appendToSink(styled(text))
        }

        private fun appendLink(pending: Pair<String, String>) {
            val (href, rawText) = pending
            val label = rawText.trim { it.isWhitespace() }
            if (label.isEmpty()) return
            val tail = sinkTail
            if (tail != null && !tail.isWhitespace()) {
                appendToSink(Run(" "))
            }
            // Only vetted schemes stay tappable (web is_safe_href) — anything
            // else is demoted to plain text.
            val safe = isSafeHref(href)
            appendToSink(styled(label).copy(link = if (safe) href.trim() else null))
        }

        private fun styled(text: String): Run =
            Run(text, bold = bold > 0 || heading, italic = italic > 0, code = code > 0)

        // MARK: blocks

        private fun flushPara() {
            val block = trimmed(para)
            para = mutableListOf()
            heading = false
            if (block.isEmpty()) return
            blocks.add(render(block))
        }

        private fun flushItem() {
            val item = listItem ?: return
            listItem = null
            val body = trimmed(item)
            if (body.isEmpty()) return
            // Web parity: every list renders bulleted; nesting indents.
            val bullet = "    ".repeat(maxOf(0, listDepth - 1)) + "•  "
            blocks.add(render(listOf(Run(bullet)) + body))
        }

        private fun trimmed(runs: List<Run>): List<Run> {
            val out = runs.toMutableList()
            while (out.isNotEmpty()) {
                val text = out.first().text.dropWhile { it.isWhitespace() }
                if (text.isEmpty()) {
                    out.removeAt(0)
                    continue
                }
                out[0] = out.first().copy(text = text)
                break
            }
            while (out.isNotEmpty()) {
                val text = out.last().text.dropLastWhile { it.isWhitespace() }
                if (text.isEmpty()) {
                    out.removeAt(out.size - 1)
                    continue
                }
                out[out.size - 1] = out.last().copy(text = text)
                break
            }
            return out
        }

        private fun render(runs: List<Run>): AnnotatedString = buildAnnotatedString {
            for (run in runs) {
                val style = SpanStyle(
                    fontWeight = if (run.bold) FontWeight.Bold else null,
                    fontStyle = if (run.italic) FontStyle.Italic else null,
                    fontFamily = if (run.code) FontFamily.Monospace else null,
                )
                if (run.link != null) {
                    val linked = style.copy(
                        color = linkColor, textDecoration = TextDecoration.Underline)
                    withLink(
                        LinkAnnotation.Url(run.link, TextLinkStyles(style = linked), listener)
                    ) { append(run.text) }
                } else {
                    withStyle(style) { append(run.text) }
                }
            }
        }

        /// Collapse whitespace runs (feed HTML is newline-soup) to single
        /// spaces; `br` is the only intra-paragraph line break we honor.
        private fun collapseWhitespace(s: String): String {
            val out = StringBuilder(s.length)
            var lastWasSpace = false
            for (ch in s) {
                if (ch.isWhitespace()) {
                    if (!lastWasSpace) {
                        out.append(' ')
                        lastWasSpace = true
                    }
                } else {
                    out.append(ch)
                    lastWasSpace = false
                }
            }
            return out.toString()
        }
    }
}

/// The rendered description body: one Text per parsed block, spaced by the
/// view — per-block layout gives real, consistent paragraph/list spacing.
/// Feed links leave the app, so a tap confirms before opening (web parity:
/// episode detail's pending-external-link dialog).
@Composable
fun HtmlDescription(html: String, modifier: Modifier = Modifier) {
    var pendingLink by remember { mutableStateOf<String?>(null) }
    val linkColor = MaterialTheme.colorScheme.primary
    val blocks = remember(html, linkColor) {
        HtmlText.blocks(html, linkColor) { pendingLink = it }
    }

    SelectionContainer {
        Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (block in blocks) {
                Text(block, modifier = Modifier.fillMaxWidth())
            }
        }
    }

    val target = pendingLink
    if (target != null) {
        val uriHandler = LocalUriHandler.current
        val host = runCatching { URI(target).host }.getOrNull() ?: "link"
        AlertDialog(
            onDismissRequest = { pendingLink = null },
            title = { Text("Open external link?") },
            text = { Text(target) },
            confirmButton = {
                TextButton(onClick = {
                    runCatching { uriHandler.openUri(target) }
                    pendingLink = null
                }) { Text("Open $host") }
            },
            dismissButton = {
                TextButton(onClick = { pendingLink = null }) { Text("Cancel") }
            },
        )
    }
}
