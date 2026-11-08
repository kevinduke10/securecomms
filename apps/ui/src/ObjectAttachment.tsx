import {saveBlob} from './transport';
import {useEffect,useState} from 'react';
import {Download,File,Image as ImageIcon} from 'lucide-react';
import {fetchObjectAttachment,previewableMedia,type MediaSummary} from './media';
import './object-attachments.css';
type Props={file:MediaSummary;load?:()=>Promise<Blob>;profile:string;conversation:string;message:string;sender:string};
export default function ObjectAttachment({file,load,profile,conversation,message,sender}:Props){
 const [url,setUrl]=useState<string>();const [busy,setBusy]=useState(false);const [error,setError]=useState('');
 useEffect(()=>()=>{if(url)URL.revokeObjectURL(url)},[url]);
 async function get(preview:boolean){setBusy(true);setError('');try{const blob=await (load?.() || fetchObjectAttachment(profile,conversation,message,sender));if(blob.size!==file.size)throw new Error('Attachment size mismatch.');
 if(preview){setUrl(URL.createObjectURL(new Blob([blob],{type:file.mime})))}else{await saveBlob(new Blob([blob],{type:'application/octet-stream'}),file.name)}}catch(e){setError(e instanceof Error?e.message:'Attachment is unavailable.')}finally{setBusy(false)}}
 return <div className="object-attachment">{url&&<img className="object-preview" src={url} alt={file.name}/>}<div className="object-file-row"><File size={20}/><span><strong>{file.name}</strong><small>{file.size>=1048576?`${(file.size/1048576).toFixed(1)} MiB`:`${Math.ceil(file.size/1024)} KiB`}</small></span><button onPointerDown={e=>e.stopPropagation()} aria-label={`Download ${file.name}`} disabled={busy} onClick={()=>void get(false)}><Download size={18}/></button>{previewableMedia(file.mime)&&!url&&<button onPointerDown={e=>e.stopPropagation()} aria-label={`Preview ${file.name}`} disabled={busy} onClick={()=>void get(true)}><ImageIcon size={18}/></button>}</div>{busy&&<small role="status">Loading attachment…</small>}{error&&<small role="alert">{error}</small>}</div>
}
