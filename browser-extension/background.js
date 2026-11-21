const DEFAULTS={endpoint:"http://127.0.0.1:17843",captureArticles:true,captureChats:true};
async function settings(){return {...DEFAULTS,...await browser.storage.local.get(DEFAULTS)}}
browser.runtime.onMessage.addListener(async(message)=>{
  if(!message||message.type!=="hydrogen-capture")return;
  const cfg=await settings();
  if(message.kind==="article"&&!cfg.captureArticles)return {status:"disabled"};
  if(message.kind==="conversation"&&!cfg.captureChats)return {status:"disabled"};
  try{
    const route=message.kind==="article"?"/api/web-pages":"/api/capture/conversation";
    const response=await fetch(cfg.endpoint.replace(/\/$/,"")+route,{
      method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(message.payload)
    });
    const result=await response.json().catch(()=>({error:"Invalid Hydrogen response"}));
    if(!response.ok)throw new Error(result.error||response.statusText);
    return result;
  }catch(error){return {status:"unavailable",error:error.message}}
});
