import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { ArrowLeft, Plus, Search, X } from 'lucide-react';
type EmojiEntry = { emoji: string; name: string; keywords: string[]; search: string };
let catalogPromise: Promise<EmojiEntry[]> | undefined;
function loadCatalog() {
  return catalogPromise ??= import('../../../shared/emoji.json').then(({ default: entries }) => entries.map(entry => ({ ...entry, search: `${entry.name} ${entry.keywords.join(' ')}`.toLowerCase().replaceAll('_', ' ') }))).catch(error => { catalogPromise = undefined; throw error; });
}
const quickNames: Record<string,string> = { '👍': 'Thumbs up', '❤️': 'Heart', '😂': 'Laughing', '🎉': 'Celebrate', '😮': 'Surprised', '😢': 'Sad' };
const HOLD_MS = 500;
type Position = { top: number; left: number; maxHeight: number };
type Props = {
  message: { reactions?: { emoji: string; users: string[]; pending: boolean }[] };
  profile: string; profiles: { id: string; name: string }[]; options: string[];
  onReact: (emoji: string | null) => Promise<void>; children: ReactNode;
};

export default function MessageBubble({ message, profile, profiles, options, onReact, children }: Props) {
  const [position, setPosition] = useState<Position | null>(null);
  const [expanded, setExpanded] = useState(false); const [query, setQuery] = useState(''); const [busy, setBusy] = useState(false);
  const [catalog, setCatalog] = useState<EmojiEntry[]>([]); const [loading, setLoading] = useState(false); const [loadError, setLoadError] = useState(false);
  const bubble = useRef<HTMLDivElement>(null); const picker = useRef<HTMLDivElement>(null); const search = useRef<HTMLInputElement>(null);
  const holdTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pointer = useRef<{ id: number; x: number; y: number } | null>(null); const held = useRef(false);
  const reactions = message.reactions || [];
  const mine = reactions.find(reaction => reaction.users.includes(profile))?.emoji;
  const tokens = query.toLowerCase().trim().replace(/[_:]/g, ' ').split(/\s+/).filter(Boolean);
  const results = tokens.length ? catalog.filter(entry => entry.emoji === query.trim() || tokens.every(token => entry.search.includes(token))) : catalog;

  function cancelHold() {
    if (holdTimer.current) clearTimeout(holdTimer.current);
    holdTimer.current = null; pointer.current = null;
  }
  function place(isExpanded: boolean): Position | null {
    const rect = bubble.current?.getBoundingClientRect();
    if (!rect || rect.bottom < 0 || rect.top > innerHeight) return null;
    const width = Math.min(isExpanded ? 328 : 294, innerWidth - 20);
    const viewportTop = window.visualViewport?.offsetTop || 0;
    const viewportHeight = window.visualViewport?.height || innerHeight;
    const height = Math.min(isExpanded ? 366 : 54, viewportHeight - 20);
    return {
      left: Math.max(10, Math.min(innerWidth - width - 10, rect.right - width)),
      top: Math.max(viewportTop + 10, Math.min(viewportTop + viewportHeight - height - 10, rect.top >= viewportTop + height + 18 ? rect.top - height - 10 : rect.bottom + 10)),
      maxHeight: height,
    };
  }
  function close() { setPosition(null); setExpanded(false); setQuery(''); }
  function open() {
    if (busy) return;
    cancelHold();
    const selection = window.getSelection();
    if (selection?.anchorNode && bubble.current?.contains(selection.anchorNode)) selection.removeAllRanges();
    setExpanded(false); setQuery(''); setPosition(place(false));
  }
  function startHold(event: ReactPointerEvent<HTMLDivElement>) {
    cancelHold(); held.current = false;
    if (!event.isPrimary || event.button !== 0 || busy) return;
    if (position) { close(); return; }
    pointer.current = { id: event.pointerId, x: event.clientX, y: event.clientY };
    holdTimer.current = setTimeout(() => { held.current = true; open(); }, HOLD_MS);
  }
  function move(event: ReactPointerEvent<HTMLDivElement>) {
    const origin = pointer.current;
    if (origin && origin.id === event.pointerId && Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > 10) cancelHold();
  }
  useEffect(() => {
    const cancel = () => cancelHold();
    window.addEventListener('pointerup', cancel); window.addEventListener('pointercancel', cancel); window.addEventListener('blur', cancel); window.addEventListener('scroll', cancel, true);
    return () => { cancelHold(); window.removeEventListener('pointerup', cancel); window.removeEventListener('pointercancel', cancel); window.removeEventListener('blur', cancel); window.removeEventListener('scroll', cancel, true); };
  }, []);
  useLayoutEffect(() => { if (position) setPosition(place(expanded)); }, [expanded]);
  useEffect(() => {
    if (!position) return;
    if (expanded) search.current?.focus({ preventScroll: true }); else picker.current?.querySelector('button')?.focus({ preventScroll: true });
    const outside = (event: PointerEvent) => { if (!picker.current?.contains(event.target as Node) && !bubble.current?.contains(event.target as Node)) close(); };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { close(); bubble.current?.focus({ preventScroll: true }); } };
    const reposition = () => { const next = place(expanded); setPosition(previous => !next ? null : previous && (previous.top !== next.top || previous.left !== next.left || previous.maxHeight !== next.maxHeight) ? next : previous); };
    document.addEventListener('pointerdown', outside); document.addEventListener('keydown', escape); window.addEventListener('resize', reposition); window.addEventListener('scroll', reposition, true); window.visualViewport?.addEventListener('resize', reposition); window.visualViewport?.addEventListener('scroll', reposition);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape); window.removeEventListener('resize', reposition); window.removeEventListener('scroll', reposition, true); window.visualViewport?.removeEventListener('resize', reposition); window.visualViewport?.removeEventListener('scroll', reposition); };
  }, [position, expanded]);
  async function choose(emoji: string) {
    if (busy) return; setBusy(true); close();
    try { await onReact(mine === emoji ? null : emoji); } finally { setBusy(false); bubble.current?.focus({ preventScroll: true }); }
  }
  async function expand() {
    setExpanded(true); setLoading(true); setLoadError(false);
    try { setCatalog(await loadCatalog()); } catch { setLoadError(true); } finally { setLoading(false); }
  }
  return <>
    <div ref={bubble} className={`message-bubble ${position ? 'reaction-open' : ''}`} tabIndex={0} role="group" aria-label="Message" aria-haspopup="dialog" aria-description="Press and hold to react. You can also right-click, or press Enter or Space."
      onPointerDown={startHold} onPointerMove={move} onPointerUp={cancelHold} onPointerCancel={() => { cancelHold(); held.current = false; }} onPointerLeave={cancelHold} onDragStart={cancelHold}
      onClickCapture={event => { if (held.current) { event.preventDefault(); event.stopPropagation(); held.current = false; } }}
      onContextMenu={event => { event.preventDefault(); cancelHold(); open(); }}
      onKeyDown={event => { if (event.target === event.currentTarget && (event.key === 'Enter' || event.key === ' ' || event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey))) { event.preventDefault(); open(); } }}>
      {children}
    </div>
    {!!reactions.length && <div className="reaction-bar">{reactions.map(reaction => {
      const own = reaction.users.includes(profile);
      const reactors = reaction.users.map(id => profiles.find(p => p.id === id)?.name || id).join(', ');
      return <button key={reaction.emoji} type="button" className={`reaction-chip ${own ? 'mine' : ''} ${reaction.pending ? 'queued' : ''}`} disabled={busy} aria-pressed={own}
        aria-label={`${own ? 'Remove your' : 'Add'} ${reaction.emoji} reaction (${reaction.users.length}): ${reactors}`} title={`${reactors}${reaction.pending ? ' · Waiting for connection' : ''}`} onClick={() => { void choose(reaction.emoji); }}><span>{reaction.emoji}</span><span>{reaction.users.length}</span></button>;
    })}</div>}
    {position && createPortal(<div ref={picker} className={`reaction-picker ${expanded ? 'expanded' : ''}`} role="dialog" aria-label="Choose a reaction" style={position}>
      {!expanded ? <>
        {options.map(emoji => <button key={emoji} type="button" title={quickNames[emoji]} aria-label={`React with ${emoji}`} aria-pressed={mine === emoji} className={mine === emoji ? 'selected' : ''} onClick={() => { void choose(emoji); }}>{emoji}</button>)}
        <button type="button" className="more-emoji" aria-label="More emoji" title="More emoji" onClick={() => { void expand(); }}><Plus size={22} /></button>
      </> : <>
        <div className="emoji-search-heading"><button type="button" aria-label="Back to quick reactions" onClick={() => { setExpanded(false); setQuery(''); }}><ArrowLeft size={17} /></button><strong>Choose an emoji</strong><button type="button" aria-label="Close emoji picker" onClick={() => { close(); bubble.current?.focus({ preventScroll: true }); }}><X size={17} /></button></div>
        <label className="emoji-search"><Search size={15} /><input ref={search} type="search" aria-label="Search emoji" placeholder="Search emoji…" value={query} onChange={event => setQuery(event.target.value)} onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); picker.current?.querySelector<HTMLButtonElement>('.emoji-choice')?.focus(); } if (event.key === 'Enter' && results.length) { event.preventDefault(); void choose(results[0].emoji); } }} /></label>
        <div className="emoji-results" role="group" aria-label="Emoji search results" onKeyDown={event => {
          const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('.emoji-choice'));
          const index = buttons.indexOf(event.target as HTMLButtonElement);
          const offset = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 8, ArrowUp: -8 }[event.key];
          if (index >= 0 && offset !== undefined && buttons[index + offset]) { event.preventDefault(); buttons[index + offset].focus(); }
        }}>
          {results.map(entry => <button type="button" className={`emoji-choice ${mine === entry.emoji ? 'selected' : ''}`} key={entry.emoji} title={entry.name} aria-label={`React with ${entry.emoji} — ${entry.name}`} aria-pressed={mine === entry.emoji} onClick={() => { void choose(entry.emoji); }}>{entry.emoji}</button>)}
          {loading ? <p className="emoji-no-results" role="status">Opening your emoji library…</p> : loadError ? <div className="emoji-no-results" role="alert">Couldn’t open the emoji library.<br /><button type="button" onClick={() => { void expand(); }}>Try again</button></div> : !results.length && <p className="emoji-no-results" role="status">No emoji found. Try a different word.</p>}
        </div>
        <div className="emoji-search-footer" role="status">{loading ? 'Loading emoji…' : `${results.length.toLocaleString()} ${query.trim() ? 'matching emoji' : 'emoji to make it yours'}`}</div>
      </>}
    </div>, document.body)}
  </>;
}
