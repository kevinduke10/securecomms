import { useEffect, useState } from 'react';

export const themes = [
  { id: 'forest', name: 'Forest', description: 'Green' },
  { id: 'ocean', name: 'Ocean', description: 'Blue' },
  { id: 'rose', name: 'Rose', description: 'Pink' },
  { id: 'sand', name: 'Sand', description: 'Tan' },
  { id: 'midnight', name: 'Midnight', description: 'Dark blue' },
] as const;
export type Theme = typeof themes[number]['id'];
const storageKey = 'croaker:theme';
function validate(value: string | null): Theme {
  return themes.find(theme => theme.id === value)?.id ?? 'forest';
}
function readTheme(): Theme {
  try { return validate(localStorage.getItem(storageKey)); } catch { return 'forest'; }
}
function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', theme === 'midnight' ? '#101827' : { forest: '#173f35', ocean: '#153e59', rose: '#542a40', sand: '#4e3a28' }[theme]);
}
// Run before mounting React, including when storage is unavailable.
export function initializeTheme() { applyTheme(readTheme()); }
export function useTheme() {
  const [theme, setTheme] = useState<Theme>(readTheme);
  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.storageArea === localStorage && (event.key === storageKey || event.key === null)) {
        const next = readTheme(); applyTheme(next); setTheme(next);
      }
    };
    window.addEventListener('storage', sync);
    return () => window.removeEventListener('storage', sync);
  }, []);
  function chooseTheme(next: Theme) {
    applyTheme(next); setTheme(next);
    try { localStorage.setItem(storageKey, next); } catch { /* Keep the in-memory selection when storage is blocked. */ }
  }
  return [theme, chooseTheme] as const;
}
