import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import MessageBubble from './MessageBubble';
import { ArrowLeft, ArrowUp, ArrowUpRight, Check, CheckCheck, ChevronDown, CircleHelp, Download, FileText, LockKeyhole, MessageCircle, Paperclip, PanelRight, Search, Settings2, ShieldCheck, Smile, SquarePen, Users, Wifi, WifiOff, X } from 'lucide-react';

type Profile = { id: string; name: string; title: string; role: 'admin' | 'member' | 'guest'; online: boolean; fingerprint: string };
type Attachment = { name: string; mime: string; data: string; size: number };
type Reaction = { emoji: string; users: string[]; pending: boolean };
type Message = { id: string; sender: string; text: string; attachment: Attachment | null; sent_at: number; pending: boolean; reactions?: Reaction[] };
type Conversation = { id: string; title: string; kind: 'direct' | 'group'; members: string[]; owner: string; messages: Message[]; typing?: string[] };
type Snapshot = { test_mode: boolean; current_profile: string; profiles: Profile[]; conversations: Conversation[]; online: boolean; attachment_limit: number; reaction_options?: string[] };
type Draft = { text: string; attachment: Attachment | null; requestId: string };
const REACTION_OPTIONS = ['👍', '❤️', '😂', '🎉', '😮', '😢'];

