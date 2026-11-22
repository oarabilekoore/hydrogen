const ids = ["enabled", "captureArticles", "captureChats"];
(async () => {
  const values = await chrome.storage.local.get({ enabled: true, captureArticles: true, captureChats: true, apiBase: "http://127.0.0.1:17891", excludedHosts: [], apiToken: "" });
  ids.forEach(id => { document.getElementById(id).checked = !!values[id]; });
  document.getElementById("excludedHosts").value = (values.excludedHosts || []).join(", ");
  document.getElementById("apiBase").value = values.apiBase;
  document.getElementById("apiToken").placeholder = values.apiToken ? "Token saved. Enter a new one to replace it." : "Paste token from hydrogen --print-api-token";
})();
ids.forEach(id => document.getElementById(id).addEventListener("change", async event => {
  await chrome.storage.local.set({ [id]: event.target.checked });
}));
document.getElementById("excludedHosts").addEventListener("change", async event => {
  const excludedHosts = [...new Set(event.target.value
    .split(",")
    .map(value => value.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, ""))
    .filter(value => /^[a-z0-9.-]+$/.test(value)))];
  event.target.value = excludedHosts.join(", ");
  await chrome.storage.local.set({ excludedHosts });
  document.getElementById("status").textContent = "Domain exclusions saved.";
});
document.getElementById("capture").addEventListener("click", async () => {
  const status = document.getElementById("status");
  status.textContent = "Capturing page…";
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  try {
    const result = await chrome.tabs.sendMessage(tab.id, { type: "CAPTURE_NOW" });
    status.textContent = result?.ok ? "Capture requested. Check Hydrogen search." : (result?.error || "Nothing could be captured.");
  } catch (error) {
    status.textContent = "This page cannot be captured: " + error.message;
  }
});

document.getElementById("importHistory").addEventListener("click", async () => {
  const status = document.getElementById("status");
  status.textContent = "Importing recent history. Eligible pages open in background tabs one at a time…";
  try {
    const result = await chrome.runtime.sendMessage({ type: "IMPORT_HISTORY", days: 30, limit: 25 });
    if (result?.error) throw new Error(result.error);
    status.textContent = "History import finished · " + result.captured + " pages captured · " + result.skipped + " skipped · " + result.failed + " failed.";
  } catch (error) {
    status.textContent = "History import failed: " + error.message;
  }
});

document.getElementById("apiBase").addEventListener("change", async event => {
  await chrome.storage.local.set({ apiBase: event.target.value.trim() || "http://127.0.0.1:17891" });
  document.getElementById("status").textContent = "Hydrogen service address saved.";
});


document.getElementById("pair").addEventListener("click", async () => {
  const status = document.getElementById("status");
  const token = document.getElementById("apiToken").value.trim();
  const apiBase = (document.getElementById("apiBase").value.trim() || "http://127.0.0.1:17891").replace(/\/$/, "");
  if (!token) { status.textContent = "Enter the API token first."; return; }
  status.textContent = "Checking Hydrogen token…";
  try {
    const response = await fetch(apiBase + "/api/settings", {
      cache: "no-store",
      headers: { "Authorization": "Bearer " + token }
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error || "Hydrogen rejected this token.");
    await chrome.storage.local.set({ apiToken: token, apiBase });
    document.getElementById("apiToken").value = "";
    document.getElementById("apiToken").placeholder = "Token saved.";
    status.textContent = "Paired with Hydrogen successfully.";
  } catch (error) {
    status.textContent = "Pairing failed: " + error.message;
  }
});


document.getElementById("ignoreSite").addEventListener("click", async () => {
  const status = document.getElementById("status");
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (!tab?.url || !/^https?:/.test(tab.url)) throw new Error("This tab has no website domain to ignore.");
    const { apiBase = "http://127.0.0.1:17891", apiToken = "" } = await chrome.storage.local.get({ apiBase: "http://127.0.0.1:17891", apiToken: "" });
    if (!apiToken) throw new Error("Pair the extension with Hydrogen first.");
    const response = await fetch(String(apiBase).replace(/\/$/, "") + "/api/rules/ignore", {
      method: "POST", headers: { "Content-Type": "application/json", "Authorization": "Bearer " + apiToken },
      body: JSON.stringify({ url: tab.url })
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error || "Hydrogen could not update its rules.");
    status.textContent = "Ignored " + body.domain + " and its subdomains.";
  } catch (error) { status.textContent = "Could not ignore site: " + error.message; }
});
