import {useEffect,useRef,useState} from 'react';
import {Check,Copy} from 'lucide-react';
// Only public identifiers and explicitly exported packages belong in this control.
export default function SecurityValue({label,value,packageValue=false}:{label:string;value:string;packageValue?:boolean}){
 const ref=useRef<HTMLElement|null>(null);const [status,setStatus]=useState('');
 useEffect(()=>setStatus(''),[value]);
 async function copy(){
  try{await navigator.clipboard.writeText(value);setStatus('Copied');}
  catch{
   const element=ref.current;
   if(element instanceof HTMLTextAreaElement){element.focus();element.select();}
   else if(element){const range=document.createRange();range.selectNodeContents(element);const selection=window.getSelection();selection?.removeAllRanges();selection?.addRange(range);}
   setStatus('Selected. Use your device’s Copy command.');
  }
 }
 return <div className="security-value"><div className="security-value-head"><span>{label}</span><button type="button" className="security-copy" aria-label={`Copy ${label.toLowerCase()}`} onClick={()=>void copy()}>{status==='Copied'?<Check size={14}/>:<Copy size={14}/>}Copy</button></div>
 {packageValue?<textarea ref={node=>{ref.current=node;}} aria-label={label} rows={7} readOnly spellCheck={false} value={value}/>:<code className={label==='Device fingerprint'?'device-fingerprint':undefined} ref={node=>{ref.current=node;}}>{value}</code>}
 {status&&<span className="security-copy-status" role="status">{status}</span>}
 </div>;
}
