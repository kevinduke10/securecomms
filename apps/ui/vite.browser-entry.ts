import type {Plugin} from 'vite';
export const browserEntry=():Plugin=>({
 name:'cirra-browser-entry',
 transformIndexHtml:{order:'pre',handler(html){return html.replace('/src/main.tsx','/src/browser-main.tsx');}},
});
