const ids = ["enabled", "captureArticles", "captureChats"];
(async () => {
  const values = await chrome.storage.local.get({ enabled: true, captureArticles: true, captureChats: true });
  ids.forEach(id => { document.getElementById(id).checked = !!values[id]; });
})();
ids.forEach(id => document.getElementById(id).addEventListener("change", async event => {
  await chrome.storage.local.set({ [id]: event.target.checked });
}));
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