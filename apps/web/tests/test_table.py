#!/usr/bin/env python3
"""Verify GFM tables render in Reading and Live Preview, with screenshots.

Run after building apps/web/dist:

    python3 apps/web/tests/test_table.py

Reading is drawn by ReadingView; Live Preview needs an EmbedSource on the
Editor, otherwise tables stay raw source lines. The bitmap check looks for the
table's long horizontal border runs, which raw `| ... |` lines do not produce.
"""
import json
import subprocess
import time
import urllib.request
from pathlib import Path
from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
PORT = 8103
URL = f"http://127.0.0.1:{PORT}/"
SOURCE = """# Tables

| Name | Value | Notes |
| --- | ---: | --- |
| Alpha | 1 | first |
| Beta | 2 | second |
"""
WORKSPACE = json.dumps({
    "tftWeb": {
        "version": 2,
        "tabs": [{"file": "Welcome.md", "mode": "reading", "pinned": False, "historyIx": 1,
                  "history": [{"file": "Welcome.md", "mode": "reading", "offset": 0}]}],
        "activeTab": 0,
        "activeGroup": 1,
        "groups": [{"id": 1, "activeTab": 0, "tabs": [
            {"file": "Welcome.md", "mode": "reading", "pinned": False, "historyIx": 1,
             "history": [{"file": "Welcome.md", "mode": "reading", "offset": 0}]}]}],
        "layout": {"direction": "vertical", "dimensions": None, "children": [{"group": 1}]},
    }
})
GSI_SHIM = r"""
window.google = {
  accounts: { oauth2: { initTokenClient: ({callback}) => ({requestAccessToken: () => { callback({access_token: 'mock-token', expires_in: 3600}); }}) } },
  picker: {
    ViewId: { FOLDERS: 'folders' }, Action: { PICKED: 'picked', CANCEL: 'cancel' },
    DocsView: class { setSelectFolderEnabled() { return this; } setIncludeFolders() { return this; } },
    PickerBuilder: class {
      setOAuthToken() { return this; } setTitle() { return this; } setAppId() { return this; }
      setDeveloperKey() { return this; } addView() { return this; }
      setCallback(callback) { this.callback = callback; return this; }
      build() { return {setVisible: () => this.callback({action: 'picked', docs: [{id: 'mock-root', name: 'PKMS', mimeType: 'application/vnd.google-apps.folder'}]})}; }
    }
  }
};
window.gapi = {load: (_name, {callback}) => callback()};
"""


def table_border_rows(path):
    """Rows with a long horizontal dark run, which a table's borders make."""
    image = Image.open(path).convert("L")
    found = 0
    for y in range(120, min(520, image.height)):
        dark = sum(1 for x in range(400, 900) if image.getpixel((x, y)) < 245)
        if dark > 300:
            found += 1
    return found


def main():
    server = subprocess.Popen(
        ["python3", "-m", "http.server", str(PORT), "--directory", str(POC / "dist")],
        cwd=POC, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT,
    )

    def mock_drive(route):
        request = route.request
        url = request.url
        if "/files/mock-root?" in url:
            route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"id": "mock-root", "name": "PKMS", "mimeType": "application/vnd.google-apps.folder"}))
        elif request.method in ("PATCH", "POST"):
            route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"id": "mock-workspace", "name": "workspace.json"}))
        elif "/files?" in url:
            if "name%20%3D%20%27.tft%27" in url:
                files = [{"id": "mock-tft", "name": ".tft", "mimeType": "application/vnd.google-apps.folder"}]
            elif "name%20%3D%20%27workspace.json%27" in url:
                files = [{"id": "mock-workspace", "name": "workspace.json", "mimeType": "application/json"}]
            elif "mock-tft" in url:
                files = []
            else:
                files = [{"id": "mock-note", "name": "Welcome.md", "mimeType": "text/markdown",
                          "size": str(len(SOURCE)), "modifiedTime": "2026-11-24T00:00:00Z",
                          "createdTime": "2026-11-24T00:00:00Z"}]
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"files": files}))
        elif "alt=media" in url:
            body = WORKSPACE if "mock-workspace" in url else SOURCE
            route.fulfill(status=200, content_type="text/plain; charset=utf-8", body=body)
        else:
            route.fulfill(status=404, content_type="application/json", body=json.dumps(
                {"error": "unexpected", "url": url}))

    try:
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                urllib.request.urlopen(URL, timeout=1).close()
                break
            except Exception:
                time.sleep(0.25)
        with sync_playwright() as p:
            browser = p.chromium.launch(channel="chrome", headless=True, args=["--enable-unsafe-webgpu"])
            context = browser.new_context(viewport={"width": 1280, "height": 900}, color_scheme="light")
            page = context.new_page()
            page.route("https://accounts.google.com/gsi/client", lambda route: route.fulfill(
                content_type="application/javascript", body=GSI_SHIM))
            page.route("https://apis.google.com/js/api.js", lambda route: route.fulfill(
                content_type="application/javascript", body=GSI_SHIM))
            page.route("https://www.googleapis.com/**", mock_drive)
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
            page.goto(URL, wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftWebDebug?.activeLoaded === true", timeout=30000)
            page.wait_for_timeout(600)

            reading = "/tmp/tft-web-table-reading.png"
            page.screenshot(path=reading)
            reading_borders = table_border_rows(reading)
            print(f"reading table border rows: {reading_borders}")
            assert reading_borders >= 3, reading_borders

            page.mouse.click(700, 300)
            page.keyboard.press("Meta+E")
            page.wait_for_function("window.__tftWebDebug?.mode === 'live-preview'", timeout=5000)
            page.wait_for_function("window.__tftWebDebug?.tableWidgets >= 1", timeout=10000)
            page.wait_for_timeout(600)
            live = "/tmp/tft-web-table-live.png"
            page.screenshot(path=live)
            live_borders = table_border_rows(live)
            print(f"live table border rows: {live_borders}")
            assert live_borders >= 3, live_borders
            state = page.evaluate("window.__tftWebDebug")
            assert state["tableWidgets"] == 1, state
            assert state["editorText"] == len(SOURCE), state
            if errors:
                raise AssertionError(errors)
            browser.close()
            print("PASS table renders in Reading and Live Preview")
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
