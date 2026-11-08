import {useEffect,useState} from 'react';
import {Check,Laptop,Plus,ShieldCheck,X} from 'lucide-react';
import {authRequest} from './Login';
import type {Account,LoginSession} from './Login';
import SecurityValue from './SecurityValue';
export default function Devices({session,onChange}:{session:LoginSession;onChange:()=>Promise<void>}){
 const [account,setAccount]=useState<Account|undefined>(session.account),[label,setLabel]=useState('My replacement device');
 const [error,setError]=useState(''),[busy,setBusy]=useState(false),[fingerprints,setFingerprints]=useState<Record<string,string>>({});
 const [lookup,setLookup]=useState(''),[target,setTarget]=useState(''),[id,setId]=useState(()=>crypto.randomUUID()),[status,setStatus]=useState('');
 async function refresh(){const accountId=target||(session.desktop?session.account?.id:undefined);setAccount(accountId?await authRequest<Account>(`/devices/inspect?account=${encodeURIComponent(accountId)}`):(await authRequest<LoginSession>('/me')).account);}
 useEffect(()=>{void refresh().catch(error=>setError(error.message));const timer=setInterval(()=>void refresh().catch(error=>setError(error.message)),2000);return()=>clearInterval(timer);},[target]);
 async function run(action:()=>Promise<void>){setError('');setStatus('');setBusy(true);try{await action();await refresh();}catch(error){setError((error as Error).message);}finally{setBusy(false);}}
 const current=session.local_devices?.find(device=>device.alias===session.profile)?.device_id;
 return <div className="devices-panel security-stack"><header className="security-section-heading"><h3>{target?`${account?.name || 'Account'} devices`:'Your devices'}</h3><p>Review approved devices and verify new enrollment requests.</p></header>
 {session.auto_login?<p className="security-empty">Sign in with the normal local login to test device enrollment. Unrestricted test mode bypasses account sign-in.</p>:<>
 {account?.id&&<SecurityValue label="Account enrollment ID" value={account.id}/>}
 <section className="security-card"><div className="security-card-heading"><h4>Approved devices</h4><span className="security-count">{account?.devices.length || 0}</span></div>
 {!account?.devices.length&&<p className="security-empty">No approved devices.</p>}
 <div className="security-device-list">{account?.devices.map(device=><div className="device-approved" key={device.binding.device_id}><span className="security-device-icon"><Laptop size={18}/></span><span><strong>{device.label}</strong><small>{device.revoked?'Relay access revoked':device.binding.device_id===current?'Current device':'Approved identity'}</small></span><ShieldCheck size={17}/>
 {!target&&!device.revoked&&device.binding.device_id!==current&&<button className="secondary-button" disabled={busy} onClick={()=>void run(async()=>{await authRequest('/devices/revoke',{device_id:device.binding.device_id});setStatus('Device relay access revoked. Existing groups still need an MLS removal and key rotation.');})}>Revoke access</button>}</div>)}</div></section>
 <section className="security-request-section"><div className="security-card-heading"><div><h4>Enrollment requests</h4><p>Compare the full fingerprint with the new device using a trusted channel.</p></div></div>
 {!account?.requests.length&&<p className="security-empty">No pending requests.</p>}
 {account?.requests.map(request=>{const pending=request.status==='pending'&&request.expires_at*1000>Date.now();return <section className="device-enrollment security-card" key={request.id}>
 <div className="device-enrollment-title"><strong>{request.label}</strong><span className={`security-badge ${pending?'is-pending':''}`}>{request.status==='pending'&&!pending?'expired':request.status}</span></div>
 <SecurityValue label="Device fingerprint" value={request.fingerprint.match(/.{1,8}/g)?.join(' ') || request.fingerprint}/>
 {pending&&<><label className="field-label">Enter the fingerprint shown on the new device<input autoComplete="off" spellCheck={false} value={fingerprints[request.id] || ''} onChange={event=>setFingerprints({...fingerprints,[request.id]:event.target.value})} placeholder="Full fingerprint"/></label>
 <div className="device-actions"><button className="primary-button" disabled={busy||(fingerprints[request.id] || '').replace(/\s/g,'').toLowerCase()!==request.fingerprint} onClick={()=>void run(async()=>{await authRequest('/devices/decision',{account_id:account.id,id:request.id,fingerprint:request.fingerprint,approve:true});setStatus('Identity approved. Conversation membership still requires an invitation.');})}><Check size={14}/>Approve device</button><button className="secondary-button" disabled={busy} onClick={()=>void run(async()=>{await authRequest('/devices/decision',{account_id:account.id,id:request.id,fingerprint:request.fingerprint,approve:false});setStatus('Device request rejected.');})}><X size={14}/>Reject</button></div></>}
 {request.status==='approved'&&!target&&!session.desktop&&<button className="primary-button" disabled={busy} onClick={()=>void run(async()=>{await authRequest('/devices/activate',{id:request.id});history.replaceState(null,'','/');await onChange();})}>Use this new device</button>}
 </section>;})}</section>
 {session.account?.role==='admin'&&<details className="security-card security-disclosure"><summary>Review another account</summary><form className="device-admin-lookup security-form" onSubmit={event=>{event.preventDefault();setTarget(lookup.trim());}}><label className="field-label">Review another account’s enrollment ID<input value={lookup} onChange={event=>setLookup(event.target.value)} minLength={64} maxLength={64} autoComplete="off" spellCheck={false}/></label><div className="device-actions"><button className="secondary-button" disabled={busy||lookup.trim().length!==64}>Review account</button>{target&&<button type="button" className="secondary-button" onClick={()=>setTarget('')}>My account</button>}</div></form></details>}
 {!target&&!session.desktop&&<form className="device-request security-card security-form" onSubmit={event=>{event.preventDefault();void run(async()=>{await authRequest('/devices/request',{id,label});setId(crypto.randomUUID());setStatus('Device request saved. Compare its fingerprint using a trusted channel, then approve it from an existing device.');});}}><h4>Request a replacement device</h4><label className="field-label">New device name<input value={label} maxLength={80} required onChange={event=>setLabel(event.target.value)}/></label><button className="primary-button" disabled={busy||!label.trim()}><Plus size={15}/>Request replacement device</button></form>}
 {!session.desktop&&<p className="backup-hint">Local test: this Mac hosts both devices. A history backup does not approve a device or rejoin groups.</p>}
 </>}
 {busy&&<p role="status" className="backup-hint">Updating device enrollment…</p>}{status&&<p role="status" className="backup-status">{status}</p>}{error&&<p role="alert" className="form-error">{error}</p>}
 </div>;
}
