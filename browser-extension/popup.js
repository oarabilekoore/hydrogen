document.getElementById("save").onclick=async()=>{const status=document.getElementById("status");status.textContent="Saving…";try{const [tab]=await browser.tabs.query({active:true,currentWindow:true});const result=await browser.tabs.sendMessage(tab.id,{type:"hydrogen-save-current"});status.textContent=result?.error||result?.reason||result?.status||"Sent to Hydrogen";}catch(error){status.textContent="Could not capture this page. Reload the tab and try again.";}};
document.getElementById("history").onclick=async()=>{
  const status=document.getElementById("status");status.textContent="Importing recent history…";
  try{const result=await browser.runtime.sendMessage({type:"hydrogen-history-import",days:30});status.textContent=result?.error||("Imported "+(result?.entries??0)+" history entries.");}
  catch(error){status.textContent="History import failed: "+error.message}
};
