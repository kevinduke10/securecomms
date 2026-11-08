import {useEffect,useRef,useState} from 'react';
import type {KeyboardEvent} from 'react';
import {Laptop,Users,Mail,GitBranch} from 'lucide-react';
import Devices from './Devices';
import {NativeContacts} from './DesktopSetup';
import {isNative} from './transport';
import type {LoginSession} from './Login';
import './device-security.css';
const tabs=[{id:'devices',label:'Devices',icon:Laptop},{id:'contacts',label:'Contacts',icon:Users},{id:'invitations',label:'Invitations',icon:Mail},{id:'changes',label:'Group changes',icon:GitBranch}] as const;
type Tab=typeof tabs[number]['id'];
export default function DeviceSecurity({session,onChange}:{session:LoginSession;onChange:()=>Promise<void>}){
 const [active,setActive]=useState<Tab>('devices');const native=isNative();const body=useRef<HTMLDivElement>(null);
 useEffect(()=>{body.current?.scrollTo({top:0});},[active]);
 function navigate(event:KeyboardEvent,index:number){
  const next=event.key==='ArrowRight'?(index+1)%tabs.length:event.key==='ArrowLeft'?(index+tabs.length-1)%tabs.length:event.key==='Home'?0:event.key==='End'?tabs.length-1:null;
  if(next===null)return;event.preventDefault();setActive(tabs[next].id);document.getElementById(`security-tab-${tabs[next].id}`)?.focus();
 }
 return <div className="security-content">
 {native&&<div className="security-tabs" role="tablist" aria-label="Security settings">{tabs.map(({id,label,icon:Icon},index)=><button key={id} id={`security-tab-${id}`} role="tab" aria-selected={active===id} aria-controls={`security-panel-${id}`} tabIndex={active===id?0:-1} onClick={()=>setActive(id)} onKeyDown={event=>navigate(event,index)}><Icon size={16}/>{label}</button>)}</div>}
 <div className="security-body" ref={body}>
 <section className="security-panel" id="security-panel-devices" role={native?'tabpanel':undefined} aria-labelledby={native?'security-tab-devices':undefined} hidden={active!=='devices'}><Devices session={session} onChange={onChange}/></section>
 {native&&(['contacts','invitations','changes'] as const).map(section=><section key={section} className="security-panel" id={`security-panel-${section}`} role="tabpanel" aria-labelledby={`security-tab-${section}`} hidden={active!==section}><NativeContacts section={section}/></section>)}
 </div></div>;
}
