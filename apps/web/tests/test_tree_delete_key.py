#!/usr/bin/env python3
"""Verify macOS fn+Backspace (the Delete key) trashes the focused tree row.

Run after building apps/web/dist:

    python3 apps/web/tests/test_tree_delete_key.py
"""
import json
import subprocess
import time
import urllib.request
from pathlib import Path
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
PORT = 8104
URL = f"http://127.0.0.1:{PORT}/"
WELCOME = "# Welcome\n\nBody.\n"
INNER = "# Inner\n\nInner.\n"
WORKSPACE = json.dumps({
    "tftWeb": {
        "version": 2,
        "tabs": [{"file": "Welcome.md", "mode": "reading", "pinned": False, "historyIx": 1,
                  "history": [{"file": "Welcome.md", "mode": "reading", "offset": 0}]}],
        "activeTab": 0, "activeGroup": 1,
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


def main():
    server = subprocess.Popen(
        ["python3", "-m", "http.server", str(PORT), "--directory", str(POC / "dist")],
        cwd=POC, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT,
    )
    trash = []

    def mock_drive(route):
        request = route.request
        url = request.url
        if "/files/mock-root?" in url:
            route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"id": "mock-root", "name": "PKMS", "mimeType": "application/vnd.google-apps.folder"}))
        elif request.method == "PATCH" and "trashed" in (request.post_data or ""):
            trash.append(url)
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"id": "mock-deleted", "trashed": True}))
        elif request.method == "PATCH":
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"id": "mock-workspace"}))
        elif "/files?" in url:
            if "name%20%3D%20%27.tft%27" in url:
                files = [{"id": "mock-tft", "name": ".tft", "mimeType": "application/vnd.google-apps.folder"}]
            elif "name%20%3D%20%27workspace.json%27" in url:
                files = [{"id": "mock-workspace", "name": "workspace.json", "mimeType": "application/json"}]
            elif "mock-tft" in url:
                files = []
            elif "mock-folder" in url:
                files = [{"id": "mock-inner", "name": "Inner.md", "mimeType": "text/markdown",
                          "size": str(len(INNER)), "modifiedTime": "2026-11-24T00:00:00Z",
                          "createdTime": "2026-11-24T00:00:00Z"}]
            else:
                files = [
                    {"id": "mock-folder", "name": "Notes", "mimeType": "application/vnd.google-apps.folder"},
                    {"id": "mock-note", "name": "Welcome.md", "mimeType": "text/markdown",
                     "size": str(len(WELCOME)), "modifiedTime": "2026-11-24T00:00:00Z",
                     "createdTime": "2026-11-24T00:00:00Z"},
                ]
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"files": files}))
        elif "alt=media" in url:
            body = WORKSPACE if "mock-workspace" in url else (INNER if "mock-inner" in url else WELCOME)
            route.fulfill(status=200, content_type="text/plain; charset=utf-8", body=body)
        else:
            route.fulfill(status=404, content_type="application/json", body=json.dumps(
                {"error": "unexpected", "url": url}))

    def click_row(page, path):
        paths = page.evaluate("window.__tftWebDebug.treePaths")
        page.mouse.click(100, 49 + 24 * paths.index(path))
        page.wait_for_timeout(250)

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
            page.wait_for_function("window.__tftWebDebug?.treePaths?.includes('Notes')", timeout=15000)

            # Folder row: fn+Backspace opens the same Move to trash alert.
            click_row(page, "Notes")
            page.keyboard.press("Delete")
            page.wait_for_timeout(500)
            page.screenshot(path="/tmp/tft-web-tree-delete-key.png")
            page.mouse.click(810, 483)
            page.wait_for_function(
                "!window.__tftWebDebug?.treePaths?.includes('Notes')", timeout=15000)
            assert any("mock-folder" in url for url in trash), trash
            print("folder trashed by Delete key:", trash[-1])

            # File row: same key path.
            click_row(page, "Welcome.md")
            page.keyboard.press("Delete")
            page.wait_for_timeout(500)
            page.mouse.click(810, 483)
            page.wait_for_function(
                "!window.__tftWebDebug?.treePaths?.includes('Welcome.md')", timeout=15000)
            assert any("mock-note" in url for url in trash), trash
            assert len(trash) == 2, trash
            page.screenshot(path="/tmp/tft-web-tree-delete-done.png")
            if errors:
                raise AssertionError(errors)
            browser.close()
            print("PASS fn+Backspace moves the focused tree row to trash")
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
