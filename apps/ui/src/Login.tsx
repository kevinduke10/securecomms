import { clientFetch } from './transport';
import { useCallback, useEffect, useState } from 'react';
import { ArrowRight, LogIn, ShieldCheck } from 'lucide-react';
import App from './App';
import { isNative } from './transport';
import DesktopSetup from './DesktopSetup';
export type Binding = { tenant_id: string; device_id: string; signature_key: number[] };
export type Account = { id: string; name: string; role: string; devices: { binding: Binding; label: string; approved_at: number; revoked: boolean }[]; requests: { id: string; label: string; binding: Binding; fingerprint: string; status: 'pending'|'approved'|'rejected'; expires_at: number }[] };
export type LoginSession = { authenticated: boolean; desktop?:boolean; signed_in?:boolean; auto_login: boolean; mode: 'demo'|'cognito'; profile?: string; owner?: string; account?: Account; local_devices?: { alias: string; device_id: string; label: string }[] };
export async function authRequest<T>(path: string, body?: unknown): Promise<T> {
  const response = await clientFetch(`/local-auth${path}`, { method: body === undefined ? 'GET' : 'POST', cache: 'no-store', headers: { 'x-croaker-local': '1', ...(body === undefined ? {} : { 'content-type': 'application/json' }) }, body: body === undefined ? undefined : JSON.stringify(body) });
  if (!response.ok) { const error = await response.json().catch(() => ({})); throw new Error(error.error || 'Sign-in or device operation failed.'); }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}
export default function Login() {
  const [session, setSession] = useState<LoginSession | null>(null); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const refresh = useCallback(async () => { try { setSession(await authRequest<LoginSession>('/me')); } catch (error) { setError((error as Error).message); } }, []);
  useEffect(() => { const expired = () => { setSession(previous => previous ? { ...previous, authenticated: false } : null); }; window.addEventListener('croaker-signin-required',expired); return ()=>window.removeEventListener('croaker-signin-required',expired); }, []);
  useEffect(() => { void refresh(); const timer = setInterval(() => void refresh(), 20_000); return () => clearInterval(timer); }, [refresh]);
  async function signIn(profile: string) { setBusy(true); setError(''); try { await authRequest('/demo', { profile }); history.replaceState(null, '', '/'); await refresh(); } catch (error) { setError((error as Error).message); } finally { setBusy(false); } }
  async function signOut() {
    const result = await authRequest<{ redirect: string | null }>('/logout', {}); setSession({ authenticated: false, auto_login: false, mode: session?.mode || 'demo' }); history.replaceState(null, '', '/');
    if (result.redirect) location.assign(result.redirect);
  }
  if (session?.authenticated) return <App key={session.profile || 'demo'} session={session} onSignOut={() => void signOut().catch(error => setError(error.message))} onSessionChange={refresh} />;
  if (isNative()) return <DesktopSetup session={session} onRefresh={refresh} error={error}/>;
  return <main className="login-page"><section className="login-card"><img src="/cirra.svg" alt="Cirra" /><h1>Sign in to Cirra</h1>
    {!session ? <p role="status">Loading…</p> : session.mode === 'cognito' ? <><a className="primary-button" href="/local-auth/start"><LogIn size={16} />Sign in<ArrowRight size={16} /></a><p className="login-note">Use your invited business account. Your organization’s sign-in page handles your password and verification.</p></> : <><div className="login-demo-label"><ShieldCheck size={16} />Local development sign-in</div><p className="login-note">Choose a test account on this Mac. This demo does not verify a password or provide production device isolation.</p><div className="login-accounts">{[['alice','Alice Morgan','Product & strategy'],['bob','Bob Chen','Engineering'],['maya','Maya Patel','Guest · Design partner']].map(([id,name,title]) => <button key={id} aria-label={`Sign in as ${name}`} disabled={busy} onClick={() => void signIn(id)}><span className={`avatar ${id}`}>{name.split(' ').map(n=>n[0]).join('')}</span><span><strong>Sign in as {name}</strong><small>{title}</small></span><ArrowRight size={16} /></button>)}</div></>}
    {(error || new URLSearchParams(location.search).has('login_error')) && <p className="form-error" role="alert">{error || 'Sign-in was not completed. Try again or contact your administrator.'}</p>}
  </section></main>;
}
