#!/usr/bin/env python3
"""Verify the Drive cache: cold load waits for the network; a warm reload and a
cached folder expansion paint from IndexedDB before Drive answers.

Run against a built `apps/web/dist` (serve-web.sh or build-web.sh) with
Playwright + Chrome installed:

    python3 apps/web/tests/test_cache.py

The test serves dist on 127.0.0.1:8095 and mocks GIS/Picker/Drive.
"""
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
PORT = 8095
URL = f"http://127.0.0.1:{PORT}/"
WELCOME = "# Welcome\n\nWelcome body.\n"
WELCOME_CHANGED = "# Welcome\n\nChanged on another device.\n"
INNER = "# Inner\n\nInner body.\n"
NETWORK_DELAY_MS = 2500
WARM_BUDGET_MS = 1000

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
window.__tokenRequests = 0;
window.google = {
  accounts: { oauth2: { initTokenClient: ({callback}) => ({requestAccessToken: () => { window.__tokenRequests += 1; callback({access_token: 'mock-token', expires_in: 3600}); }}) } },
  picker: {
    ViewId: { FOLDERS: 'folders' },
    Action: { PICKED: 'picked', CANCEL: 'cancel' },
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

# Delay every Drive GET so cached and network paths are distinguishable. The
# delay lives inside the page and does not block Playwright's own polling.
DELAY_FETCH = f"""
(() => {{
  const original = window.fetch.bind(window);
  window.fetch = async (url, init) => {{
    const target = typeof url === 'string' ? url : (url && url.url) || '';
    const method = (init && init.method) || 'GET';
    if (method === 'GET' && target.includes('www.googleapis.com')) {{
      await new Promise((resolve) => setTimeout(resolve, {NETWORK_DELAY_MS}));
    }}
    return original(url, init);
  }};
}})();
"""


def main():
    server = subprocess.Popen(
        ["python3", "-m", "http.server", str(PORT), "--directory", str(POC / "dist")],
        cwd=POC,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    calls = []
    source = {"welcome": WELCOME}

    def mock_drive(route):
        request = route.request
        url = request.url
        calls.append((request.method, url))
        if "/files/mock-root?" in url:
            route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"id": "mock-root", "name": "PKMS", "mimeType": "application/vnd.google-apps.folder"}))
        elif request.method == "PATCH":
            route.fulfill(status=200, content_type="application/json", body=json.dumps(
                {"id": "mock-note", "modifiedTime": "2026-11-24T00:00:00Z"}))
        elif "/files?" in url:
            if "name%20%3D%20%27.tft%27" in url:
                files = [{"id": "mock-tft", "name": ".tft", "mimeType": "application/vnd.google-apps.folder"}]
            elif "name%20%3D%20%27workspace.json%27" in url:
                files = [{"id": "mock-workspace", "name": "workspace.json", "mimeType": "application/json"}]
            elif "mock-tft%27%20in%20parents" in url or "mock-tft' in parents" in url:
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
                body = source["welcome"]
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
            page.add_init_script(DELAY_FETCH)
            page.route("https://accounts.google.com/gsi/client", lambda route: route.fulfill(
                content_type="application/javascript", body=GSI_SHIM))
            page.route("https://apis.google.com/js/api.js", lambda route: route.fulfill(
                content_type="application/javascript", body=GSI_SHIM))
            page.route("https://www.googleapis.com/**", mock_drive)

            def wait_tree_path(path, timeout=30000):
                page.wait_for_function(
                    "path => window.__tftWebDebug?.treePaths?.includes(path)",
                    arg=path, timeout=timeout)

            def click_tree_path(path):
                paths = page.evaluate("window.__tftWebDebug.treePaths")
                index = paths.index(path)
                page.mouse.click(100, 49 + 24 * index)

            # --- cold visit: the network delay is visible ---
            started = time.monotonic()
            page.goto(URL, wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftDriveDebug?.workspace === true", timeout=30000)
            cold_workspace_ms = (time.monotonic() - started) * 1000
            page.wait_for_function("window.__tftWebDebug?.activeLoaded === true", timeout=30000)
            cold_note_ms = (time.monotonic() - started) * 1000
            print(f"cold workspace visible: {cold_workspace_ms:.0f} ms")
            print(f"cold note visible: {cold_note_ms:.0f} ms")
            assert cold_workspace_ms >= NETWORK_DELAY_MS * 0.75, cold_workspace_ms
            assert page.evaluate("window.__tftWebDebug.vaultText") == len(WELCOME)

            page.evaluate("window.__tftDriveCache.flush()")
            click_tree_path("Notes")
            started = time.monotonic()
            wait_tree_path("Notes/Inner.md")
            cold_folder_ms = (time.monotonic() - started) * 1000
            print(f"cold folder children: {cold_folder_ms:.0f} ms")
            assert cold_folder_ms >= NETWORK_DELAY_MS * 0.75, cold_folder_ms
            page.evaluate("window.__tftDriveCache.flush()")
            # Leave the folder collapsed in the saved workspace so the warm
            # reload has to expand it from the cached listing.
            click_tree_path("Notes")
            page.wait_for_function(
                "!window.__tftWebDebug.treePaths.includes('Notes/Inner.md')", timeout=5000)

            # --- warm reload: IndexedDB paints the workspace and note first ---
            network_before = len(calls)
            started = time.monotonic()
            page.reload(wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftDriveDebug?.workspace === true", timeout=30000)
            warm_workspace_ms = (time.monotonic() - started) * 1000
            page.wait_for_function("window.__tftWebDebug?.activeLoaded === true", timeout=30000)
            warm_note_ms = (time.monotonic() - started) * 1000
            print(f"warm workspace visible: {warm_workspace_ms:.0f} ms")
            print(f"warm note visible: {warm_note_ms:.0f} ms")
            assert warm_workspace_ms < WARM_BUDGET_MS, warm_workspace_ms
            assert warm_note_ms < WARM_BUDGET_MS, warm_note_ms
            assert page.evaluate("window.__tftWebDebug.vaultText") == len(WELCOME)

            # Expand from cached children, then collapse the warm view so the
            # click measures the cache rather than an already-expanded folder.
            wait_tree_path("Notes")
            paths = page.evaluate("window.__tftWebDebug.treePaths")
            if "Notes/Inner.md" not in paths:
                click_tree_path("Notes")
                page.wait_for_function(
                    "window.__tftWebDebug.treePaths.includes('Notes/Inner.md')", timeout=30000)
            click_tree_path("Notes")  # collapse
            page.wait_for_function(
                "!window.__tftWebDebug.treePaths.includes('Notes/Inner.md')", timeout=5000)
            started = time.monotonic()
            click_tree_path("Notes")  # expand again: must come from cache
            wait_tree_path("Notes/Inner.md", timeout=5000)
            warm_folder_ms = (time.monotonic() - started) * 1000
            print(f"warm folder children: {warm_folder_ms:.0f} ms")
            assert warm_folder_ms < WARM_BUDGET_MS, warm_folder_ms

            # The cache is stale-while-revalidate, not offline-only: the live
            # reads still reach Drive after the cached frame.
            page.wait_for_timeout(NETWORK_DELAY_MS + 500)
            fresh = calls[network_before:]
            assert any(url.endswith("mock-note?alt=media&supportsAllDrives=true") for _, url in fresh), fresh
            assert any("mock-folder" in url and "in%20parents" in url for _, url in fresh), fresh
            print(f"network revalidation calls: {len(fresh)}")

            # A cached body is stale-while-revalidate: the live read catches a
            # change made on another device and replaces the cached render.
            source["welcome"] = WELCOME_CHANGED
            page.evaluate("window.__tftDriveCache.flush()")
            page.reload(wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftWebDebug?.activeLoaded === true", timeout=30000)
            page.wait_for_function(
                f"window.__tftWebDebug?.editorText === {len(WELCOME_CHANGED)}", timeout=15000)
            print("revalidated note body length:", page.evaluate("window.__tftWebDebug.editorText"))
            page.evaluate("window.__tftDriveCache.flush()")

            # The refreshed body is what the next warm load paints, still
            # before the delayed network can answer.
            started = time.monotonic()
            page.reload(wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftWebDebug?.activeLoaded === true", timeout=30000)
            refreshed_note_ms = (time.monotonic() - started) * 1000
            assert page.evaluate("window.__tftWebDebug.vaultText") == len(WELCOME_CHANGED)
            print(f"warm note after revalidation: {refreshed_note_ms:.0f} ms")
            assert refreshed_note_ms < WARM_BUDGET_MS, refreshed_note_ms

            page.screenshot(path="/tmp/tft-web-cache-warm.png")
            browser.close()
            print("PASS cache: warm workspace/folder paint before Drive answers")
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
