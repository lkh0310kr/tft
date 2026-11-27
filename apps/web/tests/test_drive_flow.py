#!/usr/bin/env python3
"""Mocked OAuth/Picker/Drive flow through the production GPUI Web UI."""

import json
import subprocess
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
URL = "http://127.0.0.1:8092/"
SOURCE = "# TFT workspace\n\nThis is a local sample vault for comparing the **Desktop** and **Web** application shells.\n\n- [ ] Shared editor\n- [x] GPUI Web canvas\n\n[[Architecture]] #tft\n"

GSI_SHIM = r"""
window.google = {
  accounts: { oauth2: { initTokenClient: ({callback}) => ({requestAccessToken: () => callback({access_token: 'mock-token'})}) } },
  picker: {
    ViewId: { FOLDERS: 'folders' },
    Action: { PICKED: 'picked', CANCEL: 'cancel' },
    DocsView: class { setSelectFolderEnabled() { return this; } setIncludeFolders() { return this; } },
    PickerBuilder: class {
      setOAuthToken() { return this; } setTitle() { return this; } setAppId() { return this; }
      setDeveloperKey() { return this; } addView() { return this; }
      setCallback(callback) { this.callback = callback; return this; }
      build() { return {setVisible: () => this.callback({action: 'picked', docs: [{id: 'mock-root', name: 'tft-ui-vault', mimeType: 'application/vnd.google-apps.folder'}]})}; }
    }
  }
};
window.gapi = {load: (_name, {callback}) => callback()};
"""


