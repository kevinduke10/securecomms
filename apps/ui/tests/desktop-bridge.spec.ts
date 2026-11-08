import {test,expect} from '@playwright/test';
for(const platform of ['desktop','ios']) test(`${platform} setup uses guarded commands and keeps login outside the webview`,async({page})=>{
 await page.addInitScript(platform=>{
  let configured=false;const calls:string[]=[];
  (window as unknown as {nativeCalls:string[]}).nativeCalls=calls;
  const invoke=async(command:string,args?:{request?:{method:string;path:string;body?:{relay?:string}}})=>{
   calls.push(command+':'+(args?.request?.path || ''));
   if(command==='open_login')return null;
   const path=args?.request?.path;
   if(path==='/desktop/configure'){configured=true;return{status:200,body:{}};}
   if(path==='/desktop/status')return{status:200,body:{configured,relay:'https://relay.example.com',enrolled:false,approved:false}};
   if(path==='/local-auth/me')return{status:200,body:{authenticated:false,auto_login:false,mode:'cognito',desktop:true,signed_in:false}};
   return{status:400,body:{error:'Unexpected bridge route'}};
  };
  if(platform==='ios')(window as unknown as {__CIRRA_IOS__:unknown}).__CIRRA_IOS__={invoke};
  else (window as unknown as {__TAURI__:unknown}).__TAURI__={core:{invoke}};
 },platform);
 await page.goto('/');await expect(page.getByRole('heading',{name:'Sign in to Cirra'})).toBeVisible();
 await page.getByLabel('Business relay URL').fill('https://relay.example.com');await page.getByRole('button',{name:'Save relay'}).click();
 await page.getByRole('button',{name:'Sign in with business account'}).click();
 await expect.poll(()=>page.evaluate(()=>(window as unknown as {nativeCalls:string[]}).nativeCalls)).toContain('open_login:');
 await expect(page.getByRole('link',{name:'Sign in'})).toHaveCount(0);
 expect(page.url()).toContain('127.0.0.1:3107');
});
