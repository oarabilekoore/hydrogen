(() => {
  if (window.__hydrogenCaptureInstalled) return;
  window.__hydrogenCaptureInstalled = true;

  const excludedPath = /\/(login|signin|sign-in|checkout|payment|account|settings|admin)(\/|$)/i;
  const excludedHost = /(^|\.)(bank|banking|paypal|stripe)\./i;
  let timer = null;
  let lastArticleHash = "";
  let lastConversationHash = "";

  function cleanText(node) {
    const clone = node.cloneNode(true);
    clone.querySelectorAll("script,style,noscript,nav,header,footer,aside,form,button,input,textarea,select,[role=navigation],[aria-hidden=true]").forEach(el => el.remove());
    return (clone.innerText || clone.textContent || "").replace(/\n{3,}/g, "\n\n").trim();
  }

  function articleCandidate() {
    if (typeof Readability === "function") {
      try {
        const parsed = new Readability(document.cloneNode(true), {
          charThreshold: 300,
          keepClasses: false
        }).parse();
        if (parsed?.textContent?.trim()) {
          const articleDocument = new DOMParser().parseFromString(parsed.content || "", "text/html");
          return {
            text: parsed.textContent.replace(/\n{3,}/g, "\n\n").trim(),
            paragraphs: articleDocument.querySelectorAll("p").length,
            metadata: {
              title: parsed.title || "",
              author: parsed.byline || "",
              published_at: parsed.publishedTime || "",
              site_name: parsed.siteName || ""
            }
          };
        }
      } catch (error) {
        console.debug("Hydrogen Readability extraction failed; using structural fallback.", error);
      }
    }

    const candidates = [...document.querySelectorAll("article,main,[role=main],.post-content,.article-content")];
    const ranked = candidates.map(node => ({
      text: cleanText(node),
      paragraphs: node.querySelectorAll("p").length
    })).sort((a, b) => b.text.length - a.text.length);
    return ranked[0] || { text: cleanText(document.body), paragraphs: document.querySelectorAll("p").length, metadata: {} };
  }

  function isSupportedChatHost() {
    const host = location.hostname.toLowerCase();
    return [
      "chatgpt.com",
      "chat.openai.com",
      "claude.ai",
      "gemini.google.com",
      "perplexity.ai",
      "deepseek.com",
      "copilot.microsoft.com",
      "bing.com",
      "grok.com",
      "poe.com",
      "chat.mistral.ai"
    ].some(domain => host === domain || host.endsWith("." + domain));
  }

  function pageMetadata(article = {}) {
    const get = selector => document.querySelector(selector)?.content?.trim() || "";
    const canonical = document.querySelector('link[rel="canonical"]')?.href || location.href;
    return {
      url: location.href,
      canonical_url: canonical,
      title: get('meta[property="og:title"]') || article.title || document.title,
      author: get('meta[name="author"]') || get('meta[property="article:author"]') || article.author || null,
      published_at: get('meta[property="article:published_time"]') || get('meta[name="date"]') || article.published_at || null,
      site_name: get('meta[property="og:site_name"]') || article.site_name || location.hostname
    };
  }

  async function captureArticle(force = false) {
    if (excludedPath.test(location.pathname) || excludedHost.test(location.hostname)) return;
    const appPath = /\/(app|editor|workspace|canvas|playground|ide|dashboard|studio)(\/|$)/i;
    if (!force && appPath.test(location.pathname)) return;
    const candidate = articleCandidate();
    const { text, paragraphs } = candidate;
    const canvases = document.querySelectorAll("canvas").length;
    const controls = document.querySelectorAll("button,input,textarea,select,[contenteditable=true]").length;
    if (!force && (text.length < 800 || paragraphs < 4)) return;
    if (!force && ((canvases > 0 && paragraphs < 8) || (controls > 35 && paragraphs < 8))) return;
    if (text.length < 120) return;
    const meta = pageMetadata(candidate.metadata);
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(meta.canonical_url + "\n" + text));
    const hash = [...new Uint8Array(digest)].map(v => v.toString(16).padStart(2, "0")).join("");
    if (hash === lastArticleHash) return;
    lastArticleHash = hash;
    chrome.runtime.sendMessage({
      type: "ARTICLE",
      payload: { ...meta, content: text, content_hash: hash, paragraph_count: paragraphs, manual: force }
    });
  }

  function roleFromNode(node) {
    const details = [
      node.getAttribute("data-message-author-role"),
      node.getAttribute("data-role"),
      node.getAttribute("data-message-role"),
      node.getAttribute("data-testid"),
      node.getAttribute("aria-label"),
      typeof node.className === "string" ? node.className : ""
    ].filter(Boolean).join(" ").toLowerCase();
    if (/(user|human|prompt|question)/.test(details)) return "user";
    if (/(assistant|bot|answer|response|model)/.test(details)) return "assistant";
    return null;
  }

  function chatMessages() {
    const host = location.hostname.toLowerCase();
    let nodes = [];

    if (host.includes("chatgpt.com") || host.includes("chat.openai.com")) {
      nodes = [...document.querySelectorAll("[data-message-author-role]")].map(node => ({
        role: roleFromNode(node),
        content: cleanText(node.querySelector(".markdown") || node)
      }));
    } else if (host.includes("claude.ai")) {
      nodes = [...document.querySelectorAll('[data-testid^="conversation-turn"]')].map((node, index) => ({
        role: node.querySelector('[data-testid*="human"]') ? "user" : (index % 2 === 0 ? "user" : "assistant"),
        content: cleanText(node)
      }));
    } else if (host.includes("gemini.google.com")) {
      nodes = [...document.querySelectorAll("user-query, model-response")].map(node => ({
        role: node.matches("user-query") ? "user" : "assistant",
        content: cleanText(node)
      }));
    } else if (host.includes("perplexity.ai")) {
      nodes = [...document.querySelectorAll('[data-testid*="user-message"],[data-testid*="answer"],[data-testid*="question"]')].map(node => ({
        role: roleFromNode(node),
        content: cleanText(node)
      }));
    } else if (host.includes("chat.deepseek.com") || host.includes("deepseek.com")) {
      nodes = [...document.querySelectorAll("[data-message-author-role],[data-role],[data-testid*='message']")].map(node => ({
        role: roleFromNode(node),
        content: cleanText(node.querySelector(".markdown,.ds-markdown") || node)
      }));
    } else if (host.includes("copilot.microsoft.com") || host.includes("bing.com")) {
      nodes = [...document.querySelectorAll("cib-message-group,[data-testid*='message']")].map(node => ({
        role: roleFromNode(node),
        content: cleanText(node.querySelector("cib-message, .ac-textBlock") || node)
      }));
    } else if (host.includes("grok.com")) {
      nodes = [...document.querySelectorAll("[data-message-author-role],[data-testid*='message'],[data-testid*='response']")].map(node => ({
        role: roleFromNode(node),
        content: cleanText(node.querySelector(".message-bubble,.prose") || node)
      }));
    } else if (host.includes("poe.com")) {
      nodes = [...document.querySelectorAll(".Message_humanMessageBubble,.Message_botMessageBubble")].map(node => ({
        role: node.classList.contains("Message_humanMessageBubble") ? "user" : "assistant",
        content: cleanText(node)
      }));
    } else if (host.includes("chat.mistral.ai")) {
      nodes = [...document.querySelectorAll("[data-message-author-role],[data-testid*='message']")].map(node => ({
        role: roleFromNode(node),
        content: cleanText(node.querySelector(".prose,.markdown") || node)
      }));
    }

    const seen = new Set();
    return nodes.filter(item => {
      if (!["user", "assistant"].includes(item.role) || !item.content) return false;
      const key = item.role + ":" + item.content;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }

  async function captureConversation() {
    if (excludedPath.test(location.pathname)) return;
    const messages = chatMessages();
    if (messages.length < 2) return;
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(messages)));
    const hash = [...new Uint8Array(digest)].map(v => v.toString(16).padStart(2, "0")).join("");
    if (hash === lastConversationHash) return;
    lastConversationHash = hash;
    chrome.runtime.sendMessage({
      type: "CONVERSATION",
      payload: {
        source_key: location.origin + location.pathname,
        title: document.title || "Captured AI conversation",
        messages
      }
    });
  }

  function scheduleCapture() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      captureConversation();
      captureArticle();
    }, 1800);
  }

  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (message?.type === "CAPTURE_NOW") {
      Promise.all([captureArticle(true), captureConversation()])
        .then(() => sendResponse({ ok: true }))
        .catch(error => sendResponse({ ok: false, error: error.message }));
      return true;
    }
  });

  scheduleCapture();
  const observer = new MutationObserver(scheduleCapture);
  observer.observe(document.documentElement, { childList: true, subtree: true });
  window.addEventListener("pagehide", () => observer.disconnect(), { once: true });
})();