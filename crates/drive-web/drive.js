// Phase 0 Google Drive adapter: GIS token, Picker, and the few REST calls
// the spike needs. The Rust app owns the flow; this file only talks to Google.

let tokenClient = null;
let accessToken = null;
let accessTokenExpiresAt = 0;
let pending = null;
let tokenRequest = null;
let tokenClientGeneration = 0;
let tokenClientId = "";
const audioPlayers = new Map();
let videoViewer = null;

const SESSION_TOKEN_KEY = "tft.drive-session-token";
const PERSISTED_TOKEN_KEY = "tft.drive-access-token";
// `drive.file` covers writes to files the app created/opened; `drive.readonly`
// covers listing the children of a Picker-selected folder and reading them.
const DRIVE_SCOPE =
  "https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/drive.readonly";

function restoreSessionToken(clientId) {
  // Check each store independently: a stale local copy must not erase a valid
  // session copy, and a blocked localStorage must not prevent session restore.
  const candidates = [];
  try {
    candidates.push([window.localStorage, PERSISTED_TOKEN_KEY]);
  } catch {}
  try {
    candidates.push([window.sessionStorage, SESSION_TOKEN_KEY]);
  } catch {}
  let saved = null;
  for (const [storage, key] of candidates) {
    let raw = null;
    try {
      raw = storage.getItem(key);
    } catch {
      continue;
    }
    if (!raw) continue;
    let candidate = null;
    try {
      candidate = JSON.parse(raw);
    } catch {}
    if (
      candidate?.token &&
      candidate.clientId === clientId &&
      candidate.scope === DRIVE_SCOPE &&
      Number.isFinite(candidate.expiresAt) &&
      Date.now() < candidate.expiresAt
    ) {
      if (!saved || candidate.expiresAt > saved.expiresAt) saved = candidate;
    } else {
      try {
        storage.removeItem(key);
      } catch {}
    }
  }
  if (!saved) return false;
  // Keep only the short-lived OAuth access token. It is scoped to the OAuth
  // client that issued it and is never treated as a refresh token.
  accessToken = saved.token;
  accessTokenExpiresAt = saved.expiresAt;
  rememberSessionToken(accessToken, accessTokenExpiresAt, clientId);
  return true;
}

function rememberSessionToken(token, expiresAt, clientId = tokenClientId) {
  const value = JSON.stringify({ token, expiresAt, clientId, scope: DRIVE_SCOPE });
  try {
    sessionStorage.setItem(SESSION_TOKEN_KEY, value);
  } catch {}
  try {
    localStorage.setItem(PERSISTED_TOKEN_KEY, value);
  } catch {}
}

export function getConfig() {
  let saved = {};
  try {
    saved = JSON.parse(localStorage.getItem("tft.drive-config") || "{}");
  } catch {}
  const config = { ...(window.TFT_CONFIG || {}), ...saved };
  config.appId = config.appId || String(config.clientId || "").split("-")[0];
  return JSON.stringify(config);
}

export function saveConfig(clientId, apiKey) {
  const config = { clientId: String(clientId).trim(), apiKey: String(apiKey).trim() };
  window.TFT_CONFIG = { ...(window.TFT_CONFIG || {}), ...config };
  let persisted = true;
  try {
    localStorage.setItem("tft.drive-config", JSON.stringify(config));
  } catch {
    persisted = false;
  }
  return persisted;
}

export function currentToken() {
  return accessToken || "";
}

export function openDriveFile(id) {
  const url = `https://drive.google.com/file/d/${encodeURIComponent(id)}/view`;
  window.open(url, "_blank", "noopener,noreferrer");
}

export function modifierKeyLabel() {
  return /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl";
}

