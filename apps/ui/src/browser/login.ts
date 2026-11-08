const flowKey='cirra-browser-pkce-v1';
const relay=()=>String(import.meta.env.VITE_CIRRA_RELAY);
type Config={issuer:string;client_id:string;domain:string;scope:string};
type Flow={state:string;verifier:string;created:number;config:Config;callback:string};
const encode=(bytes:Uint8Array)=>btoa(String.fromCharCode(...bytes)).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');
async function config():Promise<Config>{
  const r=await fetch(relay()+'/v1/login/config',{cache:'no-store',credentials:'omit',redirect:'error',signal:AbortSignal.timeout(15000)});
  if(!r.ok)throw new Error('Could not load business sign-in.');
  const c=await r.json() as Config;
  const domain=new URL(c.domain),issuer=new URL(c.issuer);
  if(domain.protocol!=='https:'||domain.href!==domain.origin+'/'||issuer.protocol!=='https:'||!c.client_id||!c.scope.split(' ').includes('croaker/access'))throw new Error('Invalid business sign-in configuration.');
  return c;
}
export async function startLogin():Promise<void>{
  const c=await config();
  const state=encode(crypto.getRandomValues(new Uint8Array(32)));
  const verifier=encode(crypto.getRandomValues(new Uint8Array(48)));
  const callback=location.origin+'/auth/callback';
  const flow:Flow={state,verifier,created:Date.now(),config:c,callback};
  const challenge=encode(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(verifier))));
  sessionStorage.setItem(flowKey,JSON.stringify(flow));
  const url=new URL('/oauth2/authorize',c.domain);
  url.search=new URLSearchParams({response_type:'code',client_id:c.client_id,redirect_uri:callback,scope:c.scope,state,code_challenge:challenge,code_challenge_method:'S256'}).toString();
  location.assign(url.href);
}
export async function completeLogin():Promise<string|null>{
  if(location.pathname!=='/auth/callback')return null;
  const query=new URLSearchParams(location.search);
  const saved=sessionStorage.getItem(flowKey);sessionStorage.removeItem(flowKey);
  history.replaceState(null,'','/');
  if(!saved)throw new Error('Sign-in session missing. Start sign-in again.');
  const flow=JSON.parse(saved) as Flow;
  if(query.getAll('state').length!==1||query.get('state')!==flow.state||Date.now()-flow.created>300000||Date.now()<flow.created||flow.callback!==location.origin+'/auth/callback'||query.has('error')||query.getAll('code').length!==1||!query.get('code')||query.get('code')!.length>4096)throw new Error('Sign-in expired or could not be verified. Start sign-in again.');
  const current=await config();
  if(JSON.stringify(current)!==JSON.stringify(flow.config))throw new Error('Sign-in provider changed. Start sign-in again.');
  const r=await fetch(new URL('/oauth2/token',current.domain),{method:'POST',credentials:'omit',redirect:'error',signal:AbortSignal.timeout(15000),headers:{'content-type':'application/x-www-form-urlencoded'},body:new URLSearchParams({grant_type:'authorization_code',client_id:current.client_id,redirect_uri:flow.callback,code:query.get('code')!,code_verifier:flow.verifier})});
  if(!r.ok)throw new Error('Business sign-in was rejected. Start sign-in again.');
  const tokens=await r.json() as {access_token:string;token_type:string;expires_in:number};
  if(!tokens.access_token||tokens.access_token.length>32768||tokens.token_type.toLowerCase()!=='bearer'||tokens.expires_in<=0||tokens.expires_in>86400)throw new Error('Invalid sign-in response.');
  return tokens.access_token;
}
