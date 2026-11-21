const DEFAULTS={endpoint:"http://127.0.0.1:17843",captureArticles:true,captureChats:true};
async function settings(){return {...DEFAULTS,...await browser.storage.local.get(DEFAULTS)}}
browser.runtime.onMessage.addListener(async(message)=>{
  if(message?.type==="hydrogen-history-import"){
    const cfg=await settings();
    try{
      const days=Math.max(1,Math.min(365,Number(message.days)||30));
      const items=await browser.history.search({text:"",startTime:Date.now()-days*86400000,maxResults:500});
      const entries=items.filter(item=>/^https?:/.test(item.url||"")&&!/(^|\.)((bank|banking|payments?|checkout|paypal|stripe)\.)/i.test(new URL(item.url).hostname))
        .map(item=>({url:item.url,title:item.title||"",lastVisitTime:item.lastVisitTime||0,visitCount:item.visitCount||1,typedCount:item.typedCount||0}));
      const response=await fetch(cfg.endpoint.replace(/\/$/,"")+"/api/history/import",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({entries})});
      const result=await response.json().catch(()=>({error:"Invalid Hydrogen response"}));
      if(!response.ok)throw new Error(result.error||response.statusText);
      return result;
    }catch(error){return {status:"unavailable",error:error.message}}
  }
  if(!message||message.type!=="hydrogen-capture")return;
  const cfg=await settings();
  if(message.kind==="article"&&!cfg.captureArticles)return {status:"disabled"};
  if(message.kind==="conversation"&&!cfg.captureChats)return {status:"disabled"};
  try{
    const route=message.kind==="article"?"/api/web-pages":message.kind==="feed"?"/api/feeds":"/api/capture/conversation";
    const response=await fetch(cfg.endpoint.replace(/\/$/,"")+route,{
      method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(message.payload)
    });
    const result=await response.json().catch(()=>({error:"Invalid Hydrogen response"}));
    if(!response.ok)throw new Error(result.error||response.statusText);
    return result;
  }catch(error){return {status:"unavailable",error:error.message}}
});
