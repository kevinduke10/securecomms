import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const args = process.argv.slice(2);
let port = '3000';
let dataDir = 'data/ui';
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--port' && args[i + 1]) port = args[++i];
  else if (args[i] === '--data-dir' && args[i + 1]) dataDir = args[++i];
  else { console.error('Usage: npm run dev -- [--port 3000] [--data-dir data/ui]'); process.exit(1); }
}
if (!/^\d+$/.test(port) || Number(port) < 1024 || Number(port) > 65535) {
  console.error('Choose a port between 1024 and 65535.'); process.exit(1);
}
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const build = spawnSync(npm, ['run', 'build:ui'], { cwd: root, stdio: 'inherit' });
if (build.status !== 0) process.exit(build.status ?? 1);
const server = spawn('cargo', ['run', '--locked', '-p', 'croaker-local', '--', '--bind', `127.0.0.1:${port}`, '--data-dir', path.resolve(root, dataDir)], { cwd: root, stdio: 'inherit' });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.kill(signal));
server.on('error', error => { console.error(error.message); process.exit(1); });
server.on('exit', (code, signal) => { process.exit(code ?? (signal === 'SIGINT' ? 0 : 1)); });
