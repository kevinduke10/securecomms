import {test,expect,type Page} from '@playwright/test';
import {randomUUID,createHash} from 'node:crypto';
type Result={status:number;body:any};
async function rpc(page:Page,path:string,body?:unknown):Promise<any>{
 const r=await page.evaluate(async({path,body})=>{
  return (window as any).__CIRRA_BROWSER__.invoke('client_request',{request:{method:body===undefined?'GET':'POST',path,body}});
 },{path,body}) as Result;
 expect(r.status,JSON.stringify(r.body)).toBe(200);return r.body;
}
async function enroll(page:Page,name:'alice'|'bob'){
 await page.goto('/');await expect(page.getByRole('button',{name:'Sign in with business account'})).toBeVisible();
 await page.evaluate(name=>(window as any).__CIRRA_BROWSER__.invoke('sign_in',{access:'fixture-'+name}),name);
 const request=await rpc(page,'/desktop/enrollment',{});
 const r=await page.request.post('/test/approve',{data:{account:request.account_id,fingerprint:request.fingerprint}});expect(r.status()).toBe(204);
 const session=await rpc(page,'/local-auth/me');expect(session.authenticated).toBe(true);
 return {id:session.profile as string, fingerprint:request.fingerprint as string, account:request.account_id as string};
}
test('real WASM clients enroll, pin contacts, exchange MLS, survive ambiguous sends/reload and preserve sign-out',async({browser})=>{
 const a=await browser.newContext(),b=await browser.newContext();const alice=await a.newPage(),bob=await b.newPage();
 const aa=await enroll(alice,'alice'),bb=await enroll(bob,'bob');
 const ac=await rpc(alice,'/desktop/contact'),bc=await rpc(bob,'/desktop/contact');
 await rpc(alice,'/desktop/contacts',{contact:bc,fingerprint:bb.fingerprint});await rpc(bob,'/desktop/contacts',{contact:ac,fingerprint:aa.fingerprint});
 const chat=await rpc(alice,`/local-api/${aa.id}/conversations`,{title:'Browser group',kind:'group',members:[bb.id]});
 const invitations=await rpc(alice,'/desktop/invitations');await rpc(bob,'/desktop/invitations',{invitation:invitations[0],fingerprint:aa.fingerprint});
 await rpc(bob,`/local-api/state?profile=${bb.id}`);
 let dropped=false;await alice.route('**/v1/conversations/*/events',async route=>{
  if(route.request().method()==='POST'&&!dropped){dropped=true;await route.fetch();await route.abort('connectionreset');}else await route.continue();
 });
 const body={conversation_id:chat.id,request_id:randomUUID(),content:{version:1,text:'Private browser message',attachment:null}};
 await rpc(alice,`/local-api/${aa.id}/send`,body);expect(dropped).toBe(true);
 await alice.reload();await expect(alice.getByRole('button',{name:'New conversation',exact:true})).toBeVisible();
 await rpc(alice,`/local-api/state?profile=${aa.id}`);
 const received=await rpc(bob,`/local-api/state?profile=${bb.id}`);
 const messages=received.conversations.find((c:any)=>c.id===chat.id).messages;
 expect(messages.filter((m:any)=>m.text==='Private browser message')).toHaveLength(1);
 const fileRequest=randomUUID();const fileData=Buffer.alloc(40*1024,37).toString('base64');
 await rpc(alice,`/local-api/${aa.id}/media/send`,{conversation_id:chat.id,request_id:fileRequest,name:'browser.bin',mime:'application/octet-stream',data:fileData,text:'Encrypted browser attachment'});
 await rpc(bob,`/local-api/state?profile=${bb.id}`);
 const download=await rpc(bob,`/local-api/${bb.id}/media/${chat.id}/${fileRequest}?sender=${aa.id}`);
 expect(download.binary_base64).toBe(fileData);
 let leaked=0;const observe=(request:any)=>{if(request.method()==='POST'&&request.url().endsWith('/events'))leaked++;};
 alice.on('request',observe);
 const engine=alice.workers().find(w=>w.url().includes('worker.ts'))!;expect(engine).toBeTruthy();
 await engine.evaluate(()=>{Object.assign(globalThis,{cirraPersist:async()=>{throw new Error('Forced IndexedDB failure');}});});
 await expect(rpc(alice,`/local-api/${aa.id}/send`,{conversation_id:chat.id,request_id:randomUUID(),content:{version:1,text:'Must not leave this browser',attachment:null}})).rejects.toThrow();
 expect(leaked).toBe(0);alice.off('request',observe);
 await alice.reload();await expect(alice.getByRole('button',{name:'New conversation',exact:true})).toBeVisible();
 const afterFailure=await rpc(bob,`/local-api/state?profile=${bb.id}`);
 expect(afterFailure.conversations.find((c:any)=>c.id===chat.id).messages.some((m:any)=>m.text==='Must not leave this browser')).toBe(false);
 let accountReads=0;const review=(request:any)=>{if(request.url().endsWith('/v1/enrollments/'+aa.account))accountReads++;};
 alice.on('request',review);await alice.getByRole('button',{name:'Devices',exact:true}).click();
 await expect(alice.getByRole('dialog',{name:'Devices & security'}).getByText('Current device',{exact:true})).toBeVisible();
 await expect.poll(()=>accountReads).toBeGreaterThan(0);alice.off('request',review);
 await alice.getByRole('dialog',{name:'Devices & security'}).getByRole('button',{name:'Close dialog'}).click();
 const raw=await alice.evaluate(async()=>{
  const db=await new Promise<IDBDatabase>((resolve,reject)=>{const r=indexedDB.open('cirra-browser-device-v1');r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
  return await new Promise<any>(resolve=>{const tx=db.transaction('device');const store=tx.objectStore('device');const key=store.get('identity'),snapshot=store.get('snapshot');tx.oncomplete=()=>{resolve({extractable:key.result.key.extractable,snapshotKeys:Object.keys(snapshot.result),text:new TextDecoder().decode(snapshot.result.ciphertext)});db.close();};});
 });
 expect(raw.extractable).toBe(false);expect(raw.snapshotKeys.sort()).toEqual(['ciphertext','iv']);expect(raw.text).not.toContain('Private browser message');
 const duplicate=await a.newPage();await duplicate.goto('/');await expect(duplicate.getByRole('alert')).toContainText('Close other Cirra tabs',{timeout:12000});await duplicate.close();
 await rpc(alice,'/local-auth/logout',{});await alice.reload();await expect(alice.getByRole('button',{name:'Sign in with business account'})).toBeVisible();
 const locked=await rpc(alice,'/local-auth/me');expect(locked.authenticated).toBe(false);
 await a.close();await b.close();
});
test('PKCE callback rejects an unbound login session',async({page})=>{
 await page.goto('/auth/callback?code=untrusted&state=untrusted');
 await expect(page.getByRole('alert')).toContainText('Sign-in session missing');expect(new URL(page.url()).search).toBe('');
});

test('browser PKCE binds the code, callback and state and consumes the verifier',async({page})=>{
 await page.route('https://login.fixture.example/oauth2/authorize?*',route=>route.fulfill({contentType:'text/html',body:'<p>Fixture identity provider</p>'}));
 let tokenRequest:URLSearchParams|undefined;
 await page.route('https://login.fixture.example/oauth2/token',route=>{
  tokenRequest=new URLSearchParams(route.request().postData()!);
  return route.fulfill({contentType:'application/json',headers:{'access-control-allow-origin':'http://127.0.0.1:3111'},body:JSON.stringify({access_token:'fixture-alice',token_type:'Bearer',expires_in:300})});
 });
 await page.goto('/');await page.getByRole('button',{name:'Sign in with business account'}).click();
 await page.waitForURL('https://login.fixture.example/oauth2/authorize?*');
 const authorize=new URL(page.url());
 expect(authorize.searchParams.get('code_challenge_method')).toBe('S256');
 expect(authorize.searchParams.get('redirect_uri')).toBe('http://127.0.0.1:3111/auth/callback');
 await page.goto('/auth/callback?'+new URLSearchParams({code:'fixture-code',state:authorize.searchParams.get('state')!}));
 await expect(page.getByRole('button',{name:'Enroll this device',exact:true})).toBeVisible();
 expect(tokenRequest?.get('code')).toBe('fixture-code');
 expect(createHash('sha256').update(tokenRequest!.get('code_verifier')!).digest('base64url')).toBe(authorize.searchParams.get('code_challenge'));
 expect(await page.evaluate(()=>sessionStorage.getItem('cirra-browser-pkce-v1'))).toBeNull();
 expect(new URL(page.url()).search).toBe('');
});
