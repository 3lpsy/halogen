#!/usr/bin/env python3
"""Render contact sheets to adjacent PNGs using their measured page dimensions.
Requires chromedriver/Chrome on PATH or CHROMEDRIVER/CHROME_BIN overrides.
"""
import base64
import glob
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = os.path.dirname(os.path.abspath(__file__))


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def call(base, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, data=data, method=method,
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.load(resp)["value"]
    except urllib.error.HTTPError as err:
        sys.exit(f"render.py: {method} {path} failed: {err.read().decode()[:600]}")


def main(pages):
    os.chdir(ROOT)
    if not pages:
        pages = sorted(glob.glob("pages/*/*.html") + glob.glob("system/*.html"))
    driver = os.environ.get("CHROMEDRIVER") or shutil.which("chromedriver")
    chrome = os.environ.get("CHROME_BIN") or shutil.which("chromium") or shutil.which("chromium-browser")
    if not driver or not chrome:
        sys.exit("render.py: chromedriver and Chrome are both required")
    port = free_port()
    profile = tempfile.mkdtemp(prefix="halogen-render-")
    proc = subprocess.Popen([driver, f"--port={port}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    base = f"http://127.0.0.1:{port}"
    try:
        for _ in range(50):
            try:
                urllib.request.urlopen(base + "/status", timeout=1)
                break
            except OSError:
                time.sleep(0.1)
        caps = {"capabilities": {"alwaysMatch": {"goog:chromeOptions": {
            "binary": chrome,
            "args": ["--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                     "--hide-scrollbars", "--allow-file-access-from-files",
                     "--force-device-scale-factor=1", f"--user-data-dir={profile}"]}}}}
        session = call(base, "POST", "/session", caps)["sessionId"]
        s = f"/session/{session}"
        for page in pages:
            call(base, "POST", s + "/window/rect", {"width": 800, "height": 1000})
            call(base, "POST", s + "/url", {"url": f"file://{ROOT}/{page}"})
            time.sleep(0.4)
            size = call(base, "POST", s + "/execute/sync", {
                "script": "return [document.documentElement.scrollWidth, document.documentElement.scrollHeight];",
                "args": []})
            w, h = int(size[0]), int(size[1])
            call(base, "POST", s + "/window/rect", {"width": w, "height": h})
            time.sleep(0.3)
            # Capture document pixels, independent of browser window decorations.
            png = call(base, "POST", s + "/goog/cdp/execute", {
                "cmd": "Page.captureScreenshot",
                "params": {"captureBeyondViewport": True,
                           "clip": {"x": 0, "y": 0, "width": w, "height": h, "scale": 1}}
            })["data"]
            out = page[:-5] + ".png"
            with open(out, "wb") as f:
                f.write(base64.b64decode(png))
            print(f"{out} ({w}x{h})")
        call(base, "DELETE", s)
    finally:
        proc.terminate()
        shutil.rmtree(profile, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1:])
