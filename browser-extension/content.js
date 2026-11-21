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
    let articleTimer=0;
  let lastArticleSignature="";
  const captureArticle=()=>{
    const text=clean();
    const paragraphs=text.split(/\n\s*\n/).filter(p=>p.trim().length>40);
    if(text.length<800||paragraphs.length<4)return;
    let hash=2166136261;
    for(let i=0;i<text.length;i++){hash^=text.charCodeAt(i);hash=Math.imul(hash,16777619)}
    const signature=(hash>>>0).toString(16);
    if(signature===lastArticleSignature)return;
    lastArticleSignature=signature;
    const payload={
      url:location.href,canonicalUrl:canonical,title:document.title,
      author:document.querySelector('meta[name="author"]')?.content||document.querySelector('[rel=author]')?.textContent?.trim()||"",
      publishedAt:document.querySelector('meta[property="article:published_time"]')?.content||document.querySelector('time[datetime]')?.getAttribute("datetime")||"",
      siteName:document.querySelector('meta[property="og:site_name"]')?.content||host,
      text,html:"",manual:false
    };
    send("article",payload);
  };
  captureArticle();
  const articleRoot=document.querySelector("article")||document.querySelector("main");
  if(articleRoot){
    new MutationObserver(()=>{
      clearTimeout(articleTimer);
      articleTimer=setTimeout(captureArticle,5000);
    }).observe(articleRoot,{childList:true,subtree:true,characterData:true});
  }
})();
