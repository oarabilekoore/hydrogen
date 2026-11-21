(() => {
  if (!/^https?:$/.test(location.protocol)) return;

  const host = location.hostname.toLowerCase();
  const excluded = /^(localhost|127\.0\.0\.1|::1)$/.test(host)
    || /(^|\.)(bank|banking|payments?|checkout|paypal|stripe)\./.test(host)
    || /(^|\.)(accounts\.google|login\.live|auth0)\./.test(host);
  if (excluded) return;

  const send = (kind, payload) =>
    browser.runtime.sendMessage({ type: "hydrogen-capture", kind, payload }).catch(() => {});
  const canonical = document.querySelector('link[rel="canonical"]')?.href || location.href;

  function cleanArticle(manual = false) {
    const root = document.querySelector("article") || document.querySelector("main") || (manual ? document.body : null);
    if (!root) return "";
    const clone = root.cloneNode(true);
    clone.querySelectorAll(
      "nav,header,footer,aside,form,button,script,style,noscript,canvas,svg,[role=navigation],[aria-hidden=true],.advertisement,.ads,[class*=advert],[id*=cookie]"
    ).forEach(node => node.remove());
    const paragraphs = [...clone.querySelectorAll("p")]
      .map(node => node.innerText.trim())
      .filter(text => text.length > 40);
    const extracted = paragraphs.length >= 4 ? paragraphs.join("\n\n") : clone.innerText;
    return extracted.replace(/\n[ \t]+/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
  }

  function articlePayload(manual = false) {
    const text = cleanArticle(manual);
    const paragraphs = text.split(/\n\s*\n/).filter(part => part.trim().length > 40);
    if (!manual && (text.length < 800 || paragraphs.length < 4)) return null;
    if (!text) return null;
    return {
      url: location.href,
      canonicalUrl: canonical,
      title: document.title,
      author: document.querySelector('meta[name="author"]')?.content
        || document.querySelector('[rel=author]')?.textContent?.trim() || "",
      publishedAt: document.querySelector('meta[property="article:published_time"]')?.content
        || document.querySelector('time[datetime]')?.getAttribute("datetime") || "",
      siteName: document.querySelector('meta[property="og:site_name"]')?.content || host,
      text,
      html: "",
      manual
    };
  }

  browser.runtime.onMessage.addListener(message => {
    if (!message || message.type !== "hydrogen-save-current") return;
    const payload = articlePayload(true);
    if (!payload) return { status: "skipped", reason: "No readable page text found" };
    return send("article", payload);
  });

  function platform() {
    if (/(^|\.)chatgpt\.com$/.test(host)) return "chatgpt";
    if (/(^|\.)gemini\.google\.com$/.test(host)) return "gemini";
    if (/(^|\.)claude\.ai$/.test(host)) return "claude";
    return null;
  }

  function extractChat() {
    const currentPlatform = platform();
    if (!currentPlatform) return null;
    let nodes = [];
    if (currentPlatform === "chatgpt") {
      nodes = [...document.querySelectorAll("[data-message-author-role]")].map(node => ({
        role: node.getAttribute("data-message-author-role") === "user" ? "user" : "assistant",
        content: node.innerText.trim()
      }));
    } else if (currentPlatform === "gemini") {
      nodes = [...document.querySelectorAll(
        "user-query,model-response,.user-query,.model-response-text,[data-test-id*=user-query],[data-test-id*=response],.query-text"
      )].map(node => {
        const tag = node.tagName.toLowerCase();
        const cls = String(node.className || "");
        const user = tag === "user-query" || cls.includes("user-query")
          || node.matches("[data-test-id*=user-query],.query-text");
        return { role: user ? "user" : "assistant", content: node.innerText.trim() };
      });
    } else if (currentPlatform === "claude") {
      nodes = [...document.querySelectorAll(
        '[data-testid="chat-message"],.font-user-message,.font-claude-message'
      )].map(node => ({
        role: String(node.className).includes("user") ? "user" : "assistant",
        content: node.innerText.trim()
      }));
    }
    const messages = nodes.filter(message => message.content);
    if (!messages.length) return null;
    return {
      platform: currentPlatform,
      url: location.href,
      title: document.title,
      messages
    };
  }

  const chat = extractChat();
  if (chat) {
    let timer = 0;
    const captureChat = () => {
      const current = extractChat();
      if (current) send("conversation", current);
    };
    captureChat();
    const chatRoot = document.querySelector("main") || document.body;
    new MutationObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(captureChat, 1200);
    }).observe(chatRoot, { childList: true, subtree: true, characterData: true });
    return;
  }

  const feedUrl = document.querySelector(
    'link[type="application/rss+xml"],link[type="application/atom+xml"]'
  )?.href;
  if (feedUrl) send("feed", { url: feedUrl });

  let articleTimer = 0;
  let lastSignature = "";
  const captureArticle = () => {
    const payload = articlePayload(false);
    if (!payload) return;
    let hash = 2166136261;
    for (let i = 0; i < payload.text.length; i++) {
      hash ^= payload.text.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
    const signature = (hash >>> 0).toString(16);
    if (signature === lastSignature) return;
    lastSignature = signature;
    send("article", payload);
  };

  captureArticle();
  const articleRoot = document.querySelector("article") || document.querySelector("main");
  if (articleRoot) {
    new MutationObserver(() => {
      clearTimeout(articleTimer);
      articleTimer = setTimeout(captureArticle, 5000);
    }).observe(articleRoot, { childList: true, subtree: true, characterData: true });
  }
})();
