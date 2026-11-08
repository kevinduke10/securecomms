import {clientFetch} from './transport';
import { useState } from 'react';
import './group-lifecycle.css';
type Member = { profile: string; name: string; label: string; fingerprint: string; owner: boolean };
export default function GroupLifecycle({ profile, conversationId, members, onChanged }: { profile: string; conversationId: string; members: Member[]; onChanged: () => Promise<void> }) {
  const [action, setAction] = useState<'transfer' | 'remove'>('transfer');
  const [target, setTarget] = useState(''); const [fingerprint, setFingerprint] = useState('');
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());
  const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [status, setStatus] = useState('');
  const candidates = members.filter(member => !member.owner);
  const selected = candidates.find(member => member.profile === target);
  const normalized = fingerprint.replace(/\s/g, '').toLowerCase();
  function reset() { setTarget(''); setFingerprint(''); setRequestId(crypto.randomUUID()); setError(''); setStatus(''); }
  async function submit() {
    if (!selected || normalized !== selected.fingerprint || busy) return;
    setBusy(true); setError(''); setStatus('');
    try {
      const response = await clientFetch(`/local-api/${encodeURIComponent(profile)}/conversations/${encodeURIComponent(conversationId)}/lifecycle`, { method: 'POST', cache: 'no-store', headers: { 'x-croaker-local': '1', 'content-type': 'application/json' }, body: JSON.stringify({ request_id: requestId, target_profile: target, fingerprint: normalized, action }), signal: AbortSignal.timeout(30_000) });
      if (response.status === 401) window.dispatchEvent(new Event('croaker-signin-required'));
      if (!response.ok) { const value = await response.json().catch(() => ({})); throw new Error(value.error || 'Group access could not be updated.'); }
      reset(); setStatus(action === 'transfer' ? 'Ownership transferred.' : 'Device removed. Group keys rotated.'); await onChanged();
    } catch (error) { setError((error as Error).message); } finally { setBusy(false); }
  }
  return <form className="group-lifecycle" aria-busy={busy} onSubmit={event => { event.preventDefault(); void submit(); }}>
    <h3>Group access</h3>
    <label className="field-label">Action<select disabled={busy} value={action} onChange={event => { setAction(event.target.value as 'transfer' | 'remove'); reset(); }}><option value="transfer">Transfer ownership</option><option value="remove">Remove device</option></select></label>
    <label className="field-label">Current member<select required disabled={busy} value={target} onChange={event => { setTarget(event.target.value); setFingerprint(''); setRequestId(crypto.randomUUID()); setStatus(''); setError(''); }}><option value="">Select a device</option>{candidates.map(member => <option key={member.profile} value={member.profile}>{member.name} — {member.label}</option>)}</select></label>
    <p className="group-members-note">{action === 'transfer' ? 'The selected device will control member invitations and removals. Your device will remain a member. Ownership requires the current owner’s keys; a lost owner cannot be replaced automatically.' : 'The selected device will lose access to future messages. Previously received messages remain on that device.'}</p>
    {selected && <><code className="group-member-fingerprint">{selected.fingerprint.match(/.{1,8}/g)?.join(' ')}</code><label className="field-label">Verify the device fingerprint<input value={fingerprint} disabled={busy} autoComplete="off" spellCheck={false} onChange={event => setFingerprint(event.target.value)} /></label></>}
    <button className="primary-button" disabled={busy || !selected || normalized !== selected.fingerprint}>{busy ? 'Updating group…' : action === 'transfer' ? 'Transfer ownership' : 'Remove device'}</button>
    {status && <p role="status" className="backup-status">{status}</p>}{error && <p role="alert" className="form-error">{error}</p>}
  </form>;
}
