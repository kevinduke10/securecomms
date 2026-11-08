import {useEffect,useState} from 'react';
import type {LoginSession} from './Login';
import {clientFetch,nativeCommand} from './transport';
import SecurityValue from './SecurityValue';
import './device-security.css';
export async function desktopRequest<T>(path:string,body?:unknown):Promise<T>{const response=await clientFetch(path,{method:body===undefined?'GET':'POST',body:body===undefined?undefined:JSON.stringify(body)});const result=await response.json();if(!response.ok)throw new Error(result.error || 'Device operation failed.');return result;}
type Status={configured:boolean;relay?:string;enrolled:boolean;approved:boolean;fingerprint?:string;account_id?:string};
export default function DesktopSetup({session,onRefresh,error:outerError}:{session:LoginSession|null;onRefresh:()=>Promise<void>;error:string}){
 const[status,setStatus]=useState<Status|null>(null),[relay,setRelay]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false);
 const refresh=async()=>{const result=await desktopRequest<Status>('/desktop/status');setStatus(result);};
 useEffect(()=>{void refresh().catch(e=>setError(e.message));const timer=setInterval(()=>{void refresh().catch(()=>{});void onRefresh();},1500);return()=>clearInterval(timer);},[onRefresh]);
 async function run(action:()=>Promise<void>){setBusy(true);setError('');try{await action();await refresh();await onRefresh();}catch(e){setError((e as Error).message);}finally{setBusy(false);}}
 return <main className="login-page"><section className="login-card"><img src="/cirra.svg" alt="Cirra"/><h1>Sign in to Cirra</h1>
 {!status?<p role="status">Opening device store…</p>:!status.configured?<form onSubmit={e=>{e.preventDefault();void run(async()=>{await desktopRequest('/desktop/configure',{relay});});}}><label className="field-label">Business relay URL<input type="url" required placeholder="https://relay.example.com" value={relay} onChange={e=>setRelay(e.target.value)}/></label><button className="primary-button" disabled={busy}>Save relay</button></form>:<><p className="backup-hint setup-relay">Relay: {status.relay}</p><div className="setup-actions"><button className="primary-button" disabled={busy} onClick={()=>void run(async()=>{await nativeCommand('open_login');})}>Sign in with business account</button>
 {session?.signed_in&&!status.approved&&<button className="secondary-button" disabled={busy} onClick={()=>void run(async()=>{await desktopRequest('/desktop/enrollment',{});})}>{status.enrolled?'Resend enrollment request':'Enroll this device'}</button>}</div>
 {status.fingerprint&&<><p>Compare this fingerprint with an approved device or your administrator.</p><SecurityValue label="Device fingerprint" value={status.fingerprint.match(/.{1,8}/g)?.join(' ') || status.fingerprint}/><SecurityValue label="Account enrollment ID" value={status.account_id || ''}/><p role="status">{status.approved?'Device approved. Sign in to unlock.':'Waiting for approval. This device keeps its own private keys.'}</p></>}
 </>}{(error||outerError)&&<p className="form-error" role="alert">{error||outerError}</p>}</section></main>;
}
export function NativeContacts({section}:{section:'contacts'|'invitations'|'changes'}){
 const [packet,setPacket]=useState(''),[fingerprint,setFingerprint]=useState(''),[output,setOutput]=useState(''),[error,setError]=useState(''),[status,setStatus]=useState(''),[busy,setBusy]=useState(false);
 const settings={
  contacts:{title:'Verified contacts',description:'Exchange contact packages and compare the full fingerprint using a trusted channel.',exportLabel:'Show my contact package',exportPath:'/desktop/contact',outputLabel:'Public contact package',importTitle:'Add a contact',packetLabel:'Contact package',fingerprintLabel:'Verified device fingerprint',action:'Import verified contact',path:'/desktop/contacts',key:'contact'},
  invitations:{title:'Conversation invitations',description:'Import and verify every participant’s contact before accepting an invitation.',exportLabel:'Show conversation invitations',exportPath:'/desktop/invitations',outputLabel:'Conversation invitations',importTitle:'Accept an invitation',packetLabel:'Conversation invitation',fingerprintLabel:'Verified owner fingerprint',action:'Import invitation',path:'/desktop/invitations',key:'invitation'},
  changes:{title:'Group changes',description:'Import owner-approved changes in order. Verify all participant contacts and sync between changes.',exportLabel:'Show group decisions',exportPath:'/desktop/group-decisions',outputLabel:'Group decisions',importTitle:'Apply a group decision',packetLabel:'Group decision',fingerprintLabel:'Verified approving owner fingerprint',action:'Import group decision',path:'/desktop/group-decisions',key:'decision'},
 }[section];
 async function run(action:()=>Promise<void>){setError('');setStatus('');setBusy(true);try{await action();}catch(error){setError((error as Error).message);}finally{setBusy(false);}}
 return <div className="security-stack"><header className="security-section-heading"><h3>{settings.title}</h3><p>{settings.description}</p></header>
 <section className="security-card"><div className="security-card-heading"><div><h4>Share your package</h4><p>Packages contain no private keys. They can reveal participant and group information.</p></div></div>
 <button className="secondary-button" disabled={busy} onClick={()=>void run(async()=>{setOutput(JSON.stringify(await desktopRequest(settings.exportPath),null,2));})}>{settings.exportLabel}</button>
 {output&&<SecurityValue label={settings.outputLabel} value={output} packageValue/>}</section>
 <form className="security-card security-form" onSubmit={event=>{event.preventDefault();void run(async()=>{await desktopRequest(settings.path,{[settings.key]:JSON.parse(packet),fingerprint});setPacket('');setFingerprint('');setStatus('Package imported.');});}}>
 <div className="security-card-heading"><div><h4>{settings.importTitle}</h4><p>{section==='contacts'?'Paste the other device’s package and its independently verified fingerprint.':'Paste one object from the exported list and the owner’s independently verified fingerprint.'}</p></div></div>
 <label className="field-label">{settings.packetLabel}<textarea aria-label={settings.packetLabel} rows={5} required spellCheck={false} autoComplete="off" value={packet} onChange={event=>setPacket(event.target.value)} placeholder="Paste package JSON"/></label>
 <label className="field-label">{settings.fingerprintLabel}<input required value={fingerprint} onChange={event=>setFingerprint(event.target.value)} autoComplete="off" spellCheck={false} placeholder="Full fingerprint"/></label>
 <div className="security-form-footer"><button className="primary-button" disabled={busy||!packet.trim()||!fingerprint.trim()}>{busy?'Working…':settings.action}</button></div></form>
 {status&&<p role="status" className="backup-status">{status}</p>}{error&&<p className="form-error" role="alert">{error}</p>}
 </div>;
}
