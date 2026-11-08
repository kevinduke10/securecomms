/// <reference lib="webworker" />
import init,{BrowserDevice} from './generated/cirra_device';
import wasmUrl from './generated/cirra_device_bg.wasm?url';
import {BrowserStorage} from './storage';
const originalFetch=globalThis.fetch.bind(globalThis);
const confinedFetch:typeof fetch=(input,options)=>originalFetch(input,{...options,redirect:'error',credentials:'omit'});
globalThis.fetch=confinedFetch;
const scope=self as unknown as DedicatedWorkerGlobalScope;
let device:BrowserDevice;
let sequence=Promise.resolve();
let storage:BrowserStorage;
Object.assign(globalThis,{cirraPersist:(snapshot:string)=>storage.persist(snapshot)});
async function run(message:{id:number;command:string;args?:Record<string,unknown>}){
  try{
    let result:unknown;
    if(message.command==='client_request'){
      const r=message.args?.request as {method:string;path:string;body:unknown};
      result=JSON.parse(await device.request(r.method,r.path,JSON.stringify(r.body ?? {})));
    }else if(message.command==='sign_in'){
      await device.sign_in(String(message.args?.access));result=true;
    }else throw new Error('Unsupported browser command.');
    scope.postMessage({id:message.id,result});
  }catch(error){scope.postMessage({id:message.id,error:String(error instanceof Error?error.message:error)});}
}
void navigator.locks.request('cirra-browser-device-v1',{mode:'exclusive',signal:AbortSignal.timeout(5000)},async()=>{
  await init({module_or_path:wasmUrl});
  const saved=await BrowserStorage.open();storage=saved.storage;
  try{device=new BrowserDevice(saved.key,saved.snapshot);}finally{saved.key.fill(0);saved.snapshot='';}
  scope.onmessage=event=>{sequence=sequence.then(()=>run(event.data));};
  scope.postMessage({ready:true});
  await new Promise(()=>{});
}).catch(()=>scope.postMessage({fatal:'Could not open browser device storage. Close other Cirra tabs, check storage permissions, and reload. Existing data was preserved.'}));
