#!/usr/bin/env python3
"""Build the current-state mock deck, label index and separate capture gallery."""
import html
from pathlib import Path

root = Path(__file__).resolve().parent
sections = []
for platform in sorted((root / "screenshots").iterdir()):
    if not platform.is_dir():
        continue
    figures = []
    for image in sorted(platform.rglob("*.png")):
        path = image.relative_to(root).as_posix()
        label = image.relative_to(platform).as_posix()
        figures.append(f'<figure><a href="{html.escape(path)}"><img loading="lazy" src="{html.escape(path)}" alt="{html.escape(label)}"></a><figcaption>{html.escape(label)}</figcaption></figure>')
    sections.append(f'<section><h2>{html.escape(platform.name)}</h2><div class="screens">{"".join(figures)}</div></section>')
(root / "captures.html").write_text('''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Halogen screen reference</title>
<style>body{background:#000;color:#fff;font:15px system-ui;margin:24px}a{color:inherit}.screens{display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:24px}figure{margin:0}img{max-width:100%;max-height:580px}figcaption{overflow-wrap:anywhere}h2{margin-top:40px}</style>
<h1>Halogen screen reference</h1><p>Application captures. Historical images are retained until a current journey replaces them.</p>''' + "".join(sections) + "</html>\n")


# Keep editable mock sheets separate from captured application evidence.
import re

sheets = []
labels = ["# Frame labels", "", "Current-state mocks. Labels stay with their frame when sheets move.", "", "| Label | Screen | Sheet |", "|---|---|---|"]
order = {name: index for index, name in enumerate(["start", "library", "listen", "settings", "webui"])}
for page in sorted((root / "pages").glob("*/*.html"), key=lambda page: (order.get(page.parent.name, 99), page.name)):
    source = page.read_text()
    body = source.split("<body>", 1)[1].split("<script", 1)[0]
    body = re.sub(r'<nav class="sheet-nav">.*?</nav>', '', body, flags=re.S)
    body = body.removesuffix("</body></html>\n")
    title = re.search(r'<h1 class="sheet-title">(.*?)</h1>', source).group(1)
    path = page.relative_to(root).as_posix()
    frames = re.findall(r'<span class="fid">(.*?) · (.*?)</span>', source)
    for label, screen in frames:
        labels.append(f"| {label} | {screen} | [{page.stem}]({path}#{label}) |")
    # Source links are repo-relative; shared styles and assets are design-relative.
    body = body.replace('../../../', '../').replace('../../', '')
    sheets.append((path, title, body, frames))

links = ''.join(f'<div class="entry"><a href="{path}">{title}<small>{", ".join(label for label, _ in frames)}</small></a><a href="{path[:-5]}.png">PNG</a></div>' for path, title, _, frames in sheets)
contents = ''.join(f'<section class="csheet">{body}</section>' for _, _, body, _ in sheets)
(root / "master.html").write_text('''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Halogen current UI</title><link rel="stylesheet" href="system/halogen.css"></head><body>
<div class="index"><h1>Halogen current UI</h1><p>Editable reconstructions of the existing app. iOS first, followed by mobile web, desktop web and the desktop app. Sample data and source-based states are identified in captions.</p><p><a href="captures.html">Application captures</a> · <a href="LABELS.md">Frame labels</a> · <a href="system/01-components.html">Shared components</a></p>''' + links + '</div><hr style="margin:40px 0;border:0;border-top:1px solid #333">' + contents + '<script src="system/halogen.js"></script></body></html>\n')
for label, screen in re.findall(r'<span class="fid">(.*?) · (.*?)</span>', (root / "system/01-components.html").read_text()):
    labels.append(f"| {label} | {screen} | [Components](system/01-components.html#{label}) |")
(root / "LABELS.md").write_text("\n".join(labels) + "\n")
