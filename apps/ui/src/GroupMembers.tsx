import { clientFetch } from './transport';
import { useEffect, useState } from 'react';
import { Check, UserPlus } from 'lucide-react';
import './group-members.css';
import GroupLifecycle from './GroupLifecycle';

type Member = { profile: string; name: string; label: string; fingerprint: string; owner: boolean };
type Membership = { can_invite: boolean; members: Member[]; candidates: Member[] };
async function request<T>(profile: string, conversation: string, body?: unknown): Promise<T> {
  const response = await clientFetch(`/local-api/${encodeURIComponent(profile)}/conversations/${encodeURIComponent(conversation)}/members`, {
    method: body === undefined ? 'GET' : 'POST', cache: 'no-store',
    headers: { 'x-croaker-local': '1', ...(body === undefined ? {} : { 'content-type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(30_000),
  });
  if (response.status === 401) window.dispatchEvent(new Event('croaker-signin-required'));
  if (!response.ok) { const error = await response.json().catch(() => ({})); throw new Error(error.error || 'Group membership could not be updated.'); }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}
export default function GroupMembers({ profile, conversationId, onChanged }: { profile: string; conversationId: string; onChanged: () => Promise<void> }) {
  const [membership, setMembership] = useState<Membership | null>(null);
  const [target, setTarget] = useState(''); const [fingerprint, setFingerprint] = useState('');
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());
  const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [status, setStatus] = useState('');
  useEffect(() => {
    let mounted = true;
    const load = () => request<Membership>(profile, conversationId).then(value => { if (mounted) setMembership(value); }).catch(error => { if (mounted) setError(error.message); });
    void load(); const timer = setInterval(() => void load(), 3000);
    return () => { mounted = false; clearInterval(timer); };
  }, [profile, conversationId]);
  const candidate = membership?.candidates.find(member => member.profile === target);
  const normalized = fingerprint.replace(/\s/g, '').toLowerCase();
  async function invite() {
    if (!candidate || !membership?.can_invite || normalized !== candidate.fingerprint || busy) return;
    setBusy(true); setError(''); setStatus('');
    try {
      await request(profile, conversationId, { request_id: requestId, target_profile: target, fingerprint: normalized });
      setMembership(await request<Membership>(profile, conversationId));
      setTarget(''); setFingerprint(''); setRequestId(crypto.randomUUID());
      setStatus('Device added. It can receive new messages; previous messages were not transferred.');
      await onChanged();
    } catch (error) { setError((error as Error).message); }
    finally { setBusy(false); }
  }
  return <section className="group-members" aria-busy={busy}>
    <p className="modal-intro">Only the conversation owner can change group access. Verify the target device fingerprint.</p>
    <h3>Current members</h3>
    {!membership && !error && <p className="group-members-note" role="status">Loading members…</p>}
    <ul className="group-member-list">{membership?.members.map(member => <li key={member.profile}><span><strong>{member.name}</strong><small>{member.label}{member.owner ? ' · Conversation owner' : ''}</small></span><Check size={16} aria-hidden="true" /></li>)}</ul>
    {membership && !membership.can_invite && <p className="group-members-note">Reconnect as the conversation owner to add a device.</p>}
    {membership?.can_invite && <form onSubmit={event => { event.preventDefault(); void invite(); }}>
      <h3>Add an approved device</h3>
      {!membership.candidates.length ? <p className="group-members-note">No additional approved devices are available. Approve and activate a replacement in Devices first.</p> : <>
        <label className="field-label">Device<select required value={target} disabled={busy} onChange={event => { setTarget(event.target.value); setFingerprint(''); setRequestId(crypto.randomUUID()); setStatus(''); setError(''); }}><option value="">Select a device</option>{membership.candidates.map(member => <option value={member.profile} key={member.profile}>{member.name} — {member.label}</option>)}</select></label>
        {candidate && <><p className="group-members-note">Compare this fingerprint with the device using a trusted channel.</p><code className="group-member-fingerprint">{candidate.fingerprint.match(/.{1,8}/g)?.join(' ')}</code><label className="field-label">Enter the fingerprint shown on the device<input value={fingerprint} onChange={event => setFingerprint(event.target.value)} autoComplete="off" spellCheck={false} disabled={busy} /></label></>}
        <button className="primary-button" disabled={busy || !candidate || normalized !== candidate.fingerprint}><UserPlus size={15} />{busy ? 'Adding device…' : 'Add device to group'}</button>
      </>}
    </form>}
    {membership?.can_invite && <GroupLifecycle profile={profile} conversationId={conversationId} members={membership.members} onChanged={async () => { setMembership(await request<Membership>(profile, conversationId)); await onChanged(); }} />}
    {status && <p className="backup-status" role="status">{status}</p>}{error && <p className="form-error" role="alert">{error}</p>}
    <p className="group-members-note">Enrollment and group access are separate. A history backup does not grant group access. This local test hosts all device keys on this Mac.</p>
  </section>;
}
