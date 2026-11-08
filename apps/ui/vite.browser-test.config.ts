import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';
import {browserEntry} from './vite.browser-entry.ts';
export default defineConfig({plugins:[react(),browserEntry()],server:{host:'127.0.0.1',port:3111,strictPort:true,proxy:{'/v1':'http://127.0.0.1:3112','/test':'http://127.0.0.1:3112'}}});
