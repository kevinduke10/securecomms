import {startLogin,completeLogin} from './login';
type Reply={status:number;body:unknown};
export async function initializeBrowser():Promise<void>{
  if(!isSecureContext||!navigator.locks||!crypto.subtle)throw new Error('Cirra requires HTTPS and a browser with Web Crypto, IndexedDB, and Web Locks.');
  const worker=new Worker(new URL('./worker.ts',import.meta.url),{type:'module'});
  let next=0;
  const pending=new Map<number,{resolve:(value:unknown)=>void;reject:(error:Error)=>void}>();
  const ready=new Promise<void>((resolve,reject)=>{
    worker.onmessage=({data})=>{
      if(data.ready){resolve();return;}
      if(data.fatal){reject(new Error(data.fatal));for(const p of pending.values())p.reject(new Error(data.fatal));pending.clear();return;}
      const p=pending.get(data.id);if(!p)return;pending.delete(data.id);
      if(data.error)p.reject(new Error(data.error));else p.resolve(data.result);
    };
    worker.onerror=()=>{reject(new Error('The encrypted browser client could not start. Reload to retry.'));for(const p of pending.values())p.reject(new Error('Browser client stopped. Reload to retry.'));pending.clear();};
  });
  await ready;
  async function invoke<T>(command:string,args?:Record<string,unknown>):Promise<T>{
    if(command==='open_login'){await startLogin();return true as T;}
    if(command==='save_download'){
      const filename=String(args?.filename??'download');const bytes=Uint8Array.from(atob(String(args?.data)),c=>c.charCodeAt(0));
      const url=URL.createObjectURL(new Blob([bytes]));const a=document.createElement('a');a.href=url;a.download=filename;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);return true as T;
    }
    return new Promise<T>((resolve,reject)=>{const id=++next;pending.set(id,{resolve:value=>resolve(value as T),reject});worker.postMessage({id,command,args});});
  }
  window.__CIRRA_BROWSER__={invoke};
  const relay=import.meta.env.VITE_CIRRA_RELAY as string;
  if(!relay)throw new Error('This deployment is missing its relay configuration.');
  const state=await invoke<Reply>('client_request',{request:{method:'GET',path:'/desktop/status'}});
  const configured=state.body as {configured:boolean;relay?:string};
  if(configured.configured&&configured.relay!==relay)throw new Error('This browser device belongs to a different relay. Existing data was preserved.');
  if(!configured.configured){
    const r=await invoke<Reply>('client_request',{request:{method:'POST',path:'/desktop/configure',body:{relay}}});
    if(r.status!==200)throw new Error('Could not save the relay configuration.');
  }
  const access=await completeLogin();
  if(access)await invoke('sign_in',{access});
}
