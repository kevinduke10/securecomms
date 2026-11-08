type NativeResponse = { status:number; body:unknown };
type Bridge = { core:{invoke:<T>(command:string,args?:Record<string,unknown>)=>Promise<T>} };
declare global { interface Window { __CIRRA_BROWSER__?:{invoke:<T>(command:string,args?:Record<string,unknown>)=>Promise<T>}; __CIRRA_IOS_ACTIVE__?:boolean; __TAURI__?:Bridge; __CIRRA_IOS__?:{invoke:<T>(command:string,args?:Record<string,unknown>)=>Promise<T>} } }
export const isNative = () => Boolean(window.__TAURI__?.core || window.__CIRRA_IOS__ || window.__CIRRA_BROWSER__);
export async function nativeCommand<T>(command:string,args?:Record<string,unknown>):Promise<T> {
  if(window.__CIRRA_BROWSER__) return window.__CIRRA_BROWSER__.invoke<T>(command,args);
  if(window.__CIRRA_IOS__) return window.__CIRRA_IOS__.invoke<T>(command,args);
  if (!window.__TAURI__?.core) throw new Error('The native client is unavailable.');
  return window.__TAURI__.core.invoke<T>(command,args);
}
export async function clientFetch(path:string,init:RequestInit={}):Promise<Response> {
  if (!isNative()) return fetch(path,init);
  const result=await nativeCommand<NativeResponse>('client_request',{request:{method:init.method || 'GET',path,body:typeof init.body==='string'?JSON.parse(init.body):null}});
  const binary=result.body as {binary_base64?:string;content_type?:string;filename?:string};
  if (result.status>=200&&result.status<300&&binary?.binary_base64!==undefined) {
    const data=Uint8Array.from(atob(binary.binary_base64),c=>c.charCodeAt(0));
    return new Response(data,{status:result.status,headers:{'content-type':binary.content_type || 'application/octet-stream','content-disposition':`attachment; filename*=UTF-8''${encodeURIComponent(binary.filename || 'download')}`}});
  }
  return new Response(JSON.stringify(result.body),{status:result.status,headers:{'content-type':'application/json'}});
}
export async function saveBlob(blob:Blob,filename:string):Promise<boolean>{
  if(isNative()){
    const bytes=new Uint8Array(await blob.arrayBuffer());let data='';
    for(let offset=0;offset<bytes.length;offset+=8192)data+=String.fromCharCode(...bytes.subarray(offset,offset+8192));
    return nativeCommand<boolean>('save_download',{filename,data:btoa(data)});
  }
  const url=URL.createObjectURL(blob);const anchor=document.createElement('a');anchor.href=url;anchor.download=filename;anchor.click();setTimeout(()=>URL.revokeObjectURL(url),1000);return true;
}