def main():
    subprocess.run([str(POC / "build-web.sh"), "--features", "drive-smoke-test"], cwd=POC, check=True)
    server = subprocess.Popen(
        ["python3", "-m", "http.server", "8092", "--directory", str(POC / "dist")],
        cwd=POC,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    errors = []
    calls = []

    def mock_drive(route):
        request = route.request
        url = request.url
        calls.append((request.method, url))
        if request.method == "POST":
            calls.append(("POST_BODY", request.post_data or ""))
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"id": "mock-created-note", "name": "Created.md"}))
        elif request.method == "PATCH":
            calls.append(("PATCH_BODY", request.post_data or ""))
            file_id = url.split("/files/")[-1].split("?")[0]
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"id": file_id, "etag": "rev2"}))
        elif "/files/mock-root?" in url:
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"id": "mock-root", "name": "tft-ui-vault", "mimeType": "application/vnd.google-apps.folder"}))
        elif "mock-appearance" in url and "alt=media" in url:
            route.fulfill(status=200, content_type="application/json", body='{"theme":"light"}')
        elif "alt=media" in url:
            route.fulfill(status=200, content_type="text/plain; charset=utf-8", body=SOURCE)
        elif "/files?" in url:
            if ".tft" in url:
                files = [{"id": "mock-tft", "name": ".tft", "mimeType": "application/vnd.google-apps.folder"}]
            elif "appearance.json" in url:
                files = [{"id": "mock-appearance", "name": "appearance.json", "mimeType": "text/plain"}]
            elif "mock-architecture" in url:
                files = []
            else:
                files = [
                    {"id": "mock-architecture", "name": "Architecture", "mimeType": "application/vnd.google-apps.folder"},
                    {"id": "mock-note", "name": "Welcome.md", "mimeType": "text/markdown", "size": str(len(SOURCE)), "modifiedTime": "2026-11-24T00:00:00Z", "createdTime": "2026-11-24T00:00:00Z", "etag": "rev1"},
                ]
            route.fulfill(status=200, content_type="application/json", body=json.dumps({"files": files}))
        else:
            route.fulfill(status=404, content_type="application/json", body=json.dumps({"error": "unexpected mocked URL", "url": url}))

    try:
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                urllib.request.urlopen(URL, timeout=1).close()
                break
            except Exception:
                time.sleep(0.25)
        else:
            raise TimeoutError(f"Web app did not start at {URL}")

        with sync_playwright() as p:
            browser = p.chromium.launch(channel="chrome", headless=True, args=["--enable-unsafe-webgpu"])
            # Match the Desktop app window's logical content size (1400×900).
            # The Drive flow itself uses a 1280×900 viewport for stable clicks;
            # resize only for the final visual comparison capture.
            context = browser.new_context(viewport={"width": 1280, "height": 900}, color_scheme="light")
            page = context.new_page()
            page.route("https://accounts.google.com/gsi/client", lambda route: route.fulfill(content_type="application/javascript", body=GSI_SHIM))
            page.route("https://apis.google.com/js/api.js", lambda route: route.fulfill(content_type="application/javascript", body=GSI_SHIM))
            page.route("https://www.googleapis.com/**", mock_drive)
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)
            page.goto(URL, wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftDriveAppState?.signedIn === false", timeout=30000)

            page.mouse.click(600, 434)
            page.keyboard.type("mock-client.apps.googleusercontent.com")
            page.mouse.click(600, 475)
            page.keyboard.type("mock-api-key")
            page.mouse.click(451, 520)
            page.wait_for_function("JSON.parse(localStorage.getItem('tft.drive-config') || '{}').clientId.includes('mock-client')")

            page.mouse.click(592, 520)
            page.wait_for_function("window.__tftDriveAppState?.signedIn === true", timeout=10000)
            print("PASS GPUI sign-in action awaits the GIS token callback")

            # The second action row appears after sign-in. Drive APIs are all
            # intercepted, so this creates no real Google account data.
            page.mouse.click(455, 543)
            try:
                page.wait_for_function("window.__tftDriveAppState?.activePath === 'Welcome.md'", timeout=12000)
            except Exception as error:
                page.screenshot(path="/tmp/tft-drive-flow-failure.png")
                raise AssertionError(
                    f"Vault did not open; state={page.evaluate('window.__tftDriveAppState')!r}; "
                    f"calls={calls!r}; screenshot=/tmp/tft-drive-flow-failure.png"
                ) from error
            page.wait_for_timeout(250)
            page.set_viewport_size({"width": 1400, "height": 900})
            page.wait_for_timeout(300)
            page.screenshot(path="/tmp/tft-drive-vault.png")
            page.set_viewport_size({"width": 1280, "height": 900})
            state = page.evaluate("window.__tftDriveAppState")
            if state["vaultName"] != "tft-ui-vault":
                raise AssertionError(f"Wrong opened Vault: {state!r}")
            if state["themeDark"] is not False:
                raise AssertionError(f"Vault appearance.json was not applied: {state!r}")
            if not any(method == "GET" and "/files/mock-root?" in url for method, url in calls):
                raise AssertionError(f"Folder metadata request was not made: {calls!r}")
            if not any(method == "GET" and "/files?" in url for method, url in calls):
                raise AssertionError(f"Vault listing request was not made: {calls!r}")
            if not any(method == "GET" and "alt=media" in url for method, url in calls):
                raise AssertionError(f"Markdown media request was not made: {calls!r}")
            print("PASS mocked Drive folder indexed and Markdown opened into shared Workspace")
            print("PASS hidden Vault appearance.json selects the light theme")

            page.keyboard.press("Meta+ArrowDown")
            page.keyboard.type("\nauto-saved after debounce")
            page.wait_for_function(
                "window.__tftDriveAppState?.lastSaved.endsWith('auto-saved after debounce')",
                timeout=15000,
            )
            updates = [body for method, body in calls if method == "PATCH_BODY"]
            if not updates or updates[-1] != SOURCE + "\nauto-saved after debounce":
                raise AssertionError(f"Drive autosave sent the wrong Markdown source: {calls!r}")
            print("PASS Drive autosaves the edited Markdown after the shared 2-second debounce")

            page.keyboard.type("\nEdited in GPUI")
            page.keyboard.press("Meta+S")
            page.wait_for_function("window.__tftDriveAppState?.lastSaved.endsWith('Edited in GPUI')", timeout=10000)
            updates = [body for method, body in calls if method == "PATCH_BODY"]
            expected = SOURCE + "\nauto-saved after debounce\nEdited in GPUI"
            if not updates or updates[-1] != expected:
                raise AssertionError(f"Drive did not receive the edited Markdown source: {calls!r}")
            print("PASS Cmd+S saves the edited Markdown source to Drive")

            page.keyboard.type("\nSaved by button")
            page.mouse.click(1250, 60)
            page.wait_for_function("window.__tftDriveAppState?.lastSaved.endsWith('Saved by button')", timeout=10000)
            updates = [body for method, body in calls if method == "PATCH_BODY"]
            if len(updates) != 3 or not updates[-1].endswith("\nSaved by button"):
                raise AssertionError(f"Save button did not send the current Markdown source: {calls!r}")
            print("PASS Save to Drive button persists the latest Markdown source")

            page.once("dialog", lambda dialog: dialog.accept("Created"))
            page.mouse.click(195, 60)
            try:
                page.wait_for_function("window.__tftDriveAppState?.activePath === 'Created.md'", timeout=12000)
            except Exception as error:
                page.screenshot(path="/tmp/tft-drive-create-failure.png")
                raise AssertionError(
                    f"New Drive note did not open: state={page.evaluate('window.__tftDriveAppState')!r}; "
                    f"calls={calls!r}; screenshot=/tmp/tft-drive-create-failure.png"
                ) from error
            created = page.evaluate("window.__tftDriveAppState")
            if created["activeText"] != "# Created\n\n":
                raise AssertionError(f"New Drive note was not loaded into the shared Editor: {created!r}")
            creates = [body for method, body in calls if method == "POST_BODY"]
            if len(creates) != 1 or "Created.md" not in creates[0] or "# Created" not in creates[0]:
                raise AssertionError(f"Drive did not receive the new Markdown file: {calls!r}")
            print("PASS new Markdown note is created in Drive and opened in the shared Editor")

            page.mouse.click(500, 158)
            page.keyboard.press("Meta+ArrowDown")
            page.keyboard.type("\nSaved before switching Vault")
            page.mouse.click(22, 880)
            page.wait_for_function("window.__tftDriveAppState?.activePath === ''", timeout=15000)
            updates = [body for method, body in calls if method == "PATCH_BODY"]
            if len(updates) != 4 or not updates[-1].endswith("\nSaved before switching Vault"):
                raise AssertionError(f"Switch Vault discarded unsaved Markdown: {calls!r}")
            print("PASS Switch Vault waits for unsaved Markdown to reach Drive")

            if errors:
                raise AssertionError("Browser errors:\n" + "\n".join(errors))
            print("PASS no page or console errors")
            browser.close()
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
