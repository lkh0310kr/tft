#!/usr/bin/env python3
"""Verify that a Drive folder row can be moved to trash from the Web tree.

Run after building apps/web/dist:

    python3 apps/web/tests/test_folder_delete.py
"""
import json
import subprocess
import time
import urllib.request
from pathlib import Path
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
PORT = 8101
URL = f"http://127.0.0.1:{PORT}/"
WELCOME = "# Welcome\n\nWelcome body.\n"
INNER = "# Inner\n\nInner body.\n"
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
            trash.append((url, request.post_data))
            route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"id": "mock-folder", "trashed": True}))
        elif request.method == "PATCH":
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"id": "mock-note"}))
        elif "/files?" in url:
            if "name%20%3D%20%27.tft%27" in url:
                files = [{"id": "mock-tft", "name": ".tft", "mimeType": "application/vnd.google-apps.folder"}]
            elif "name%20%3D%20%27workspace.json%27" in url:
                files = [{"id": "mock-workspace", "name": "workspace.json", "mimeType": "application/json"}]
            elif "mock-tft%27%20in%20parents" in url:
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
            if "mock-workspace" in url:
                body = WORKSPACE
            elif "mock-inner" in url:
                body = INNER
            else:
                body = WELCOME
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
            page.wait_for_function(
                "window.__tftWebDebug?.treePaths?.includes('Notes')", timeout=15000)
            print("TREE", page.evaluate("window.__tftWebDebug.treePaths"))

            # Folder is the first row (folders sort before files).
            page.mouse.click(100, 49, button="right")
            page.wait_for_timeout(500)
            page.screenshot(path="/tmp/tft-web-folder-menu.png")
            page.mouse.click(147, 531)
            page.wait_for_timeout(500)
            page.screenshot(path="/tmp/tft-web-folder-delete-confirm.png")
            page.mouse.click(810, 483)
            page.wait_for_function(
                "!window.__tftWebDebug?.treePaths?.includes('Notes')", timeout=15000)
            page.wait_for_timeout(500)
            print("STATUS", page.evaluate("window.__tftDriveDebug.status"))
            print("TRASH", trash)
            assert trash, "folder trash PATCH was not sent"
            assert trash[-1][0].endswith("files/mock-folder?fields=id,trashed&supportsAllDrives=true"), trash[-1]
            assert json.loads(trash[-1][1]) == {"trashed": True}
            left = page.evaluate("window.__tftWebDebug.treePaths")
            assert "Notes" not in left and "Notes/Inner.md" not in left, left
            assert "Welcome.md" in left, left
            page.screenshot(path="/tmp/tft-web-folder-deleted.png")
            if errors:
                raise AssertionError(errors)
            browser.close()
            print("PASS folder moved to trash")
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
