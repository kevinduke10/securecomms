import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import {browserEntry} from './vite.browser-entry.ts';

export default defineConfig({ plugins: [react(), ...(process.env.VITE_CIRRA_BROWSER==='true'?[browserEntry()]:[])], build: { target: 'es2022', outDir: process.env.VITE_CIRRA_BROWSER==='true'?'../../target/web-dist':'dist', emptyOutDir:true } });
