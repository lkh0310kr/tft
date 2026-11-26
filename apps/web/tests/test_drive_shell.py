#!/usr/bin/env python3
"""Production GPUI Web Drive landing screen and browser config persistence."""

import subprocess
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
POC = ROOT / "apps/web"
URL = "http://127.0.0.1:8091/"


def main():
    subprocess.run([str(POC / "build-web.sh")], cwd=POC, check=True)
    server = subprocess.Popen(
        ["python3", "-m", "http.server", "8091", "--directory", str(POC / "dist")],
        cwd=POC,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    errors = []
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
            context = browser.new_context(viewport={"width": 1280, "height": 900})
            page = context.new_page()
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)
            page.goto(URL, wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftDriveAdapterLoaded === true", timeout=30000)
            page.screenshot(path="/tmp/tft-drive-shell.png")
            print("PASS GPUI Web Drive landing screen rendered")

            # These coordinates target the rendered GPUI fields and button,
            # so this checks real keyboard focus, TextInput, click and storage.
            page.mouse.click(600, 434)
            page.keyboard.type("poc-client-id")
            page.mouse.click(600, 475)
            page.keyboard.type("poc-api-key")
            page.mouse.click(451, 520)
            page.wait_for_function(
                "JSON.parse(localStorage.getItem('tft.drive-config') || '{}').clientId === 'poc-client-id'"
            )
            config = page.evaluate("JSON.parse(localStorage.getItem('tft.drive-config'))")
            if config != {"clientId": "poc-client-id", "apiKey": "poc-api-key"}:
                raise AssertionError(f"Unexpected stored Drive config: {config!r}")
            print("PASS GPUI TextInput config saves to browser storage")

            page.reload(wait_until="domcontentloaded")
            page.locator("canvas").wait_for(timeout=30000)
            page.wait_for_function("window.__tftDriveAdapterLoaded === true", timeout=30000)
            if page.evaluate("JSON.parse(localStorage.getItem('tft.drive-config'))") != config:
                raise AssertionError("Drive config did not survive reload")
            print("PASS Drive config survives reload")
            if errors:
                raise AssertionError("Browser errors:\n" + "\n".join(errors))
            print("PASS no page or console errors")
            browser.close()
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
