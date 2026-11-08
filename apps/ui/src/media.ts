import {clientFetch} from './transport';
export const OBJECT_ATTACHMENT_LIMIT = 25 * 1024 * 1024;
export type MediaSummary = { object_id: string; name: string; mime: string; size: number };
export async function sendObjectAttachment(profile:string,conversation:string,requestId:string,file:{name:string;mime:string;data:string;size:number},text:string,replyTo?:{message_id:string;sender:string}) {
  if (file.size > OBJECT_ATTACHMENT_LIMIT) throw new Error('Choose a file up to 25 MiB.');
  const response = await clientFetch(`/local-api/${encodeURIComponent(profile)}/media/send`, {method:'POST',headers:{'content-type':'application/json','x-croaker-local':'1'},body:JSON.stringify({conversation_id:conversation,request_id:requestId,name:file.name,mime:file.mime,data:file.data,text,reply_to:replyTo || null})});
  if (!response.ok) {const error=await response.json().catch(()=>({}));throw new Error(error.error || 'Attachment could not be sent.');}
}
export async function fetchObjectAttachment(profile:string,conversation:string,message:string,sender:string):Promise<Blob> {
 const response=await clientFetch(`/local-api/${encodeURIComponent(profile)}/media/${encodeURIComponent(conversation)}/${encodeURIComponent(message)}?sender=${encodeURIComponent(sender)}`,{headers:{'x-croaker-local':'1'}});
 if(!response.ok) {const error=await response.json().catch(()=>({}));throw new Error(error.error || 'Attachment is unavailable.');} return response.blob();
}
export const previewableMedia=(mime:string)=>['image/png','image/jpeg','image/webp','image/gif'].includes(mime);
