#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
command -v wasm-bindgen >/dev/null || { echo 'Install wasm-bindgen-cli matching Cargo.lock before building.' >&2; exit 1; }
rustup target add wasm32-unknown-unknown
# Apple clang does not include a WebAssembly backend; Zig supplies one.
if [[ "$(uname -s)" == Darwin && -z "${CC_wasm32_unknown_unknown:-}" ]]; then
  command -v zig >/dev/null || { echo 'Install Zig for the SQLite WebAssembly build.' >&2; exit 1; }
  mkdir -p target/browser-tools
  cat > target/browser-tools/zig-cc <<'PY'
#!/usr/bin/env python3
import os, shutil, sys
args=[a for a in sys.argv[1:] if a!='--target=wasm32-unknown-unknown']
zig=shutil.which('zig')
os.execv(zig,[zig,'cc','-target','wasm32-freestanding']+args)
PY
  chmod +x target/browser-tools/zig-cc
  for tool in ar ranlib; do
    printf '#!/usr/bin/env bash\nexec zig %s "$@"\n' "$tool" > "target/browser-tools/zig-$tool"
    chmod +x "target/browser-tools/zig-$tool"
  done
  export CC_wasm32_unknown_unknown="$PWD/target/browser-tools/zig-cc"
  export AR_wasm32_unknown_unknown="$PWD/target/browser-tools/zig-ar"
  export RANLIB_wasm32_unknown_unknown="$PWD/target/browser-tools/zig-ranlib"
fi
cargo build --release --locked -p cirra-device --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir apps/ui/src/browser/generated target/wasm32-unknown-unknown/release/cirra_device.wasm
if [[ -n "${VITE_CIRRA_RELAY:-}" ]]; then
  npx tsc -p apps/ui/tsconfig.browser.json
  VITE_CIRRA_BROWSER=true npm run build:ui
  python3 - <<'PYVERIFY'
from pathlib import Path
assert list(Path('target/web-dist/assets').glob('*.wasm')), 'Hosted build is missing the Rust/WASM client; publication stopped.'
PYVERIFY
else
  echo 'Rust/WASM bindings built. Set VITE_CIRRA_RELAY to the HTTPS web origin to build hosted assets.'
fi
