import { useEffect, useState } from 'react';
import { ArrowLeft, Download, FileKey2, ShieldCheck, Trash2, Upload } from 'lucide-react';

type Summary = { id: string; created_at: number; owner: string; conversations: number; messages: number };
type Attachment = { name: string; mime: string; data: string; size: number };
type Archive = { id: string; created_at: number; source_device: string; people: { device_id: string; name: string }[]; conversations: { id: string; title: string; messages: { id: string; sender: string; sent_at: number; text: string; reply_to?: { message_id: string; sender: string } | null; attachment: Attachment | null; reactions: { sender: string; emoji: string }[] }[] }[] };
const maxFile = 16 * 1024 * 1024 + 256;
async function request(profile: string, path: string, body?: unknown) {
  const response = await fetch(`/local-api/${profile}/backups${path}`, { method: body === undefined ? 'GET' : 'POST', cache: 'no-store', headers: { 'x-croaker-local': '1', ...(body === undefined ? {} : { 'content-type': 'application/json' }) }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(60_000) });
  if (response.status === 401) window.dispatchEvent(new Event('croaker-signin-required'));
  if (!response.ok) { const error = await response.json().catch(() => ({})); throw new Error(error.error || 'The backup operation failed. Please try again.'); }
  return response;
}
function save(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob); const link = document.createElement('a');
  link.href = url; link.download = name; link.click(); window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
