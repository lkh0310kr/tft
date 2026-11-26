// Phase 0 Google Drive adapter: GIS token, Picker, and the few REST calls
// the spike needs. The Rust app owns the flow; this file only talks to Google.

let tokenClient = null;
let accessToken = null;
let pending = null;

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
  localStorage.setItem("tft.drive-config", JSON.stringify(config));
  window.TFT_CONFIG = { ...(window.TFT_CONFIG || {}), ...config };
}

export function currentToken() {
  return accessToken || "";
}

export function initTokenClient(clientId) {
  if (!window.google?.accounts?.oauth2) {
    throw new Error("Google Identity Services is not loaded");
  }
  tokenClient = google.accounts.oauth2.initTokenClient({
    client_id: clientId,
    scope: "https://www.googleapis.com/auth/drive.file",
    callback: (response) => {
      const p = pending;
      pending = null;
      if (response.error) {
        p?.reject(new Error(response.error));
      } else {
        accessToken = response.access_token;
        p?.resolve(accessToken);
      }
    },
  });
}

export function requestToken() {
  if (!tokenClient) {
    return Promise.reject(new Error("initTokenClient was not called"));
  }
  return new Promise((resolve, reject) => {
    pending = { resolve, reject };
    tokenClient.requestAccessToken({ prompt: "" });
  });
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
  const response = await fetch(url, {
    headers: { Authorization: `Bearer ${accessToken}` },
  });
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive ${response.status}: ${body}`);
  }
  return body;
}

export async function createFolder(name, parent) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await fetch(
    "https://www.googleapis.com/drive/v3/files?fields=id,name",
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
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

  const response = await fetch(
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,modifiedTime",
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": `multipart/related; boundary=${boundary}`,
      },
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

export function promptText(message, value = "") {
  const answer = window.prompt(message, value);
  return answer == null ? "" : answer;
}

export function confirmAction(message) {
  return window.confirm(message);
}

export async function renameFile(id, name) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await fetch(
    `https://www.googleapis.com/drive/v3/files/${id}?fields=id,name`,
    {
      method: "PATCH",
      headers: {
        Authorization: `Bearer ${accessToken}`,
        "Content-Type": "application/json",
      },
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
  const response = await fetch(
    `https://www.googleapis.com/drive/v3/files/${id}?alt=media`,
    { headers: { Authorization: `Bearer ${accessToken}` } },
  );
  if (!response.ok) throw new Error(`Drive media ${response.status}`);
  return URL.createObjectURL(await response.blob());
}

export async function updateTextFile(id, content, etag) {
  if (!accessToken) throw new Error("Sign in first");
  const headers = {
    Authorization: `Bearer ${accessToken}`,
    "Content-Type": "text/plain; charset=UTF-8",
  };
  if (etag) headers["If-Match"] = etag;
  const response = await fetch(
    `https://www.googleapis.com/upload/drive/v3/files/${id}?uploadType=media&fields=id,modifiedTime,etag`,
    { method: "PATCH", headers, body: content },
  );
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`Drive update ${response.status}: ${body}`);
  }
  return body;
}

export function getPref(key, fallback) {
  return localStorage.getItem(`tft.${key}`) ?? fallback;
}

export function setPref(key, value) {
  localStorage.setItem(`tft.${key}`, String(value));
}

export function prefersDark() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function savedVault() {
  return localStorage.getItem("tft.vault") || "";
}

export function saveVault(id, name) {
  localStorage.setItem("tft.vault", JSON.stringify({ id, name }));
}

export async function deleteFile(id) {
  if (!accessToken) throw new Error("Sign in first");
  const response = await fetch(`https://www.googleapis.com/drive/v3/files/${id}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${accessToken}` },
  });
  if (!response.ok) {
    throw new Error(`Drive delete ${response.status}: ${await response.text()}`);
  }
  return "";
}