export function initTokenClient(clientId) {
  if (!window.google?.accounts?.oauth2) {
    throw new Error("Google Identity Services is not loaded");
  }
  tokenClientId = clientId;
  const generation = ++tokenClientGeneration;
  tokenClient = google.accounts.oauth2.initTokenClient({
    client_id: clientId,
    scope: DRIVE_SCOPE,
    callback: (response) => {
      if (generation !== tokenClientGeneration) return;
      const p = pending;
      pending = null;
      if (p?.timer) clearTimeout(p.timer);
      if (response.error) {
        p?.reject(new Error(response.error));
      } else {
        accessToken = response.access_token;
        const expiresIn = Number(response.expires_in);
        accessTokenExpiresAt = Number.isFinite(expiresIn)
          ? Date.now() + Math.max(0, expiresIn - 60) * 1000
          : 0;
        rememberSessionToken(accessToken, accessTokenExpiresAt, tokenClientId);
        p?.resolve(accessToken);
      }
    },
    error_callback: (error) => {
      if (generation !== tokenClientGeneration) return;
      const p = pending;
      pending = null;
      if (p?.timer) clearTimeout(p.timer);
      p?.reject(new Error(error?.type || "Google token request failed"));
    },
  });
}

export function requestToken() {
  if (!tokenClient) {
    return Promise.reject(new Error("initTokenClient was not called"));
  }
  return requestAccessToken({ prompt: "" });
}

export async function tryResumeToken(clientId) {
  // A token from this browser needs no network round trip and no GIS load:
  // use it before waiting for the Google script. GIS is initialized only as
  // a best effort so the token can be refreshed when it expires.
  if (restoreSessionToken(clientId)) {
    try {
      initTokenClient(clientId);
    } catch {}
    return accessToken;
  }
  if (!(await waitFor(() => window.google?.accounts?.oauth2))) {
    throw new Error("Google Identity Services did not load");
  }
  initTokenClient(clientId);
  // Expired tokens still go through Google's silent GIS flow; refresh tokens
  // are never stored.
  return requestAccessToken({ prompt: "none" });
}

function requestAccessToken(options) {
  if (tokenRequest) return tokenRequest;
  tokenRequest = new Promise((resolve, reject) => {
    const request = { resolve, reject, timer: null };
    pending = request;
    request.timer = setTimeout(() => {
      if (pending === request) pending = null;
      // Invalidate any late callback from the timed-out request before allowing
      // another request to use the shared callback slot.
      try {
        initTokenClient(tokenClientId);
      } catch {}
      reject(new Error("Google token request timed out"));
    }, 15000);
    try {
      tokenClient.requestAccessToken(options);
    } catch (error) {
      clearTimeout(request.timer);
      if (pending === request) pending = null;
      reject(error);
    }
  }).finally(() => {
    tokenRequest = null;
  });
  return tokenRequest;
}

