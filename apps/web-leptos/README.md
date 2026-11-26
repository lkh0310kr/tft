# TFT web — Phase 1/2

Leptos web shell over Google Drive. It signs in with `drive.file`, indexes a
picked folder recursively, shows the file tree, renders Markdown, searches
file names, edits a note in Source mode, and saves it back to Drive.

The Phase 0 diagnostics (list/read/write) are still in the Drive adapter; the
UI now focuses on the read-only shell and source editing. If Picker still
rejects the API key, paste a Drive folder ID into the fallback field.

`drive.file` only grants a manually entered folder if that folder was already
selected with the Picker (or created by the app) under the same grant. For an
arbitrary existing vault, the Picker/API-key path must work first.

**Create test vault** creates an app-owned `TFT Test Vault` with a root note
and a `Notes/Idea.md`, then indexes it. This exercises the tree, reading and
editing without Picker or an API key, and is the fastest way to test the shell
while the Picker key is being sorted out.

## Setup

1. Create a Google Cloud project and enable the **Google Drive API** and
   **Google Picker API**.
2. Configure the **OAuth consent screen** (External), add
   `https://www.googleapis.com/auth/drive.file` under **Data Access**, and add
   your Google account as a **Test user**. `drive.file` is not a sensitive
   scope, so local Testing needs no verification.
3. Create an **OAuth client ID → Web application**.
4. Add `http://127.0.0.1:8080` to **Authorized JavaScript origins**.
5. **API key is optional for the current Picker**: it authenticates with the
   OAuth token. Leave `apiKey` empty first. If your project still asks for a
   developer key, create one and restrict it to `http://127.0.0.1:8080/*` and the
   Picker/Drive APIs.
6. Fill `clientId` in `config.js` (empty values still load the UI).
7. `cargo install trunk --locked` (once), then `trunk serve` in this folder.
8. Open `http://127.0.0.1:8080` and run steps 1–5; the Phase 0 line shows
   PASS for list/read/write.

Both values are public OAuth/API identifiers; the API key should be restricted
to the local origin and the Picker/Drive APIs.

Rendered note HTML is sanitized in the browser before insertion: scripts,
frames, forms and `on*` attributes are removed. The deployed build carries no
API key, the OAuth token lives only in memory, and the vault stays in Drive.

### Where each value comes from

- **Client ID**: Google Cloud Console → **APIs & Services → Credentials** →
  **Create credentials → OAuth client ID → Web application**. Add
  `http://127.0.0.1:8080` as an authorized JavaScript origin. The same dialog
  shows a **client secret**; the browser token flow does not use it, so never
  put it in `config.js` or any frontend file.
- **API key is optional**: the Picker authenticates with the OAuth token, so
  leave `apiKey` empty unless you specifically need a developer key. If you do,
  create one on the same **Credentials** page and set **Application
  restrictions → HTTP referrers** to `http://127.0.0.1:8080/*`, then **API
  restrictions** to Google Drive API and Google Picker API.
- Enable **Google Drive API** and **Google Picker API** in **APIs & Services →
  Library** before creating either credential.

A `403 API key not valid` means the key restriction or enabled APIs do not match
the page origin; `access_denied` means the Google account is not a test user on
the OAuth consent screen.

For Picker's `The API developer key is invalid`: **blank `apiKey` in
`config.js` first**. The current Picker works with `setOAuthToken` alone, and
`setDeveloperKey` is only added when `apiKey` is non-empty. If a key is truly
required, it must come from the same project, have **Google Picker API** and
**Google Drive API** in its API restrictions, and use HTTP-referrer (not IP)
restrictions: `http://127.0.0.1:8080/*`.

## UI direction

The target is not a looser web clone: inside the browser viewport the shell must
match the desktop's tokens, metrics and behavior (sidebar, tab strip, reading
view, source/Live Preview editor, menus). Browser chrome outside the viewport is
the only accepted difference. `tft_app::design` is the token source; the DOM
widgets are a second implementation of the same specs.

Current Step: token-driven light/dark shell, Files/Search docks, sortable tree,
horizontal tabs, Markdown read with image embeds and sanitized raw HTML,
PDF/image/audio/video viewers, Source-mode edit with save/autosave-on-blur,
full-text search over indexed notes, Settings, Quick switcher (Ctrl+O),
new/rename/delete, file context menu, last-vault restore, and an app-owned test
vault.

## Deploy

```powershell
.\deploy.ps1
.\deploy.ps1 -WithApiKey   # only for a key restricted to the Vercel origin
```

The script builds with Trunk, copies the security headers, blanks the API key
in the public bundle by default, and deploys `dist` to `tft-web` production.

The static deployment carries only the public OAuth client ID. The API key is
left empty when Picker uses the OAuth token; if a restricted key is needed, add
its Vercel origin to the key's HTTP-referrer list. The browser never receives a
client secret.

Production origin: `https://tft-web-sage.vercel.app`. Add it to the OAuth
client's **Authorized JavaScript origins** (and to the API key referrers if one
is used). Vercel stores no vault data; files stay in the user's Drive.

## What it answers

- Does `drive.file` + Picker let the app list the picked folder's children?
- Can it read a Markdown file with `files.get?alt=media` from the browser?
- Can it create a file in that folder and read it back?
- Does the Drive API allow these from a Vercel-style static origin?

If folder children are not listed, the fallback is an app-created vault folder
(or a broader scope), which changes the Phase 1 plan.
