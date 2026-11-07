# Bundled emoji catalog

`emoji.json` is generated from the pinned `unicode-emoji-json` and `emojilib`
development dependencies. It contains Unicode emoji, English names, and search
keywords. Both the React picker and Rust reaction validator use this same file.
There are no runtime requests to emoji services.

Regenerate after an intentional dependency update with:

```sh
node scripts/generate-emoji-data.mjs
```

The packages' MIT licenses are included alongside the data. Unicode data terms:
https://www.unicode.org/license.html