async function waitFor(check, ms = 10000) {
  const started = Date.now();
  while (!check()) {
    if (Date.now() - started > ms) return false;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return true;
}

export async function pickFolder(apiKey, appId) {
  if (!accessToken) {
    throw new Error("Sign in first");
  }
  await refreshTokenIfExpired();
  if (!(await waitFor(() => window.gapi))) {
    throw new Error("Google API loader did not load");
  }
  return new Promise((resolve, reject) => {
    gapi.load("picker", {
      callback: () => {
        if (!window.google?.picker) {
          reject(new Error("Google Picker library failed to load"));
          return;
        }
        const view = google.picker.ViewId.FOLDERS
          ? new google.picker.DocsView(google.picker.ViewId.FOLDERS)
          : new google.picker.DocsView();
        if (view.setSelectFolderEnabled) view.setSelectFolderEnabled(true);
        if (view.setIncludeFolders) view.setIncludeFolders(true);

        const builder = new google.picker.PickerBuilder()
          .setOAuthToken(accessToken)
          .setTitle("Choose your PKMS vault folder");
        if (appId) builder.setAppId(appId);
        if (apiKey) builder.setDeveloperKey(apiKey);
        const picker = builder
          .addView(view)
          .setCallback((data) => {
            if (data.action === google.picker.Action.PICKED) {
              const doc = data.docs[0];
              if (doc.mimeType && doc.mimeType !== "application/vnd.google-apps.folder") {
                reject(new Error("Pick a folder, not a file"));
                return;
              }
              resolve({ id: doc.id, name: doc.name });
            } else if (data.action === google.picker.Action.CANCEL) {
              reject(new Error("Picker cancelled"));
            }
          })
          .build();
        picker.setVisible(true);
      },
      onerror: () => reject(new Error("gapi.load('picker') failed")),
    });
  });
}

export async function driveFetch(url) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(url);
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive ${response.status}: ${body}`);
  }
  return body;
}

async function authorizedFetch(url, init = {}) {
  if (!accessToken) throw new Error("Sign in first");
  await refreshTokenIfExpired();
  const send = () => {
    const headers = new Headers(init.headers || {});
    headers.set("Authorization", `Bearer ${accessToken}`);
    return fetch(url, { ...init, headers });
  };
  let response = await send();
  if (response.status === 401) {
    // Only a live GIS client can mint a replacement token; a restored token
    // still works for reads until it expires even when GIS did not load.
    if (!tokenClient) throw new Error("The Google session expired; sign in again");
    await requestAccessToken({ prompt: "none" });
    response = await send();
  }
  return response;
}

async function refreshTokenIfExpired() {
  if (accessTokenExpiresAt && Date.now() >= accessTokenExpiresAt) {
    if (!tokenClient) {
      throw new Error("The Google session expired; sign in again");
    }
    await requestAccessToken({ prompt: "none" });
  }
}

export async function createFolder(name, parent) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(
    "https://www.googleapis.com/drive/v3/files?fields=id,name",
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name,
        mimeType: "application/vnd.google-apps.folder",
        parents: parent ? [parent] : undefined,
      }),
    },
  );
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive folder ${response.status}: ${body}`);
  }
  return body;
}

export async function createTextFile(name, parent, content) {
  if (!accessToken) throw new Error("Sign in first");
  const boundary = `tft-phase0-${crypto.randomUUID()}`;
  const metadata = { name, parents: [parent], mimeType: "text/plain" };
  const body = [
    `--${boundary}`,
    "Content-Type: application/json; charset=UTF-8",
    "",
    JSON.stringify(metadata),
    `--${boundary}`,
    "Content-Type: text/plain; charset=UTF-8",
    "",
    content,
    `--${boundary}--`,
    "",
  ].join("\r\n");

  const response = await authorizedFetch(
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,modifiedTime",
    {
      method: "POST",
      headers: { "Content-Type": `multipart/related; boundary=${boundary}` },
      body,
    },
  );
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`Drive upload ${response.status}: ${text}`);
  }
  return text;
}

export async function copyText(text) {
  await navigator.clipboard.writeText(text);
}

export function internalLinkAt(target) {
  let node = target;
  if (!(node instanceof Element)) node = node?.parentElement;
  const anchor = node?.closest?.("a.internal-link");
  if (!anchor) return "";
  return anchor.getAttribute("data-href") || anchor.getAttribute("href") || "";
}

export function confirmAction(message) {
  return window.confirm(message);
}

export async function renameFile(id, name) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(
    `https://www.googleapis.com/drive/v3/files/${id}?fields=id,name,etag`,
    {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    },
  );
  const body = await response.text();
  if (!response.ok) throw new Error(`Drive rename ${response.status}: ${body}`);
  return body;
}

export function sanitizeHtml(html) {
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content
    .querySelectorAll("script,style,iframe,object,embed,link,meta,base,form,button,textarea,select")
    .forEach((node) => node.remove());
  for (const node of template.content.querySelectorAll("*")) {
    for (const attr of [...node.attributes]) {
      if (/^on/i.test(attr.name) || attr.name === "srcdoc") {
        node.removeAttribute(attr.name);
      }
      if (
        (attr.name === "href" ||
          attr.name === "src" ||
          attr.name.endsWith(":href")) &&
        /^\s*(javascript|data:text\/html)/i.test(attr.value)
      ) {
        node.removeAttribute(attr.name);
      }
      if (attr.name === "xlink:href") node.removeAttribute(attr.name);
    }
    if (node.tagName === "INPUT" && (node.getAttribute("type") || "").toLowerCase() !== "checkbox") {
      node.remove();
    }
  }
  return template.innerHTML;
}

