const API = "http://127.0.0.1:17891";
const defaults = { enabled: true, captureArticles: true, captureChats: true, excludedHosts: [] };

async function settings() {
  const stored = await chrome.storage.local.get(defaults);
  return { ...defaults, ...stored };
}

async function post(path, payload) {
  const response = await fetch(API + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload)
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error || `Hydrogen returned HTTP ${response.status}`);
  return body;
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  (async () => {
    if (message.type === "GET_CONFIG") {
      const response = await fetch(API + "/api/settings");
      if (!response.ok) throw new Error("Hydrogen settings are unavailable");
      const data = await response.json();
      sendResponse({ ok: true, web: data.settings?.web || {} });
      return;
    }
    const prefs = await settings();
    if (!prefs.enabled) throw new Error("Capture is paused");
    const host = new URL(sender.url || message.url || "https://invalid.local").hostname;
    if (prefs.excludedHosts.some(item => host === item || host.endsWith("." + item))) {
      throw new Error("This domain is excluded");
    }
    if (message.type === "ARTICLE" && prefs.captureArticles) {
      const result = await post("/api/web-pages", message.payload);
      sendResponse({ ok: true, result });
    } else if (message.type === "CONVERSATION" && prefs.captureChats) {
      const result = await post("/api/capture/conversation", message.payload);
      sendResponse({ ok: true, result });
    } else {
      sendResponse({ ok: false, error: "Capture type disabled" });
    }
  })().catch(error => sendResponse({ ok: false, error: error.message }));
  return true;
});

chrome.runtime.onInstalled.addListener(async () => {
  const current = await chrome.storage.local.get(null);
  await chrome.storage.local.set({ ...defaults, ...current });
});