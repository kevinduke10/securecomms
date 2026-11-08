// IndexedDB holds a non-extractable wrapping key and encrypted device snapshots.
// A worker holds the origin-wide Web Lock for the entire installation lifetime.
const encoder = new TextEncoder();
const context = encoder.encode('cirra-browser-snapshot-v1');
const vaultContext = encoder.encode('cirra-browser-vault-key-v1');
type Sealed = { iv: Uint8Array; ciphertext: ArrayBuffer };
type Identity = { key: CryptoKey; vault: Sealed };
export class BrowserStorage {
  private constructor(private db: IDBDatabase, private identity: Identity) {}
  static async open(): Promise<{storage:BrowserStorage;key:Uint8Array;snapshot:string}> {
    const db = await new Promise<IDBDatabase>((resolve,reject)=>{
      const request=indexedDB.open('cirra-browser-device-v1',1);
      request.onupgradeneeded=()=>request.result.createObjectStore('device');
      request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
      request.onblocked=()=>reject(new Error('Close other Cirra tabs before opening this device.'));
    });
    const read=(key:string)=>new Promise<unknown>((resolve,reject)=>{
      const r=db.transaction('device','readonly').objectStore('device').get(key);
      r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);
    });
    let identity=await read('identity') as Identity|undefined;
    const sealed=await read('snapshot') as Sealed|undefined;
    if(!identity && sealed) throw new Error('Browser vault key missing. Existing data was preserved.');
    if(!identity){
      const key=await crypto.subtle.generateKey({name:'AES-GCM',length:256},false,['encrypt','decrypt']);
      const vault=crypto.getRandomValues(new Uint8Array(32));
      identity={key,vault:await encrypt(key,vault,vaultContext)};
      vault.fill(0);
      await new Promise<void>((resolve,reject)=>{
        const tx=db.transaction('device','readwrite',{durability:'strict'});
        tx.objectStore('device').add(identity,'identity');
        tx.oncomplete=()=>resolve();tx.onabort=()=>reject(tx.error);tx.onerror=()=>reject(tx.error);
      });
    }
    const key=new Uint8Array(await decrypt(identity.key,identity.vault,vaultContext));
    const snapshot=sealed?new TextDecoder('utf-8',{fatal:true}).decode(await decrypt(identity.key,sealed,context)):'';
    return {storage:new BrowserStorage(db,identity),key,snapshot};
  }
  async persist(snapshot:string):Promise<void>{
    const bytes=encoder.encode(snapshot);
    if(bytes.length>128*1024*1024)throw new Error('Browser device storage exceeds 128 MiB. Existing saved state was preserved.');
    let sealed:Sealed;
    try{sealed=await encrypt(this.identity.key,bytes,context);}finally{bytes.fill(0);}
    await new Promise<void>((resolve,reject)=>{
      const tx=this.db.transaction('device','readwrite',{durability:'strict'});
      tx.objectStore('device').put(sealed,'snapshot');
      tx.oncomplete=()=>resolve();tx.onabort=()=>reject(tx.error);tx.onerror=()=>reject(tx.error);
    });
  }
}
async function encrypt(key:CryptoKey,bytes:Uint8Array,additionalData:Uint8Array):Promise<Sealed>{
  const iv=crypto.getRandomValues(new Uint8Array(12));
  return {iv,ciphertext:await crypto.subtle.encrypt({name:'AES-GCM',iv,additionalData:additionalData as BufferSource},key,bytes as BufferSource)};
}
async function decrypt(key:CryptoKey,sealed:Sealed,additionalData:Uint8Array):Promise<ArrayBuffer>{
  return crypto.subtle.decrypt({name:'AES-GCM',iv:sealed.iv as BufferSource,additionalData:additionalData as BufferSource},key,sealed.ciphertext);
}