async function encodeFile(file: File) {
  const bytes = new Uint8Array(await file.arrayBuffer()); let value = '';
  for (let offset = 0; offset < bytes.length; offset += 8192) value += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
  return btoa(value);
}
export default function Backups({ profile }: { profile: string }) {
  const [tab, setTab] = useState<'export' | 'restore' | 'archives'>('export');
  const [code, setCode] = useState(''); const [confirmation, setConfirmation] = useState('');
  const [saved, setSaved] = useState(false); const [file, setFile] = useState<File | null>(null);
  const [restoreCode, setRestoreCode] = useState(''); const [error, setError] = useState('');
  const [status, setStatus] = useState(''); const [busy, setBusy] = useState(false);
  const [archives, setArchives] = useState<Summary[]>([]); const [archive, setArchive] = useState<Archive | null>(null);
  const [search, setSearch] = useState('');
  async function loadArchives() { const result = await (await request(profile, '/archives')).json(); setArchives(result.archives); }
  useEffect(() => { void loadArchives().catch(error => setError(error.message)); }, [profile]);
  async function run(action: () => Promise<void>) { setError(''); setStatus(''); setBusy(true); try { await action(); } catch (error) { setError((error as Error).message); } finally { setBusy(false); } }
  const name = (id: string) => archive?.people.find(person => person.device_id === id)?.name || 'Unknown participant';
  const date = (timestamp: number) => new Date(timestamp * 1000).toLocaleString();
  function quote(chat: Archive['conversations'][number], message: Archive['conversations'][number]['messages'][number]) {
    if (!message.reply_to) return null;
    const index = chat.messages.findIndex(item => item.id === message.id && item.sender === message.sender);
    const target = chat.messages.slice(0,index).find(item => item.id === message.reply_to!.message_id && item.sender === message.reply_to!.sender);
    return <blockquote className="archive-quote"><strong>{target ? name(target.sender) : 'Reply'}</strong><span>{target ? target.text || target.attachment?.name || 'Message unavailable' : 'Message unavailable'}</span></blockquote>;
  }

  return <div className="backup-panel" aria-busy={busy}>
    <p className="modal-intro">Back up saved history with a recovery code only you hold. Keep the file and code separately. Neither can be replaced if lost.</p>
    <div className="segmented" role="group" aria-label="Backup actions">{(['export', 'restore', 'archives'] as const).map(value => <button type="button" key={value} disabled={busy} className={tab === value ? 'selected' : ''} aria-pressed={tab === value} onClick={() => { setTab(value); setArchive(null); setError(''); setStatus(''); }}>{value === 'export' ? 'Create backup' : value === 'restore' ? 'Restore backup' : `Archives (${archives.length})`}</button>)}</div>
    {tab === 'export' && <section aria-label="Create encrypted backup">
      <div className="backup-explanation"><ShieldCheck size={20} /><p>Messages, attachments, and reactions are included. Unsent messages, typing, and live messaging keys are excluded. Maximum archive size: 16 MiB.</p></div>
      {!code ? <button type="button" className="primary-button" disabled={busy} onClick={() => void run(async () => { const result = await (await request(profile, '/code', {})).json(); setCode(result.recovery_code); setSaved(false); setConfirmation(''); })}><FileKey2 size={16} />Generate recovery code</button> : <>
        <label className="field-label">Your recovery code<textarea className="recovery-code" aria-label="Your recovery code" readOnly value={code} spellCheck={false} autoComplete="off" /></label>
        <p className="backup-hint">Save this code in a password manager or on paper. It disappears from this dialog when you close it. It unlocks any backup made with it.</p>
        <label className="backup-check"><input type="checkbox" checked={saved} onChange={e => setSaved(e.target.checked)} />I saved my recovery code somewhere safe.</label>
        <label className="field-label">Enter the saved code again<input type="password" autoComplete="off" spellCheck={false} value={confirmation} onChange={e => setConfirmation(e.target.value)} /></label>
        <button type="button" className="primary-button" disabled={busy || !saved || confirmation.trim().toUpperCase() !== code.toUpperCase()} onClick={() => void run(async () => { const response = await request(profile, '/export', { recovery_code: code, confirmation }); save(await response.blob(), 'cirra-history.crkbackup'); setStatus('Encrypted backup downloaded. Keep your recovery code separately.'); })}><Download size={16} />Download encrypted backup</button>
      </>}
    </section>}
    {tab === 'restore' && <form onSubmit={e => { e.preventDefault(); if (!file || !restoreCode || busy) return; void run(async () => {
      if (file.size > maxFile) throw new Error('Backup file is too large. Maximum archive size is 16 MiB.');
      const result = await (await request(profile, '/restore', { recovery_code: restoreCode, backup: await encodeFile(file) })).json();
      setRestoreCode(''); await loadArchives(); setTab('archives'); setArchive(null);
      setStatus(result.imported ? 'History restored as a read-only archive. Your live conversations are unchanged.' : 'This archive was already restored. No duplicate was created.');
    }); }}>
      <div className="backup-explanation"><Upload size={20} /><p>Restore past history as a separate archive. This does not sign you in, rejoin groups, or grant access to future messages.</p></div>
      <label className="field-label">Encrypted backup file<input type="file" accept=".crkbackup,application/octet-stream" required onChange={e => setFile(e.target.files?.[0] || null)} /></label>
      <label className="field-label">Recovery code<input type="password" autoComplete="off" spellCheck={false} required value={restoreCode} onChange={e => setRestoreCode(e.target.value)} /></label>
      <button className="primary-button" disabled={busy || !file || !restoreCode}><Upload size={16} />Restore history</button>
    </form>}
    {tab === 'archives' && <section aria-label="Recovered archives">
      {archive ? <>
        <button type="button" className="secondary-button" onClick={() => { setArchive(null); setSearch(''); }}><ArrowLeft size={13} /> All archives</button>
        <p className="backup-hint">Read-only history for {name(archive.source_device)} · {date(archive.created_at)}. Participant names come from this backup; they are not verified live identities.</p>
        <label className="field-label">Search recovered history<input type="search" value={search} onChange={e => setSearch(e.target.value)} /></label>
        <div className="archive-history">{archive.conversations.map(chat => {
          const messages = chat.messages.filter(message => `${message.text} ${message.attachment?.name || ''} ${chat.title}`.toLowerCase().includes(search.toLowerCase()));
          return messages.length ? <section key={chat.id}><h3>{chat.title}</h3>{messages.map(message => <article className="archive-message" key={`${message.sender}:${message.id}`}><header><strong>{name(message.sender)}</strong><time>{date(message.sent_at)}</time></header>{quote(chat,message)}<p>{message.text}</p>{message.attachment && <a className="file-card" href={`data:application/octet-stream;base64,${message.attachment.data}`} download={message.attachment.name}><Download size={16} />{message.attachment.name}</a>}{message.reactions.length > 0 && <p className="archive-reactions">{message.reactions.map(reaction => <span key={reaction.sender} title={name(reaction.sender)}>{reaction.emoji}</span>)}</p>}</article>)}</section> : null;
        })}{!archive.conversations.some(chat => chat.messages.some(message => `${message.text} ${message.attachment?.name || ''} ${chat.title}`.toLowerCase().includes(search.toLowerCase()))) && <p className="backup-hint">No matching saved messages.</p>}</div>
      </> : archives.length ? <div className="archive-list">{archives.map(item => <div className="archive-item" key={item.id}><button type="button" disabled={busy} onClick={() => void run(async () => { setArchive(await (await request(profile, `/archives/${item.id}`)).json()); setSearch(''); })}><strong>{item.owner}</strong><span>{date(item.created_at)}</span><small>{item.conversations} conversations · {item.messages} messages</small></button><button type="button" className="icon-button" aria-label={`Remove local archive for ${item.owner}`} disabled={busy} onClick={() => void run(async () => { await request(profile, `/archives/${item.id}/remove`, {}); await loadArchives(); setStatus('Local archive removed. Your exported backup file is unchanged.'); })}><Trash2 size={16} /></button></div>)}</div> : <p className="backup-hint">No restored archives yet. Choose Restore backup to open a saved file.</p>}
    </section>}
    {busy && <p className="backup-hint" role="status">Processing backup…</p>}
    {error && <p className="form-error" role="alert">{error}</p>}
    {status && <p className="backup-status" role="status">{status}</p>}
    <p className="backup-footer">Development workspace: this Mac holds all test-account keys. Backups are files you manage. Cloud backups and production device sign-in are not available.</p>
  </div>;
}
