(() => {
  if (!/^https?:$/.test(location.protocol)) return;
  const host=location.hostname.toLowerCase();
  const excluded=/^(localhost|127\.0\.0\.1|::1)$/.test(host)
    || /(^|\.)(bank|banking|payments?|checkout|paypal|stripe)\./.test(host)
    || /(^|\.)(accounts\.google|login\.live|auth0)\./.test(host);
  if(excluded)return;
  const send=(kind,payload)=>browser.runtime.sendMessage({type:"hydrogen-capture",kind,payload}).catch(()=>{});
  browser.runtime.onMessage.addListener((message)=>{
    if(!message||message.type!=="hydrogen-save-current")return;
    const text=clean();
    if(!text)return {status:"skipped",reason:"No readable page text found"};
    const payload={
      url:location.href,canonicalUrl:canonical,title:document.title,
      author:document.querySelector('meta[name="author"]')?.content||"",
      publishedAt:document.querySelector('meta[property="article:published_time"]')?.content||"",
      siteName:document.querySelector('meta[property="og:site_name"]')?.content||host,
      text,html:"",manual:true
    };
    return send("article",payload);
  });
  const canonical=document.querySelector('link[rel="canonical"]')?.href||location.href;
  const clean=()=> {
    const root=document.querySelector("article")||document.querySelector("main")||document.body;
    const clone=root.cloneNode(true);
    clone.querySelectorAll("nav,header,footer,aside,form,button,script,style,noscript,[role=navigation],[aria-hidden=true],.advertisement,.ads").forEach(n=>n.remove());
    return clone.innerText.replace(/\n[ \t]+/g,"\n").replace(/\n{3,}/g,"\n\n").trim();
  };
  const platform=()=>{
    if(/(^|\.)chatgpt\.com$/.test(host))return "chatgpt";
    if(/(^|\.)gemini\.google\.com$/.test(host))return "gemini";
    if(/(^|\.)claude\.ai$/.test(host))return "claude";
    return null;
  };
  const extractChat=()=>{
    const p=platform();if(!p)return null;
    let nodes=[];
    if(p==="chatgpt")nodes=[...document.querySelectorAll("[data-message-author-role]")].map(n=>({role:n.getAttribute("data-message-author-role")==="user"?"user":"assistant",content:n.innerText.trim()}));
    if(p==="gemini"){
      nodes=[...document.querySelectorAll("user-query, model-response, .user-query, .model-response-text, [data-test-id*=user-query], [data-test-id*=response], .query-text")].map(n=>{
        const tag=n.tagName.toLowerCase(), cls=String(n.className||"");
        const user=tag==="user-query"||cls.includes("user-query")||n.matches("[data-test-id*=user-query],.query-text");
        return {role:user?"user":"assistant",content:n.innerText.trim()};
      });
    }
    if(p==="claude")nodes=[...document.querySelectorAll('[data-testid="chat-message"], .font-user-message, .font-claude-message')].map(n=>({role:n.className.toString().includes("user")?"user":"assistant",content:n.innerText.trim()}));
    const messages=nodes.filter(n=>n.content&&n.content.length>0);
    if(!messages.length)return null;
    return {platform:p,url:location.href,title:document.title,messages};
  };
  const chat=extractChat();
  if(chat){
    let chatTimer=0;
    const captureChat=()=>{const current=extractChat();if(current)send("conversation",current)};
    captureChat();
    const chatRoot=document.querySelector("main")||document.body;
    new MutationObserver(()=>{
      clearTimeout(chatTimer);
      chatTimer=setTimeout(captureChat,1200);
    }).observe(chatRoot,{childList:true,subtree:true,characterData:true});
    return;
  }
  const text=clean();
  const paragraphs=text.split(/\n\s*\n/).filter(p=>p.trim().length>40);
  if(text.length<800||paragraphs.length<4)return;
  const payload={
    url:location.href,canonicalUrl:canonical,title:document.title,
    author:document.querySelector('meta[name="author"]')?.content||document.querySelector('[rel=author]')?.textContent?.trim()||"",
    publishedAt:document.querySelector('meta[property="article:published_time"]')?.content||document.querySelector('time[datetime]')?.getAttribute("datetime")||"",
    siteName:document.querySelector('meta[property="og:site_name"]')?.content||host,
    text,html:"",manual:false
  };
  const key="hydrogen-article:"+canonical;
  if(!sessionStorage.getItem(key)){sessionStorage.setItem(key,"1");send("article",payload);}
})();
