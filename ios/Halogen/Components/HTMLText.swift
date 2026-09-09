import SwiftUI

/// Feed-HTML rendering — native mirror of the web's whitelist parser
/// (webui/component-widgets/src/html.rs). Feed HTML is hostile input, never handed to
/// a real HTML engine: tokenize, whitelist structural tags, decode entities,
/// emit AttributedString blocks in the app's typography; script/style drop whole.
enum HTMLText {
    /// Paragraph blocks for the detail page — `HTMLDescription` renders one
    /// Text per block with real spacing between them.
    static func blocks(_ html: String) -> [AttributedString] {
        var parser = Parser()
        return parser.parse(html)
    }

    /// Flattened plain text (row previews) through the same parse.
    static func plain(_ html: String) -> String {
        blocks(html)
            .map { String($0.characters) }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// One-line row preview cache, keyed by content hash: rows re-evaluate on
    /// every overlay/player publish, and re-parsing multi-KB show-notes per
    /// visible row per render burns the main thread (web memoizes likewise).
    private static let previewCache = NSCache<NSString, NSString>()

    static func preview(_ html: String) -> String {
        let key = "\(html.hashValue)-\(html.count)" as NSString
        if let hit = previewCache.object(forKey: key) { return hit as String }
        let flat = plain(html).replacingOccurrences(
            of: "\\s+", with: " ", options: .regularExpression)
        previewCache.setObject(flat as NSString, forKey: key)
        return flat
    }

    /// Whether an href is safe to expose as tappable. Everything else
    /// (`javascript:`, `data:`, relative/unknown schemes) renders as plain
    /// text — web `is_safe_href` parity.
    static func isSafeHref(_ href: String) -> Bool {
        let h = href.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return h.hasPrefix("http://") || h.hasPrefix("https://") || h.hasPrefix("mailto:")
    }

    // MARK: - entities

    /// Single-pass entity decode — one scan, so `&amp;lt;` can't
    /// double-decode into `<` (web `decode_entities` parity, plus the named
    /// typographic set feeds actually use).
    static func decodeEntities(_ s: String) -> String {
        guard s.contains("&") else { return s }
        var out = ""
        out.reserveCapacity(s.count)
        let chars = Array(s)
        var i = 0
        while i < chars.count {
            guard chars[i] == "&" else {
                out.append(chars[i])
                i += 1
                continue
            }
            // Entity name runs to ';' within a short window (web MAX_ENTITY_LEN).
            var j = i + 1
            var end: Int?
            while j < chars.count, j - i <= 12 {
                if chars[j] == ";" {
                    end = j
                    break
                }
                j += 1
            }
            guard let end, let decoded = decodeEntity(String(chars[(i + 1)..<end])) else {
                out.append("&")
                i += 1
                continue
            }
            out.append(decoded)
            i = end + 1
        }
        return out
    }

    private static func decodeEntity(_ entity: String) -> Character? {
        switch entity {
        case "amp": return "&"
        case "lt": return "<"
        case "gt": return ">"
        case "quot": return "\""
        case "apos": return "'"
        case "nbsp": return " "
        case "rsquo": return "’"
        case "lsquo": return "‘"
        case "rdquo": return "”"
        case "ldquo": return "“"
        case "mdash": return "—"
        case "ndash": return "–"
        case "hellip": return "…"
        default:
            guard entity.hasPrefix("#") else { return nil }
            let num = entity.dropFirst()
            let value: UInt32?
            if num.hasPrefix("x") || num.hasPrefix("X") {
                value = UInt32(num.dropFirst(), radix: 16)
            } else {
                value = UInt32(num)
            }
            guard let value, let scalar = Unicode.Scalar(value) else { return nil }
            return Character(scalar)
        }
    }

    // MARK: - tokenizer

    private enum Tok {
        case open(String, href: String?)
        case close(String)
        case br
        case text(String)
    }

    /// nil = malformed (`<` with no `>`) — the caller renders the raw input
    /// literally rather than a half-interpreted result (web ParseError).
    private static func tokenize(_ input: String) -> [Tok]? {
        var toks: [Tok] = []
        let chars = Array(input)
        var i = 0
        var textStart = 0
        while i < chars.count {
            guard chars[i] == "<" else {
                i += 1
                continue
            }
            if i > textStart {
                toks.append(.text(String(chars[textStart..<i])))
            }
            var j = i + 1
            while j < chars.count, chars[j] != ">" { j += 1 }
            guard j < chars.count else { return nil }
            pushTag(
                &toks,
                String(chars[(i + 1)..<j]).trimmingCharacters(in: .whitespacesAndNewlines))
            i = j + 1
            textStart = i
        }
        if textStart < chars.count {
            toks.append(.text(String(chars[textStart...])))
        }
        return toks
    }

    private static func pushTag(_ toks: inout [Tok], _ inner: String) {
        if inner.hasPrefix("!") { return }  // comments / doctype
        if inner.hasPrefix("/") {
            toks.append(.close(tagName(String(inner.dropFirst()))))
            return
        }
        let name = tagName(inner)
        if name == "br" {
            toks.append(.br)
            return
        }
        toks.append(.open(name, href: name == "a" ? attribute(inner, "href") : nil))
    }

    private static func tagName(_ s: String) -> String {
        String(s.lowercased().prefix(while: { $0.isLetter || $0.isNumber }))
    }

    private static func attribute(_ tag: String, _ name: String) -> String? {
        // Token boundary on BOTH sides (web extract_attr): `(^|\s)name\s*=`
        // stops a name that is a prefix/suffix of a longer one (`data-href`)
        // from matching `href`.
        guard
            let match = tag.range(
                of: "(^|\\s)\(name)\\s*=\\s*", options: [.regularExpression, .caseInsensitive])
        else { return nil }
        var rest = tag[match.upperBound...]
        guard let quote = rest.first else { return nil }
        // Attribute values are entity-encoded like text (`&amp;` in query
        // strings is routine feed HTML) — decode before use (web parity).
        if quote == "\"" || quote == "'" {
            rest = rest.dropFirst()
            guard let end = rest.firstIndex(of: quote) else { return nil }
            return decodeEntities(String(rest[..<end]))
        }
        return decodeEntities(String(rest.prefix(while: { !$0.isWhitespace })))
    }

    // MARK: - parser

    private struct Parser {
        var blocks: [AttributedString] = []
        var para = AttributedString()
        var listItem: AttributedString?
        var heading = false
        var bold = 0
        var italic = 0
        var code = 0
        var listDepth = 0
        /// Inside script/style — their inner text is never rendered.
        var skip = 0
        /// An open <a>: capture its text, emit one link run on close.
        var link: (href: String, text: String)?

        mutating func parse(_ html: String) -> [AttributedString] {
            guard let toks = HTMLText.tokenize(html) else {
                let raw = html.trimmingCharacters(in: .whitespacesAndNewlines)
                return raw.isEmpty ? [] : [AttributedString(raw)]
            }
            for tok in toks { handle(tok) }
            if let pending = link {
                link = nil
                appendLink(pending)
            }
            flushItem()
            flushPara()
            return blocks
        }

        private mutating func handle(_ tok: Tok) {
            switch tok {
            case .text(let raw):
                guard skip == 0 else { return }
                let decoded = collapseWhitespace(HTMLText.decodeEntities(raw))
                guard !decoded.isEmpty else { return }
                if link != nil {
                    link!.text += decoded
                } else {
                    appendText(decoded)
                }

            case .br:
                guard skip == 0 else { return }
                appendToSink(AttributedString("\n"))

            case .open(let name, let href):
                if name == "script" || name == "style" {
                    skip += 1
                    return
                }
                guard skip == 0 else { return }
                switch name {
                case "p", "div", "section", "blockquote":
                    flushPara()
                case "h1", "h2", "h3", "h4", "h5", "h6":
                    flushPara()
                    heading = true
                case "ul", "ol":
                    flushItem()
                    flushPara()
                    listDepth += 1
                case "li":
                    flushItem()
                    listItem = AttributedString()
                case "b", "strong":
                    bold += 1
                case "i", "em":
                    italic += 1
                case "code":
                    code += 1
                case "pre":
                    flushPara()
                    code += 1
                case "a":
                    link = (href: href ?? "", text: "")
                default:
                    break  // unknown tag: dropped, inner text kept
                }

            case .close(let name):
                if name == "script" || name == "style" {
                    skip = max(0, skip - 1)
                    return
                }
                guard skip == 0 else { return }
                switch name {
                case "p", "div", "section", "blockquote":
                    flushPara()
                case "h1", "h2", "h3", "h4", "h5", "h6":
                    flushPara()
                case "ul", "ol":
                    flushItem()
                    flushPara()
                    listDepth = max(0, listDepth - 1)
                case "li":
                    flushItem()
                case "b", "strong":
                    bold = max(0, bold - 1)
                case "i", "em":
                    italic = max(0, italic - 1)
                case "code":
                    code = max(0, code - 1)
                case "pre":
                    code = max(0, code - 1)
                    flushPara()
                case "a":
                    if let pending = link {
                        link = nil
                        appendLink(pending)
                    }
                default:
                    break
                }
            }
        }

        // MARK: sinks

        private var sinkTail: Character? {
            (listItem ?? para).characters.last
        }

        private mutating func appendToSink(_ run: AttributedString) {
            if listItem != nil {
                listItem!.append(run)
            } else {
                para.append(run)
            }
        }

        /// Append a text run in the current style, normalizing the space at
        /// the run boundary (collapsed runs keep at most one).
        private mutating func appendText(_ decoded: String) {
            var text = decoded
            if text.hasPrefix(" "), sinkTail == nil || sinkTail?.isWhitespace == true {
                text.removeFirst()
            }
            guard !text.isEmpty else { return }
            appendToSink(styled(text))
        }

        private mutating func appendLink(_ pending: (href: String, text: String)) {
            let label = pending.text.trimmingCharacters(in: .whitespaces)
            guard !label.isEmpty else { return }
            if let tail = sinkTail, !tail.isWhitespace {
                appendToSink(AttributedString(" "))
            }
            var run = styled(label)
            // Only vetted schemes stay tappable (web is_safe_href) — anything
            // else is demoted to plain text.
            if HTMLText.isSafeHref(pending.href),
                let url = URL(string: pending.href.trimmingCharacters(in: .whitespacesAndNewlines))
            {
                run.link = url
                run.underlineStyle = .single
            }
            appendToSink(run)
        }

        private func styled(_ text: String) -> AttributedString {
            var run = AttributedString(text)
            var intent: InlinePresentationIntent = []
            if bold > 0 || heading { intent.insert(.stronglyEmphasized) }
            if italic > 0 { intent.insert(.emphasized) }
            if code > 0 { intent.insert(.code) }
            if !intent.isEmpty { run.inlinePresentationIntent = intent }
            return run
        }

        // MARK: blocks

        private mutating func flushPara() {
            defer { heading = false }
            let block = trimmed(para)
            para = AttributedString()
            guard !block.characters.isEmpty else { return }
            blocks.append(block)
        }

        private mutating func flushItem() {
            guard let item = listItem else { return }
            listItem = nil
            let body = trimmed(item)
            guard !body.characters.isEmpty else { return }
            // Web parity: every list renders bulleted; nesting indents.
            var block = AttributedString(
                String(repeating: "    ", count: max(0, listDepth - 1)) + "•  ")
            block.append(body)
            blocks.append(block)
        }

        private func trimmed(_ a: AttributedString) -> AttributedString {
            var a = a
            while let first = a.characters.first, first.isWhitespace {
                a.characters.removeFirst()
            }
            while let last = a.characters.last, last.isWhitespace {
                a.characters.removeLast()
            }
            return a
        }

        /// Collapse whitespace runs (feed HTML is newline-soup) to single
        /// spaces; `br` is the only intra-paragraph line break we honor.
        private func collapseWhitespace(_ s: String) -> String {
            var out = ""
            out.reserveCapacity(s.count)
            var lastWasSpace = false
            for ch in s {
                if ch.isWhitespace {
                    if !lastWasSpace {
                        out.append(" ")
                        lastWasSpace = true
                    }
                } else {
                    out.append(ch)
                    lastWasSpace = false
                }
            }
            return out
        }
    }
}

/// The rendered description body: one Text per parsed block, spaced by the view
/// (SwiftUI Text ignores paragraph styles). Inherits the caller's
/// `.font`/`.tint`; links open through the system.
struct HTMLDescription: View {
    let html: String

    @State private var blocks: [AttributedString] = []
    @State private var pendingLink: URL?
    /// The PARENT environment's opener (captured before the override below).
    @Environment(\.openURL) private var systemOpenURL

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(blocks.indices, id: \.self) { i in
                Text(blocks[i])
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
            }
        }
        .task(id: html) { blocks = HTMLText.blocks(html) }
        // Feed links leave the app — confirm first (web parity). The override
        // captures the tap; the dialog's Open uses the real system opener.
        .environment(
            \.openURL,
            OpenURLAction { url in
                pendingLink = url
                return .handled
            }
        )
        .confirmationDialog(
            "Open external link?",
            isPresented: Binding(
                get: { pendingLink != nil },
                set: { if !$0 { pendingLink = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Open \(pendingLink?.host ?? "link")") {
                if let url = pendingLink { systemOpenURL(url) }
                pendingLink = nil
            }
            Button("Cancel", role: .cancel) { pendingLink = nil }
        } message: {
            Text(pendingLink?.absoluteString ?? "")
        }
    }
}