export async function driveBlobUrl(id) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(
    `https://www.googleapis.com/drive/v3/files/${id}?alt=media&supportsAllDrives=true`,
    {},
  );
  if (!response.ok) throw new Error(`Drive media ${response.status}`);
  return URL.createObjectURL(await response.blob());
}

export async function driveFileBytes(id) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(
    `https://www.googleapis.com/drive/v3/files/${id}?alt=media&supportsAllDrives=true`,
    {},
  );
  if (!response.ok) throw new Error(`Drive file ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

function audioEntry(id, url) {
  let entry = audioPlayers.get(id);
  if (!entry || entry.url !== url) {
    entry?.audio.pause();
    const audio = new Audio(url);
    audio.preload = "metadata";
    entry = { audio, url, error: "" };
    audio.addEventListener("error", () => {
      entry.error = audio.error?.message || "The browser could not play this audio file.";
    });
    audioPlayers.set(id, entry);
  }
  return entry;
}

function audioStateJson(entry) {
  const audio = entry.audio;
  return JSON.stringify({
    playing: !audio.paused && !audio.ended,
    position: Number.isFinite(audio.currentTime) ? audio.currentTime : 0,
    duration: Number.isFinite(audio.duration) ? audio.duration : 0,
    muted: audio.muted,
    error: entry.error,
  });
}

export function toggleAudio(id, url) {
  const entry = audioEntry(id, url);
  if (entry.audio.paused || entry.audio.ended) {
    entry.error = "";
    // Called synchronously from the UI click handler to preserve browser
    // user activation required by HTMLMediaElement.play().
    entry.audio.play().catch((error) => {
      entry.error = error?.message || "Audio playback was blocked.";
    });
  } else {
    entry.audio.pause();
  }
  return audioStateJson(entry);
}

export function audioState(id) {
  const entry = audioPlayers.get(id);
  return entry ? audioStateJson(entry) : "{}";
}

export function seekAudio(id, position) {
  const entry = audioPlayers.get(id);
  if (!entry || !Number.isFinite(position)) return;
  entry.audio.currentTime = Math.max(0, Math.min(position, entry.audio.duration || position));
}

export function toggleAudioMute(id, url) {
  const entry = audioEntry(id, url);
  entry.audio.muted = !entry.audio.muted;
  return audioStateJson(entry);
}

export function openVideoPlayer(url, title) {
  if (videoViewer) {
    videoViewer.querySelector("video")?.pause();
    videoViewer.remove();
  }
  const overlay = document.createElement("div");
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  overlay.setAttribute("aria-label", title || "Video player");
  Object.assign(overlay.style, {
    position: "fixed",
    inset: "0",
    zIndex: "2147483647",
    display: "flex",
    flexDirection: "column",
    background: "rgba(0, 0, 0, .92)",
    color: "#fff",
    padding: "16px",
    boxSizing: "border-box",
  });
  const header = document.createElement("div");
  Object.assign(header.style, {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "16px",
    minHeight: "40px",
    marginBottom: "12px",
    font: "14px system-ui, sans-serif",
  });
  const label = document.createElement("span");
  label.textContent = title || "Video";
  const close = document.createElement("button");
  close.type = "button";
  close.textContent = "Close";
  close.setAttribute("aria-label", "Close video player");
  Object.assign(close.style, {
    color: "inherit",
    background: "#333",
    border: "1px solid #666",
    borderRadius: "6px",
    padding: "6px 12px",
    cursor: "pointer",
    font: "inherit",
  });
  const video = document.createElement("video");
  video.src = url;
  video.controls = true;
  video.autoplay = true;
  video.playsInline = true;
  video.preload = "metadata";
  Object.assign(video.style, {
    width: "100%",
    height: "calc(100% - 52px)",
    minHeight: "0",
    objectFit: "contain",
    background: "#000",
  });
  const closeViewer = () => {
    video.pause();
    overlay.remove();
    if (videoViewer === overlay) videoViewer = null;
    document.removeEventListener("keydown", onKeyDown, true);
  };
  const onKeyDown = (event) => {
    if (event.key === "Escape") closeViewer();
  };
  close.addEventListener("click", closeViewer);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) closeViewer();
  });
  document.addEventListener("keydown", onKeyDown, true);
  header.append(label, close);
  overlay.append(header, video);
  document.body.append(overlay);
  videoViewer = overlay;
  close.focus();
  video.play().catch(() => {});
}

export async function updateTextFile(id, content, etag) {
  if (!accessToken) throw new Error("Sign in first");
  const headers = { "Content-Type": "text/plain; charset=UTF-8" };
  if (etag) headers["If-Match"] = etag;
  const response = await authorizedFetch(
    `https://www.googleapis.com/upload/drive/v3/files/${id}?uploadType=media&fields=id,modifiedTime&supportsAllDrives=true`,
    { method: "PATCH", headers, body: content },
  );
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive update ${response.status}: ${body}`);
  }
  return body;
}

export function getPref(key, fallback) {
  try {
    return localStorage.getItem(`tft.${key}`) ?? fallback;
  } catch {
    return fallback;
  }
}

export function setPref(key, value) {
  try {
    localStorage.setItem(`tft.${key}`, String(value));
  } catch {}
}

export function prefersDark() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function savedVault() {
  try {
    return localStorage.getItem("tft.vault") || "";
  } catch {
    return "";
  }
}

export function saveVault(id, name) {
  try {
    localStorage.setItem("tft.vault", JSON.stringify({ id, name }));
  } catch {}
}

export function savedWorkspace(rootId) {
  try {
    return localStorage.getItem(`tft.workspace.${rootId}`) || "";
  } catch {
    return "";
  }
}

export function saveWorkspace(rootId, state) {
  try {
    localStorage.setItem(`tft.workspace.${rootId}`, state);
    return true;
  } catch {
    return false;
  }
}

export function clearWorkspace(rootId, state) {
  try {
    const key = `tft.workspace.${rootId}`;
    if (localStorage.getItem(key) === state) localStorage.removeItem(key);
  } catch {}
}

export async function deleteFile(id) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(`https://www.googleapis.com/drive/v3/files/${id}`, {
    method: "DELETE",
  });
  if (!response.ok) {
    throw new Error(`Drive delete ${response.status}: ${await response.text()}`);
  }
  return "";
}

export async function duplicateFile(id, parent, name) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(
    `https://www.googleapis.com/drive/v3/files/${encodeURIComponent(id)}/copy?fields=id,name,mimeType,size,modifiedTime,createdTime,etag`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, parents: [parent] }),
    },
  );
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive duplicate ${response.status}: ${body}`);
  }
  return body;
}

export async function moveFile(id, addParent, removeParent) {
  if (!accessToken) throw new Error("Sign in first");
  const query = new URLSearchParams({
    addParents: addParent,
    removeParents: removeParent,
    fields: "id,name,parents,etag",
  });
  const response = await authorizedFetch(
    `https://www.googleapis.com/drive/v3/files/${encodeURIComponent(id)}?${query}`,
    { method: "PATCH" },
  );
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive move ${response.status}: ${body}`);
  }
  return body;
}

export async function trashFile(id) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await authorizedFetch(
    `https://www.googleapis.com/drive/v3/files/${encodeURIComponent(id)}?fields=id,trashed`,
    {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ trashed: true }),
    },
  );
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive trash ${response.status}: ${body}`);
  }
  return body;
}
