const DEFAULT_API = "http://127.0.0.1:17891";
async function apiBase() {
  const stored = await chrome.storage.local.get({ apiBase: DEFAULT_API });
  return String(stored.apiBase || DEFAULT_API).replace(/\/$/, "");
}
const defaults = { enabled: true, captureArticles: true, captureChats: true, excludedHosts: [] };
const historyImports = new Map();
let historyImportBusy = false;

function sensitiveUrl(raw, prefs) {
  try {
    const url = new URL(raw);
    if (!/^https?:$/.test(url.protocol)) return true;
    if (/\\/(login|signin|sign-in|checkout|payment|account|settings|admin)(\\/|$)/i.test(url.pathname)) return true;
    return prefs.excludedHosts.some(item => url.hostname === item || url.hostname.endsWith("." + item));
  } catch (_) { return true; }
}

function finishHistoryTab(tabId, captured) {
  const resolve = historyImports.get(tabId);
  if (resolve) {
    historyImports.delete(tabId);
    resolve(captured);
  }
}

chrome.tabs.onRemoved.addListener(tabId => finishHistoryTab(tabId, false));

async function importRecentHistory(days = 30, limit = 25) {
  if (historyImportBusy) throw new Error("A history import is already running.");
  const prefs = await settings();
  if (!prefs.enabled || !prefs.captureArticles) throw new Error("Enable automatic article capture before importing history.");
  const configResponse = await fetch((await apiBase()) + "/api/settings", { cache: "no-store" });
  if (!configResponse.ok) throw new Error("Hydrogen service is unavailable.");
  const appConfig = await configResponse.json();
  const web = appConfig.settings?.web || {};
  if (!web.enabled || !web.auto_index) throw new Error("Enable web.enabled and web.auto_index in Hydrogen settings before importing history.");
  historyImportBusy = true;
  const summary = { examined: 0, captured: 0, skipped: 0, failed: 0 };
  try {
    const startTime = Date.now() - Math.max(1, Math.min(Number(days) || 30, 365)) * 86400000;
    const entries = await chrome.history.search({ text: "", startTime, maxResults: Math.max(1, Math.min(Number(limit) || 25, 100)) });
    for (const entry of entries) {
      if (!entry.url || sensitiveUrl(entry.url, prefs)) { summary.skipped++; continue; }
      summary.examined++;
      let tab;
      try {
        tab = await chrome.tabs.create({ url: entry.url, active: false });
        const captured = await new Promise(resolve => {
          const timer = setTimeout(() => finishHistoryTab(tab.id, false), 7000);
          historyImports.set(tab.id, value => { clearTimeout(timer); resolve(value); });
        });
        if (captured) summary.captured++; else summary.skipped++;
      } catch (_) {
        summary.failed++;
      } finally {
        if (tab?.id != null) {
          finishHistoryTab(tab.id, false);
          try { await chrome.tabs.remove(tab.id); } catch (_) {}
        }
      }
    }
    return summary;
  } finally {
    historyImportBusy = false;
  }
}

async function settings() {
  const stored = await chrome.storage.local.get(defaults);
  return { ...defaults, ...stored };
}

async function post(path, payload) {
  const response = await fetch((await apiBase()) + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload)
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error || `Hydrogen returned HTTP ${response.status}`);
  return body;
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type === "IMPORT_HISTORY") {
    importRecentHistory(message.days, message.limit).then(sendResponse).catch(error => sendResponse({ error: error.message }));
    return true;
  }
  (async () => {
    const prefs = await settings();
    if (!prefs.enabled) throw new Error("Capture is paused");
    const host = new URL(sender.url || message.url || "https://invalid.local").hostname;
    if (prefs.excludedHosts.some(item => host === item || host.endsWith("." + item))) {
      throw new Error("This domain is excluded");
    }
    if (message.type === "ARTICLE" && prefs.captureArticles) {
      const result = await post("/api/web-pages", message.payload);
      if (sender.tab?.id != null) finishHistoryTab(sender.tab.id, true);
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