async function api<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/local-api${path}`, {
    method: body === undefined ? 'GET' : 'POST', cache: 'no-store',
    headers: { 'x-croaker-local': '1', ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok) {
    const error = await response.json().catch(() => ({}));
    throw new Error(error.error || 'Something went wrong. Please try again.');
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}

function Avatar({ profile, size = '', group = false }: { profile?: Profile; size?: string; group?: boolean }) {
  return <span aria-hidden="true" className={`avatar ${group ? 'group-avatar' : profile?.id || ''} ${size}`}>
    {group ? <Users size={19} strokeWidth={1.6} /> : profile?.name.split(' ').map(n => n[0]).join('') || '?'}
  </span>;
}
function IconButton({ label, children, onClick, active = false, className = '' }: { label: string; children: ReactNode; onClick: () => void; active?: boolean; className?: string }) {
  return <button type="button" className={`icon-button ${active ? 'is-active' : ''} ${className}`} aria-label={label} title={label} onClick={onClick}>{children}</button>;
}
function Modal({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => { const el = dialog.current!; el.showModal(); return () => el.close(); }, []);
  return <dialog ref={dialog} className="modal" onCancel={onClose} onClick={e => { if (e.target === e.currentTarget) onClose(); }} aria-label={title}>
    <div className="modal-head"><h2>{title}</h2><IconButton label="Close dialog" onClick={onClose}><X size={19} /></IconButton></div>{children}
  </dialog>;
}
const time = (timestamp: number) => new Date(timestamp * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
function day(timestamp: number) {
  const date = new Date(timestamp * 1000);
  return date.toDateString() === new Date().toDateString() ? 'Today' : date.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' });
}
const preview = (message?: Message) => message?.text || (message?.attachment ? `📎 ${message.attachment.name}` : 'Start a conversation');

function FileAttachment({ file }: { file: Attachment }) {
  const image = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(file.mime);
  return <div className="message-attachment">
    {image && <img className="attached-image" src={`data:${file.mime};base64,${file.data}`} alt={file.name} loading="lazy" />}
    <a className="file-card" href={`data:application/octet-stream;base64,${file.data}`} download={file.name} aria-label={`Download ${file.name}`}>
      <span className="file-icon"><FileText size={21} /></span><span><strong>{file.name}</strong><small>{file.size < 1024 ? `${file.size} bytes` : `${(file.size / 1024).toFixed(1)} KB`} · Encrypted file</small></span><Download size={17} />
    </a>
  </div>;
}

function Composer({ draft, setDraft, onSend, onTyping, busy, online, onError, limit }: { draft: Draft; setDraft: (next: Draft) => void; onSend: () => void; onTyping: (active: boolean) => void; busy: boolean; online: boolean; onError: (message: string) => void; limit: number }) {
  const fileInput = useRef<HTMLInputElement>(null);
  const [emoji, setEmoji] = useState(false);
  const typingCallback = useRef(onTyping); typingCallback.current = onTyping;
  const lastPulse = useRef(0); const idle = useRef<ReturnType<typeof setTimeout> | null>(null);
  function stopTyping() {
    if (idle.current) clearTimeout(idle.current);
    if (lastPulse.current) { lastPulse.current = 0; typingCallback.current(false); }
  }
  function announceTyping(text: string) {
    if (!online || !text.trim()) { stopTyping(); return; }
    if (Date.now() - lastPulse.current >= 2000) { lastPulse.current = Date.now(); typingCallback.current(true); }
    if (idle.current) clearTimeout(idle.current);
    idle.current = setTimeout(stopTyping, 2600);
  }
  useEffect(() => {
    const hidden = () => { if (document.hidden) stopTyping(); };
    window.addEventListener('blur', stopTyping); document.addEventListener('visibilitychange', hidden);
    return () => { stopTyping(); window.removeEventListener('blur', stopTyping); document.removeEventListener('visibilitychange', hidden); };
  }, []);
  useEffect(() => { if (!online) stopTyping(); }, [online]);
  const update = (text: string, attachment = draft.attachment) => setDraft({ text, attachment, requestId: crypto.randomUUID() });
  async function attach(file?: File) {
    if (!file) return;
    if (file.size > limit) { onError('For this local demo, choose a file smaller than 32 KB.'); return; }
    try {
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader(); reader.onload = () => resolve(String(reader.result).split(',')[1]); reader.onerror = reject; reader.readAsDataURL(file);
      });
      update(draft.text, { name: file.name, mime: file.type || 'application/octet-stream', data, size: file.size });
    } catch { onError('That file couldn’t be opened. Please choose it again.'); }
  }
  function submit(event: FormEvent) { event.preventDefault(); if (!busy && (draft.text.trim() || draft.attachment)) { stopTyping(); setEmoji(false); onSend(); } }
  return <div className="composer-area">
    {!online && <div className="offline-note"><WifiOff size={14} /> You’re offline. Your messages will send when you reconnect.</div>}
    <form className={`composer ${busy ? 'sending' : ''}`} onSubmit={submit} onDragOver={e => { if (e.dataTransfer.types.includes('Files')) e.preventDefault(); }} onDrop={e => { e.preventDefault(); void attach(e.dataTransfer.files[0]); }}>
      {draft.attachment && <div className="draft-file"><FileText size={17} /><span>{draft.attachment.name}</span><IconButton label="Remove attachment" onClick={() => update(draft.text, null)}><X size={15} /></IconButton></div>}
      <textarea aria-label="Message" placeholder="A little thought, a big idea…" value={draft.text} onChange={e => { update(e.target.value); announceTyping(e.target.value); }} onBlur={stopTyping} disabled={busy} maxLength={8000} rows={2} onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); if (!busy && (draft.text.trim() || draft.attachment)) { stopTyping(); onSend(); } } }} />
      <div className="composer-bottom"><div className="composer-tools">
        <IconButton label="Attach a file (up to 32 KB)" onClick={() => fileInput.current?.click()}><Paperclip size={19} /></IconButton>
        <input ref={fileInput} type="file" aria-label="Choose an attachment" hidden onChange={e => { void attach(e.target.files?.[0]); e.currentTarget.value = ''; }} />
        <div className="emoji-wrap"><IconButton label="Add an emoji" onClick={() => setEmoji(!emoji)} active={emoji}><Smile size={19} /></IconButton>{emoji && <div className="emoji-picker" role="group" aria-label="Emoji choices">{['👋', '✨', '🙌', '💚', '👍', '🎉'].map(value => <button key={value} type="button" aria-label={`Insert ${value}`} onClick={() => { update(draft.text + value); setEmoji(false); }}>{value}</button>)}</div>}</div>
      </div><span className="enter-hint">Enter to send <span>↵</span></span><button className="send-button" type="submit" disabled={busy || (!draft.text.trim() && !draft.attachment)} aria-label="Send message" title={online ? 'Send message' : 'Queue message'}><ArrowUp size={19} strokeWidth={2.3} /></button></div>
    </form>
    <div className="composer-caption"><LockKeyhole size={11} /> A private space. An open conversation.</div>
  </div>;
}

function NewConversation({ profiles, current, onClose, onCreate }: { profiles: Profile[]; current: string; onClose: () => void; onCreate: (title: string, kind: string, members: string[]) => Promise<void> }) {
  const [kind, setKind] = useState('group'); const [title, setTitle] = useState(''); const [members, setMembers] = useState<string[]>([]); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) { event.preventDefault(); setBusy(true); setError(''); try { await onCreate(title, kind, members); onClose(); } catch (error) { setError((error as Error).message); } finally { setBusy(false); } }
  return <Modal title="Start a conversation" onClose={onClose}><p className="modal-intro">Good things start with a conversation. Who’s joining?</p><form onSubmit={e => { void submit(e); }}>
    <div className="segmented"><button type="button" className={kind === 'group' ? 'selected' : ''} onClick={() => setKind('group')}><Users size={16} /> Group</button><button type="button" className={kind === 'direct' ? 'selected' : ''} onClick={() => { setKind('direct'); setMembers(members.slice(0, 1)); }}><MessageCircle size={16} /> Direct message</button></div>
    {kind === 'group' && <label className="field-label">Group name<input autoFocus placeholder="e.g. The next big thing" value={title} minLength={2} maxLength={48} required onChange={e => setTitle(e.target.value)} /></label>}
    <div className="field-label">{kind === 'group' ? 'Add people' : 'Choose a person'}</div><div className="people-list">{profiles.filter(p => p.id !== current).map(p => <label className="person-choice" key={p.id}><Avatar profile={p} /><span><strong>{p.name}</strong><small>{p.title}{p.role === 'guest' ? ' · Guest' : ''}</small></span><input aria-label={p.name} type={kind === 'group' ? 'checkbox' : 'radio'} name="members" checked={members.includes(p.id)} onChange={() => setMembers(kind === 'direct' ? [p.id] : members.includes(p.id) ? members.filter(id => id !== p.id) : [...members, p.id])} /></label>)}</div>
    {error && <p className="form-error" role="alert">{error}</p>}<div className="modal-actions"><button type="button" className="secondary-button" onClick={onClose}>Cancel</button><button className="primary-button" disabled={busy || !members.length || (kind === 'group' && title.trim().length < 2)}>{busy ? 'Creating…' : 'Start conversation'}<ArrowUpRight size={16} /></button></div>
  </form></Modal>;
}

export default function App() {
  const initial = new URLSearchParams(location.search).get('as') || 'alice';
  const [profile, setProfile] = useState(['alice', 'bob', 'maya'].includes(initial) ? initial : 'alice');
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null); const [selected, setSelected] = useState(() => new URLSearchParams(location.search).get('chat') || '');
  const [search, setSearch] = useState(''); const [filter, setFilter] = useState('All'); const [messageSearch, setMessageSearch] = useState<string | null>(null);
  const [details, setDetails] = useState(false); const [mobileChat, setMobileChat] = useState(false); const [accountMenu, setAccountMenu] = useState(false);
  const [modal, setModal] = useState<'new' | 'help' | null>(null); const [busy, setBusy] = useState(false); const [connectionError, setConnectionError] = useState(false); const [notice, setNotice] = useState('');
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [seen, setSeen] = useState<Record<string, number>>(() => { try { return JSON.parse(localStorage.getItem('croaker:seen') || '{}'); } catch { return {}; } });
  const profileRef = useRef(profile); profileRef.current = profile;
  const searchInput = useRef<HTMLInputElement>(null); const messageList = useRef<HTMLDivElement>(null); const stickToBottom = useRef(true);
  const requestNumber = useRef(0); const appliedRequest = useRef(0);
  const refresh = useCallback(async () => {
    const wanted = profile; const number = ++requestNumber.current;
    try {
      const next = await api<Snapshot>(`/state?profile=${wanted}`);
      if (profileRef.current === wanted && number > appliedRequest.current) { appliedRequest.current = number; setSnapshot(next); setConnectionError(false); }
    } catch { if (profileRef.current === wanted) setConnectionError(true); }
  }, [profile]);
  useEffect(() => { void refresh(); const interval = setInterval(() => { void refresh(); }, 1200); return () => clearInterval(interval); }, [refresh]);
  useEffect(() => { if (notice) { const timeout = setTimeout(() => setNotice(''), 6500); return () => clearTimeout(timeout); } }, [notice]);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); setMobileChat(false); searchInput.current?.focus(); } if (event.key === 'Escape') { setAccountMenu(false); setMessageSearch(null); } };
    window.addEventListener('keydown', handler); return () => window.removeEventListener('keydown', handler);
  }, []);
  const data = snapshot?.current_profile === profile ? snapshot : null;
  const current = data?.profiles.find(p => p.id === profile);
  const chat = data?.conversations.find(c => c.id === selected) || data?.conversations[0];
  const getProfile = (id: string) => data?.profiles.find(p => p.id === id);
  const unread = (c: Conversation) => c.messages.slice(seen[`${profile}:${c.id}`] || 0).filter(m => m.sender !== profile).length;
  const lastId = chat?.messages.at(-1)?.id;
  const reactionLayout = chat?.messages.map(m => (m.reactions || []).map(r => `${r.emoji}:${r.users.join(',')}`).join('|')).join(';');
  useEffect(() => { if (chat) { const url = new URL(location.href); url.searchParams.set('chat',chat.id); history.replaceState(null,'',url); } }, [chat?.id]);
  useEffect(() => {
    if (chat) setSeen(previous => {
      const key = `${profile}:${chat.id}`; if (previous[key] === chat.messages.length) return previous;
      const next = { ...previous, [key]: chat.messages.length }; localStorage.setItem('croaker:seen', JSON.stringify(next)); return next;
    });
  }, [profile, chat?.id, chat?.messages.length]);
  useLayoutEffect(() => { if (stickToBottom.current && messageList.current) messageList.current.scrollTop = messageList.current.scrollHeight; }, [chat?.id, lastId, mobileChat, details, reactionLayout]);
  useEffect(() => { stickToBottom.current = true; setMessageSearch(null); }, [chat?.id]);
  function switchAccount(id: string) { setProfile(id); setSelected(''); setSearch(''); setFilter('All'); setAccountMenu(false); setMobileChat(false); const url = new URL(location.href); url.searchParams.set('as', id); url.searchParams.delete('chat'); history.replaceState(null, '', url); }
  async function toggleOnline() {
    if (!data) return; setBusy(true);
    try { await api(`/${profile}/online`, { online: !data.online }); await refresh(); } catch (error) { setNotice((error as Error).message); } finally { setBusy(false); }
  }
  async function sendMessage() {
    if (!chat || !data || busy) return;
    const key = `${profile}:${chat.id}`; const draft = drafts[key]; if (!draft || (!draft.text.trim() && !draft.attachment)) return;
    setBusy(true);
    try {
      await api(`/${profile}/send`, { conversation_id: chat.id, request_id: draft.requestId, content: { version: 1, text: draft.text.trim(), attachment: draft.attachment } });
      setDrafts(previous => { const next = { ...previous }; if (next[key]?.requestId === draft.requestId) delete next[key]; return next; });
      stickToBottom.current = true; await refresh();
    } catch (error) { setNotice((error as Error).message); } finally { setBusy(false); }
  }
  async function reactToMessage(message: Message, emoji: string | null) {
    if (!chat) return;
    try { await api(`/${profile}/reactions`, { conversation_id: chat.id, request_id: crypto.randomUUID(), target_message_id: message.id, target_sender: message.sender, emoji }); await refresh(); }
    catch (error) { setNotice((error as Error).message); }
  }
  const notifyTyping = useCallback((conversation: string, active: boolean) => {
    // Presence failures never interrupt composition or leak draft text.
    void api(`/${profile}/typing`, { conversation_id: conversation, active }).catch(() => {});
  }, [profile]);
  const visibleChats = data?.conversations.filter(c => (filter !== 'Groups' || c.kind === 'group') && (filter !== 'Unread' || unread(c) > 0) && `${c.title} ${preview(c.messages.at(-1))}`.toLowerCase().includes(search.toLowerCase())) || [];
  const draftKey = `${profile}:${chat?.id}`;
  const draft = drafts[draftKey] || { text: '', attachment: null, requestId: '' };

  return <div className="app-shell">
    <div className="test-strip"><span><span className="test-dot" /> LOCAL WORKSPACE</span><p>Your own little corner of the internet.</p><button onClick={() => setModal('help')}>How to test <ArrowUpRight size={12} /></button></div>
    <div className="workspace">
      <nav className="rail" aria-label="Workspace navigation"><a className="brand-mark" href="/" aria-label="Croaker home"><img src="/croaker.svg" alt="" /></a><div className="rail-separator" /><IconButton label="Messages" active onClick={() => { setMobileChat(false); searchInput.current?.focus(); }}><MessageCircle size={22} /></IconButton><IconButton label="Switch test account" active={accountMenu} onClick={() => setAccountMenu(!accountMenu)}><Users size={21} /></IconButton><IconButton label="Conversation privacy" active={details} onClick={() => setDetails(!details)}><ShieldCheck size={21} /></IconButton><div className="rail-bottom"><IconButton label="Local workspace settings" onClick={() => setModal('help')}><Settings2 size={20} /></IconButton><button className="rail-avatar" aria-label="Your account" onClick={() => setAccountMenu(!accountMenu)}><Avatar profile={current} size="small" /></button></div></nav>
      <aside className={`sidebar ${mobileChat ? 'mobile-hidden' : ''}`} aria-label="Conversations">
        <div className="workspace-heading"><div><h1>Croaker<span aria-hidden="true">●</span></h1><p>A space for your people.</p></div><span className="workspace-badge"><ShieldCheck size={16} /></span></div>
        <div className="inbox-heading"><h2>Messages <span>{data?.conversations.length ?? '—'}</span></h2><IconButton label="New conversation" onClick={() => current?.role === 'guest' ? setNotice('Guests can message in existing conversations. Switch to Alice or Bob to create one.') : setModal('new')}><SquarePen size={18} /></IconButton></div>
        <div className="search-box"><Search size={16} /><input ref={searchInput} aria-label="Search conversations" placeholder="Find a conversation" value={search} onChange={e => setSearch(e.target.value)} /><kbd>⌘ K</kbd></div>
        <div className="conversation-filters" aria-label="Filter conversations">{['All', 'Unread', 'Groups'].map(value => <button key={value} className={filter === value ? 'active' : ''} onClick={() => setFilter(value)}>{value}</button>)}</div>
        <div className="conversation-list">
          {!data && <div className="sidebar-loading"><span className="loading-dot" /> Opening your workspace…</div>}
          {data && !visibleChats.length && <div className="empty-list"><Search size={23} /><strong>{filter === 'Unread' ? 'All caught up' : 'No conversations found'}</strong><p>{filter === 'Unread' ? 'A little breathing room. Enjoy it.' : 'Try another name or start a new conversation.'}</p></div>}
          {visibleChats.map(c => {
            const other = getProfile(c.members.find(id => id !== profile) || ''); const last = c.messages.at(-1); const count = unread(c);
            return <button key={c.id} className={`conversation-row ${chat?.id === c.id ? 'selected' : ''}`} onClick={() => { stickToBottom.current = true; setSelected(c.id); setMobileChat(true); }} aria-label={`Open ${c.title}`} aria-current={chat?.id === c.id ? 'true' : undefined}>
              <span className="conversation-avatar"><Avatar profile={other} group={c.kind === 'group'} />{c.kind === 'direct' && other?.online && <span className="online-dot" />}</span>
              <span className="conversation-copy"><span className="conversation-title"><strong>{c.title}</strong>{last && <time>{time(last.sent_at)}</time>}</span><span className="conversation-preview"><span>{last?.sender === profile && <CheckCheck size={13} />}{preview(last)}</span>{count > 0 && chat?.id !== c.id && <b>{count}</b>}</span></span>
            </button>;
          })}
        </div>
        <div className="sidebar-note"><span className="note-icon"><LockKeyhole size={18} /></span><div><strong>Good conversations stay between us.</strong><p>End-to-end encrypted. Always.</p></div></div>
        <button className="account-button" aria-label="Choose test account" aria-expanded={accountMenu} onClick={() => setAccountMenu(!accountMenu)}><Avatar profile={current} size="small" /><span><strong>{current?.name || 'Connecting…'}</strong><small><span className={`status-dot ${data?.online ? '' : 'offline'}`} />{data?.online ? 'Available' : 'Offline'}</small></span><ChevronDown size={16} /></button>
      </aside>

      <main className={`chat-panel ${!mobileChat ? 'mobile-hidden' : ''}`}>
        {connectionError && <div className="connection-banner" role="status"><WifiOff size={15} /> Can’t reach the local workspace. Retrying…<button onClick={() => { void refresh(); }}>Try now</button></div>}
        {chat && data ? <>
          <header className="chat-header"><IconButton className="mobile-back" label="Back to conversations" onClick={() => setMobileChat(false)}><ArrowLeft size={21} /></IconButton><Avatar profile={getProfile(chat.members.find(id => id !== profile) || '')} group={chat.kind === 'group'} /><div className="chat-heading"><h2>{chat.title}<LockKeyhole size={13} /></h2><span>{chat.kind === 'group' ? `${chat.members.length} members` : getProfile(chat.members.find(id => id !== profile) || '')?.title}<span className="middle-dot">·</span>{chat.kind === 'group' ? 'Private group' : 'Direct message'}</span></div><div className="header-members">{chat.members.slice(0, 3).map(id => <Avatar key={id} profile={getProfile(id)} size="tiny" />)}</div><div className="header-actions"><IconButton label="Search this conversation" active={messageSearch !== null} onClick={() => setMessageSearch(messageSearch === null ? '' : null)}><Search size={19} /></IconButton><IconButton label="Conversation details" active={details} onClick={() => setDetails(!details)}><PanelRight size={19} /></IconButton></div></header>
          {messageSearch !== null && <div className="message-search"><Search size={16} /><input autoFocus aria-label="Search messages" placeholder="Find a message in this conversation" value={messageSearch} onChange={e => setMessageSearch(e.target.value)} /><IconButton label="Close message search" onClick={() => setMessageSearch(null)}><X size={16} /></IconButton></div>}
          <div className="chat-messages" ref={messageList} onScroll={() => { const el = messageList.current!; stickToBottom.current = el.scrollHeight - el.clientHeight - el.scrollTop < 120; }}>
            <div className="conversation-intro"><span className="intro-lock"><LockKeyhole size={14} /></span><span>Just you and the people in this conversation.<br /><strong>Messages and files are end-to-end encrypted.</strong></span></div>
            {!chat.messages.length && <div className="empty-conversation"><div className="empty-chat-icon"><MessageCircle size={30} strokeWidth={1.4} /></div><h3>Make yourself at home.</h3><p>Every good conversation starts somewhere.<br />Send the first hello.</p></div>}
            {messageSearch && !chat.messages.some(m => `${m.text} ${m.attachment?.name || ''}`.toLowerCase().includes(messageSearch.toLowerCase())) && <div className="empty-conversation"><Search size={25} /><h3>No messages found</h3><p>Try a different word or phrase.</p></div>}
            {chat.messages.filter(m => !messageSearch || `${m.text} ${m.attachment?.name || ''}`.toLowerCase().includes(messageSearch.toLowerCase())).map((message, index, messages) => {
              const own = message.sender === profile; const sender = getProfile(message.sender); const showDay = index === 0 || day(messages[index - 1].sent_at) !== day(message.sent_at);
              return <div key={`${message.sender}:${message.id}`} className="message-unit">{showDay && <div className="day-divider"><span />{day(message.sent_at)}<span /></div>}<article className={`message-row ${own ? 'own' : ''}`} aria-label={`Message from ${sender?.name}`}>
                {!own && <Avatar profile={sender} size="small" />}<div className="message-content"><div className="message-author">{own ? 'You' : sender?.name}{sender?.role === 'guest' && !own && <span>Guest</span>}</div><MessageBubble message={message} profile={profile} profiles={data.profiles} options={data.reaction_options || REACTION_OPTIONS} onReact={emoji => reactToMessage(message,emoji)}>{message.text && <p>{message.text}</p>}{message.attachment && <FileAttachment file={message.attachment} />}</MessageBubble><div className="message-meta"><time>{time(message.sent_at)}</time>{own && <span>{message.pending ? <><span className="pending-dot" /> Waiting for connection</> : <><CheckCheck size={13} /> Sent</>}</span>}</div></div>
              </article></div>;
            })}
          </div>
          <div className="typing-indicator" role="status" aria-live="polite">{data.online && !!chat.typing?.length && <><span className="typing-dots" aria-hidden="true"><i /><i /><i /></span><span>{chat.typing.map(id => getProfile(id)?.name.split(' ')[0] || id).join(' and ')} {chat.typing.length === 1 ? 'is' : 'are'} typing…</span></>}</div>
          <Composer key={draftKey} draft={draft} setDraft={next => setDrafts(previous => ({ ...previous, [draftKey]: next }))} onSend={() => { void sendMessage(); }} onTyping={active => notifyTyping(chat.id,active)} busy={busy} online={data.online} onError={setNotice} limit={data.attachment_limit} />
        </> : <div className="opening-workspace"><img src="/croaker.svg" alt="" /><h2>{connectionError ? 'Let’s get you connected.' : 'A little privacy. A lot of possibility.'}</h2><p>{connectionError ? 'Keep npm run dev running in your terminal, then try again.' : 'Opening your private workspace…'}</p>{connectionError && <button className="primary-button" onClick={() => { void refresh(); }}>Try again</button>}</div>}
      </main>

      {details && chat && data && <aside className="details-panel" aria-label="Conversation details"><div className="details-heading"><h3>Conversation details</h3><IconButton label="Close details" onClick={() => setDetails(false)}><X size={17} /></IconButton></div><div className="details-identity"><Avatar group={chat.kind === 'group'} profile={getProfile(chat.members.find(id => id !== profile) || '')} size="large" /><h3>{chat.title}</h3><span>{chat.kind === 'group' ? 'A private space for your team' : 'Just the two of you'}</span></div><div className="privacy-card"><ShieldCheck size={24} strokeWidth={1.5} /><strong>Private by design.</strong><p>Messages and small files are encrypted by the Rust client before reaching the relay.</p><span><Check size={12} /> End-to-end encrypted</span></div><div className="members-heading"><h4>PEOPLE</h4><span>{chat.members.length}</span></div><div className="member-list">{chat.members.map(id => { const person = getProfile(id)!; return <div key={id} className="member-row"><Avatar profile={person} size="small" /><span><strong>{person.name}{id === profile ? ' (you)' : ''}</strong><small>{person.role === 'guest' ? 'Guest' : chat.owner === id ? 'Conversation owner' : person.title}</small></span><span className={`status-dot ${person.online ? '' : 'offline'}`} /></div>; })}</div><div className="details-footer"><LockKeyhole size={15} /><p>This is a local test workspace.<br />All test accounts run on this Mac.</p><button onClick={() => setModal('help')}>About this demo <ArrowUpRight size={12} /></button></div></aside>}
    </div>

    {accountMenu && <><button className="popover-backdrop" aria-label="Dismiss account switcher" onClick={() => setAccountMenu(false)} /><div className="account-popover"><div className="popover-heading"><span>YOUR TEST ACCOUNTS</span><IconButton label="Close account menu" onClick={() => setAccountMenu(false)}><X size={15} /></IconButton></div>{(data || snapshot)?.profiles.map(person => <div className="account-option" key={person.id}><button onClick={() => switchAccount(person.id)}><Avatar profile={person} size="small" /><span><strong>{person.name}</strong><small>{person.role === 'guest' ? 'Guest account' : person.title}</small></span>{person.id === profile && <Check size={16} />}</button><a href={`/?as=${person.id}`} target="_blank" rel="noreferrer" aria-label={`Open ${person.name} in a new tab`} title="Open in a new tab"><ArrowUpRight size={17} /></a></div>)}<button className="connection-toggle" disabled={busy} onClick={() => { void toggleOnline(); }}>{data?.online ? <WifiOff size={16} /> : <Wifi size={16} />}{data?.online ? 'Go offline' : 'Reconnect'}<span>{data?.online ? 'Test queued messages' : 'Send queued messages'}</span></button><p>Open another account in a new tab to chat with yourself.</p></div></>}
    {modal === 'new' && data && <NewConversation profiles={data.profiles} current={profile} onClose={() => setModal(null)} onCreate={async (title, kind, members) => { const result = await api<{ id: string }>(`/${profile}/conversations`, { title, kind, members }); setSelected(result.id); stickToBottom.current = true; setMobileChat(true); setSearch(''); setFilter('All'); await refresh(); }} />}
    {modal === 'help' && <Modal title="Your local playground" onClose={() => setModal(null)}><div className="help-badge"><ShieldCheck size={18} /> Real encryption. Room to experiment.</div><p className="modal-intro">Try Croaker with three test accounts, all running on this machine. No AWS account or signup needed.</p><ol className="test-instructions"><li><span>01</span><div><strong>Invite yourself over.</strong><p>Open Bob or Maya in a second tab using the account menu. Send a message and watch it arrive.</p></div></li><li><span>02</span><div><strong>Give it a little space.</strong><p>Choose “Go offline” in the account menu. Queue a message, then reconnect to send it.</p></div></li><li><span>03</span><div><strong>Make it your own.</strong><p>Create a group, attach a small image or file (up to 32 KB), and refresh. Your workspace is saved locally.</p></div></li></ol><div className="demo-disclosure"><strong>A development workspace, for test data.</strong><p>The local Rust process hosts all three client accounts and their keys. The relay stores ciphertext. This is not production login or device isolation; do not use it for confidential conversations.</p></div><div className="modal-actions"><button className="primary-button" onClick={() => setModal(null)}>Make yourself at home <ArrowUpRight size={16} /></button></div></Modal>}
    {notice && <div className="toast" role="alert"><CircleHelp size={18} /><span>{notice}</span><IconButton label="Dismiss notification" onClick={() => setNotice('')}><X size={16} /></IconButton></div>}
  </div>;
}
