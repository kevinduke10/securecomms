import { createRequire } from 'node:module';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const require = createRequire(import.meta.url);
const root = fileURLToPath(new URL('../', import.meta.url));
const unicodePath = path.dirname(require.resolve('unicode-emoji-json/package.json'));
const keywordsPath = path.dirname(require.resolve('emojilib/package.json'));
const unicode = JSON.parse(readFileSync(path.join(unicodePath, 'data-by-emoji.json'), 'utf8'));
const keywords = require('emojilib');
const entries = Object.entries(unicode).map(([emoji, metadata]) => ({
  emoji, name: metadata.name,
  keywords: [...new Set([metadata.group, metadata.slug.replaceAll('_', ' '), ...(keywords[emoji] || [])])],
}));
mkdirSync(path.join(root, 'shared'), { recursive: true });
writeFileSync(path.join(root, 'shared/emoji.json'), `[\n${entries.map(entry => JSON.stringify(entry)).join(',\n')}\n]\n`);
for (const [name, source] of [['unicode-emoji-json', unicodePath], ['emojilib', keywordsPath]]) {
  writeFileSync(path.join(root, `shared/${name}.LICENSE`), readFileSync(path.join(source, 'LICENSE')));
}
console.log(`Generated ${entries.length} searchable emoji from the locked Unicode and keyword packages.`);
