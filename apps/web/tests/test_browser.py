#!/usr/bin/env python3
"""Browser regression checks against TFT's production GPUI Editor entity."""

import os
import subprocess
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
URL = os.environ.get("TFT_POC_URL", "http://127.0.0.1:8089/")


def wait_for_server(proc):
    deadline = time.time() + 240
    while time.time() < deadline:
        if proc and proc.poll() is not None:
            raise RuntimeError("test server exited during startup")
        try:
            urllib.request.urlopen(URL, timeout=1).close()
            return
        except Exception:
            time.sleep(0.5)
    raise TimeoutError(f"web app did not start at {URL}")


def main():
    server = None
    if not os.environ.get("TFT_POC_URL"):
        subprocess.run([str(POC / "build-web.sh"), "--features", "browser-test"], cwd=POC, check=True)
        port = URL.rsplit(":", 1)[-1].rstrip("/")
        server = subprocess.Popen(
            ["python3", "-m", "http.server", port, "--directory", str(POC / "dist")],
            cwd=POC,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.STDOUT,
        )
    errors = []
    try:
        wait_for_server(server)
        with sync_playwright() as p:
            browser = p.chromium.launch(channel="chrome", headless=True, args=["--enable-unsafe-webgpu"])
            context = browser.new_context(viewport={"width": 1280, "height": 900}, color_scheme="light")
            context.grant_permissions(["clipboard-read", "clipboard-write"], origin=URL.rstrip("/"))
            page = context.new_page()
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)

            def load():
                page.goto(URL, wait_until="domcontentloaded")
                page.locator("canvas").wait_for()
                page.wait_for_function("window.__tftEditorBooted && window.__tftEditorState?.text !== undefined", timeout=30000)
                page.wait_for_timeout(250)
                return page.evaluate("window.__tftEditorState")

            def state():
                return page.evaluate("window.__tftEditorState")

            def assert_eq(actual, expected, label):
                if actual != expected:
                    raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")
                print(f"PASS {label}")

            def click_main(x, y):
                # Desktop tabs start at y=0; the left dock is 300px wide.
                page.mouse.click(x + 60, y - 38)

            def click_sidebar(x, y):
                page.mouse.click(x, y)

            def click_search_field():
                # Search is a main-area tab on Desktop, not a dock tab.
                page.keyboard.press("Meta+Shift+F")
                page.wait_for_function("window.__tftWorkspaceState?.activeMode === 'search'")
                page.wait_for_timeout(80)

            initial = load()
            assert_eq(page.locator("canvas").count(), 1, "GPUI canvas rendered")
            assert_eq(page.evaluate("window.__tftWorkspaceState.leftTab"), "files", "Files dock starts selected")
            assert_eq(page.evaluate("window.__tftDriveAdapterLoaded"), True, "shared Drive adapter JS binding loaded")
            assert "한글 입력 테스트" in initial["text"]
            screenshot = Path(os.environ.get("TFT_POC_SCREENSHOT", "/tmp/tft-shared-editor.png"))
            page.screenshot(path=str(screenshot))
            print(f"SCREENSHOT {screenshot}")

            # First file rows follow the 40px dock and 40px Explorer headers.
            alpha_x, beta_x, gamma_x, text_y = 440, 488, 531, 158

            workspace = page.evaluate("window.__tftWorkspaceState")
            assert_eq(workspace["activePath"], "A.md", "shared workspace opens initial note")
            assert_eq(workspace["tabs"], ["A.md"], "shared tab model starts with one file tab")

            # Select a second file in the shared FileTree model and verify the
            # shared TabGroup keeps both files. Edits must survive tab changes.
            click_sidebar(35, 78)
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'Notes/B.md'")
            assert_eq(page.evaluate("window.__tftWorkspaceState.tabs"), ["A.md", "Notes/B.md"], "file selection opens a shared-model tab")
            page.keyboard.press("Meta+ArrowDown")
            page.keyboard.type("web-draft")
            page.wait_for_function("window.__tftEditorState?.text.endsWith('web-draft')")
            page.keyboard.press("Meta+1")
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'A.md'")
            page.keyboard.press("Meta+2")
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'Notes/B.md'")
            page.wait_for_function("window.__tftEditorState?.source.includes('web-draft')")
            assert "web-draft" in state()["source"]
            assert "web-draft" in page.evaluate("window.__tftWorkspaceState.vaultText")
            print("PASS shared tabs preserve each Editor buffer while switching")
            print("PASS active Editor text is written to shared VaultData on tab switch")
            page.keyboard.press("Meta+W")
            page.wait_for_function("window.__tftWorkspaceState?.tabs.length === 1")
            assert_eq(page.evaluate("window.__tftWorkspaceState.tabs"), ["A.md"], "closing the active tab removes it from the shared WorkspaceModel")
            assert_eq(page.evaluate("window.__tftWorkspaceState.activePath"), "A.md", "closing the active tab activates the remaining note")
            print("PASS shared WorkspaceModel closes the active Web tab")

            load()
            page.mouse.click(beta_x, text_y, click_count=2, delay=80)
            page.wait_for_function("window.__tftEditorState.selectionEnd > window.__tftEditorState.selectionStart")
            s = state()
            assert_eq(s["text"][s["selectionStart"]:s["selectionEnd"]], "beta", "production Editor double-click word selection")

            load()
            page.mouse.move(alpha_x, text_y)
            page.mouse.down()
            page.mouse.move(481, text_y, steps=8)
            page.mouse.up()
            page.wait_for_timeout(100)
            s = state()
            assert_eq(s["text"][s["selectionStart"]:s["selectionEnd"]], "alpha", "production Editor drag selection")

            load()
            page.keyboard.press("Meta+ArrowUp")
            assert_eq(state()["head"], 0, "Cmd+Up resets cursor before movement checks")
            page.keyboard.press("Meta+ArrowRight")
            assert_eq(state()["head"], len("alpha beta gamma"), "Cmd+Right line end")
            page.keyboard.press("Meta+ArrowLeft")
            assert_eq(state()["head"], 0, "Cmd+Left line start")
            page.keyboard.press("ArrowRight")
            page.keyboard.press("Alt+ArrowRight")
            assert_eq(state()["head"], len("alpha"), "Option+Right word end")
            page.keyboard.press("Alt+ArrowLeft")
            assert_eq(state()["head"], 0, "Option+Left word start")
            page.keyboard.press("ArrowDown")
            page.keyboard.press("ArrowDown")
            assert_eq(state()["head"], len("alpha beta gamma\n\n"), "vertical movement reaches next source line")
            page.keyboard.press("Meta+ArrowUp")
            assert_eq(state()["head"], 0, "Cmd+Up document start")
            page.keyboard.press("Meta+ArrowDown")
            assert_eq(state()["head"], len(state()["text"].encode("utf-8")), "Cmd+Down document end")

            load()
            page.keyboard.press("Meta+A")
            s = state()
            assert_eq(s["text"][s["selectionStart"]:s["selectionEnd"]], s["text"], "Cmd+A source selection")
            page.keyboard.press("Meta+C")
            page.wait_for_function("navigator.clipboard.readText().then(t => t === window.__tftEditorState.text)")
            assert_eq(page.evaluate("navigator.clipboard.readText()"), s["source"], "Cmd+C copies Markdown source")
            page.mouse.click(beta_x, text_y, click_count=2, delay=80)
            page.keyboard.press("Meta+C")
            page.wait_for_function("navigator.clipboard.readText().then(t => t === 'beta')")
            page.mouse.click(gamma_x, text_y, click_count=2, delay=80)
            page.keyboard.press("Meta+V")
            page.wait_for_function("window.__tftEditorState.text.includes('alpha beta beta')")
            assert "alpha beta beta" in state()["source"]
            print("PASS Cmd+V inserts clipboard source")
            page.keyboard.press("Meta+Z")
            page.wait_for_function("!window.__tftEditorState.text.includes('alpha beta beta')")
            page.keyboard.press("Meta+Shift+Z")
            page.wait_for_function("window.__tftEditorState.text.includes('alpha beta beta')")
            print("PASS Cmd+Z / Cmd+Shift+Z undo and redo paste")

            load()
            page.keyboard.press("Meta+ArrowDown")
            page.keyboard.type("web-input")
            page.wait_for_function("window.__tftEditorState.text.endsWith('web-input')")
            assert_eq(state()["source"].endswith("web-input"), True, "typed text reaches saved source string")
            input_el = page.locator("input").first
            input_el.evaluate("el => el.dispatchEvent(new CompositionEvent('compositionstart', {data: '', bubbles: true}))")
            input_el.evaluate("el => el.dispatchEvent(new CompositionEvent('compositionupdate', {data: '한글 편집', bubbles: true}))")
            input_el.evaluate("el => el.dispatchEvent(new CompositionEvent('compositionend', {data: '한글 편집', bubbles: true}))")
            page.wait_for_function("window.__tftEditorState.text.endsWith('web-input한글 편집')")
            assert_eq(state()["source"].endswith("web-input한글 편집"), True, "synthetic Hangul composition reaches saved source string")

            # The same production ReadingView/model used by Desktop can be
            # selected from the Web source editor and switched back without
            # losing the Markdown buffer.
            page.screenshot(path="/tmp/tft-before-preview-toggle.png")
            page.mouse.click(1253, 58)
            page.wait_for_function("window.__tftWorkspaceState?.activeMode === 'reading'")
            assert_eq(page.evaluate("window.__tftWorkspaceState.activePath"), "A.md", "preview keeps the active note")
            page.screenshot(path="/tmp/tft-shared-reading-preview.png")
            page.mouse.click(456, 233)
            page.wait_for_function("window.__tftWorkspaceState?.vaultText.includes('- [x] task')")
            print("PASS ReadingView task toggle updates the shared source buffer")
            page.mouse.move(489, 256)
            page.mouse.down()
            page.mouse.up()
            try:
                page.wait_for_function("window.__tftWorkspaceState?.activePath === 'Notes/B.md'", timeout=3000)
            except Exception as error:
                page.screenshot(path="/tmp/tft-wikilink-click-failure.png")
                raise AssertionError(f"Preview wikilink did not open; state={page.evaluate('window.__tftWorkspaceState')!r}") from error
            assert_eq(page.evaluate("window.__tftWorkspaceState.activeMode"), "reading", "ordinary preview wikilink preserves the current view mode")
            assert_eq(
                page.evaluate("window.__tftWorkspaceState.tabs"),
                ["Notes/B.md"],
                "ordinary preview wikilink navigates the current tab",
            )
            print("PASS ReadingView wikilink follows in the current tab")
            click_main(260, 96)
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'A.md'")
            assert_eq(page.evaluate("window.__tftWorkspaceState.activePath"), "A.md", "Workspace Back restores the previous note")
            page.wait_for_function("window.__tftWorkspaceState?.activeMode === 'reading'")
            click_main(293, 96)
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'Notes/B.md'")
            assert_eq(page.evaluate("window.__tftWorkspaceState.activePath"), "Notes/B.md", "Workspace Forward restores the next note")
            click_main(260, 96)
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'A.md'")
            click_sidebar(35, 78)
            page.wait_for_function("window.__tftWorkspaceState?.tabs.length === 2 && window.__tftWorkspaceState.activePath === 'Notes/B.md'")
            page.keyboard.press("Meta+1")
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'A.md'")
            page.wait_for_function("window.__tftWorkspaceState?.activeMode === 'reading'")
            page.keyboard.down("Meta")
            page.mouse.click(489, 256)
            page.keyboard.up("Meta")
            page.wait_for_function("window.__tftWorkspaceState?.tabs.length === 3 && window.__tftWorkspaceState.activePath === 'Notes/B.md'")
            assert_eq(
                page.evaluate("window.__tftWorkspaceState.tabs"),
                ["A.md", "Notes/B.md", "Notes/B.md"],
                "Meta-click opens a duplicate target tab through the shared model",
            )
            page.keyboard.press("Meta+1")
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'A.md'")
            page.wait_for_function("window.__tftWorkspaceState?.activeMode === 'reading'")
            page.mouse.click(1253, 58)
            page.wait_for_function("window.__tftWorkspaceState?.activeMode === 'source'")
            page.wait_for_function("window.__tftEditorState?.source.endsWith('web-input한글 편집') && window.__tftEditorState.source.includes('- [x] task')")
            restored_source = state()["source"]
            if not restored_source.endswith("web-input한글 편집") or "- [x] task" not in restored_source:
                raise AssertionError(f"Markdown changed across preview and tab navigation: {restored_source!r}")
            print("PASS source returns with the edited Markdown intact")
            print("PASS shared ReadingView preview toggles with Markdown source preserved")

            load()
            click_search_field()
            page.keyboard.type("tag:#tag")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 1")
            assert_eq(
                page.evaluate("window.__tftWorkspaceState.searchResults"),
                ["A.md"],
                "shared metadata index finds a Markdown tag",
            )
            page.keyboard.press("Meta+A")
            page.keyboard.type("task-todo:task")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 1 && window.__tftWorkspaceState.searchResults[0] === 'A.md'")
            print("PASS shared metadata index finds an open task")
            page.keyboard.press("Meta+A")
            page.keyboard.type("[status:draft]")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 1 && window.__tftWorkspaceState.searchResults[0] === 'Notes/B.md'")
            print("PASS shared metadata index finds a frontmatter property value")
            page.keyboard.press("Meta+A")
            page.keyboard.type("tag:#tag")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 1 && window.__tftWorkspaceState.searchResults[0] === 'A.md'")
            page.keyboard.press("Meta+A")
            page.keyboard.type("Shared TextBuffer")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 1 && window.__tftWorkspaceState.searchResults[0] === 'Notes/B.md'")
            assert_eq(
                page.evaluate("window.__tftWorkspaceState.searchResults"),
                ["Notes/B.md"],
                "shared search matcher finds the matching Vault note",
            )
            page.screenshot(path="/tmp/tft-web-search.png")
            page.mouse.click(330, 132)  # first Search result in the main Search tab
            try:
                page.wait_for_function("window.__tftWorkspaceState?.activePath === 'Notes/B.md'", timeout=3000)
            except Exception as error:
                page.screenshot(path="/tmp/tft-web-search-click-failure.png")
                raise AssertionError(f"search result click did not open note; state={page.evaluate('window.__tftWorkspaceState')!r}") from error
            print("PASS search result opens its note in the shared WorkspaceModel")
            load()
            click_search_field()
            page.keyboard.type("a")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 2")
            page.keyboard.press("ArrowDown")
            assert_eq(
                page.evaluate("window.__tftWorkspaceState.selectedSearchHit"),
                1,
                "search results move selection with ArrowDown",
            )
            page.keyboard.press("Enter")
            page.wait_for_function("window.__tftWorkspaceState?.activePath === 'Notes/B.md'")
            print("PASS Enter opens the selected search result")

            page.keyboard.type(" web-live-index-phrase")
            click_search_field()
            page.keyboard.press("Meta+A")
            page.keyboard.type("web-live-index-phrase")
            page.wait_for_function("window.__tftWorkspaceState?.searchResults?.length === 1 && window.__tftWorkspaceState.searchResults[0] === 'Notes/B.md'")
            print("PASS search index reflects the active editor's latest Markdown")

            if errors:
                raise AssertionError("browser errors:\n" + "\n".join(errors))
            print("PASS no page or console errors")
            browser.close()
    finally:
        if server:
            server.terminate()
            server.wait(timeout=10)


if __name__ == "__main__":
    main()
