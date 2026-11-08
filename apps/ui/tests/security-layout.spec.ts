import {test,expect,type Page} from '@playwright/test';
const accountId='a'.repeat(64),fingerprint='1234abcd'.repeat(8);
async function fixture(page:Page,setup=false){
 await page.addInitScript(({setup,accountId,fingerprint})=>{
  const id='11111111-1111-4111-8111-111111111111';
  const binding={tenant_id:'22222222-2222-4222-8222-222222222222',device_id:id,signature_key:Array(32).fill(1)};
  const account={id:accountId,name:'Administrator',role:'admin',devices:[{binding,label:'Current desktop',approved_at:1,revoked:false}],requests:[{id:'33333333-3333-4333-8333-333333333333',label:'New browser installation',binding:{...binding,device_id:'44444444-4444-4444-8444-444444444444'},fingerprint,status:'pending',expires_at:Math.floor(Date.now()/1000)+600}]};
  const session={authenticated:!setup,auto_login:false,mode:'cognito',desktop:true,signed_in:true,profile:id,account,local_devices:[{alias:id,device_id:id,label:'Current device'}]};
  const snapshot={test_mode:false,current_profile:id,profiles:[{id,name:'Administrator',title:'This device',role:'admin',online:true,fingerprint}],conversations:[],online:true,attachment_limit:25*1024*1024};
  const calls:string[]=[];Object.assign(window,{securityCalls:calls});
  Object.assign(window,{__TAURI__:{core:{invoke:async(command:string,args?:{request?:{method:string;path:string;body:any}})=>{
   if(command!=='client_request')return true;
   const path=args?.request?.path||'';calls.push(path);
   let body:unknown={};
   if(path==='/local-auth/me')body=session;
   else if(path.startsWith('/local-api/state'))body=snapshot;
   else if(path.startsWith('/local-auth/devices/inspect'))body=account;
   else if(path==='/desktop/status')body={configured:true,enrolled:setup,approved:!setup,relay:'https://'+('long-business-name-'.repeat(6))+'.example.com',fingerprint,account_id:accountId};
   else if(path==='/desktop/contact')body={version:1,name:'Administrator',binding,package:'A'.repeat(6000)};
   else if(path==='/desktop/invitations'||path==='/desktop/group-decisions')body=[{version:1,package:'B'.repeat(6000)}];
   else if(path==='/local-auth/devices/decision')account.requests[0].status='approved';
   return{status:200,body};
  }}}});
 },{setup,accountId,fingerprint});
 await page.goto('/');
}
async function noOverflow(page:Page,selector:string){
 const size=await page.locator(selector).evaluate(element=>({width:element.scrollWidth,available:element.clientWidth}));expect(size.width).toBeLessThanOrEqual(size.available+1);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(await page.evaluate(()=>innerWidth));
}
for(const width of [1440,390])test(`security sections fit ${width}px and keep package drafts when switching tabs`,async({page})=>{
 await page.setViewportSize({width,height:width===1440?1000:844});await fixture(page);
 await page.getByRole('button',{name:'Devices',exact:true}).click();const dialog=page.getByRole('dialog',{name:'Devices & security'});
 await expect(dialog.getByRole('tab',{name:'Devices',exact:true})).toHaveAttribute('aria-selected','true');
 await expect(dialog.getByText('Current device',{exact:true})).toBeVisible();
 const bounds=await dialog.boundingBox();expect(bounds!.width).toBe(width===1440?760:width-24);
 await page.screenshot({path:`/tmp/cirra-security-devices-${width}.png`});
 await expect(dialog.getByLabel('Contact package',{exact:true})).toBeHidden();
 const request=dialog.locator('.device-enrollment');await expect(request.getByRole('button',{name:'Approve device',exact:true})).toBeDisabled();
 await request.getByLabel('Enter the fingerprint shown on the new device').fill('wrong');await expect(request.getByRole('button',{name:'Approve device',exact:true})).toBeDisabled();
 await noOverflow(page,'.security-body');
 await dialog.getByRole('tab',{name:'Contacts',exact:true}).click();
 await dialog.getByRole('button',{name:'Show my contact package'}).click();
 await expect(dialog.getByLabel('Public contact package',{exact:true})).toHaveValue(/AAAA/);
 await page.screenshot({path:`/tmp/cirra-security-contacts-${width}.png`});
 await dialog.getByLabel('Contact package',{exact:true}).fill('{"draft":"keep this"}');
 await noOverflow(page,'.security-body');
 await dialog.getByRole('tab',{name:'Contacts',exact:true}).focus();await page.keyboard.press('ArrowRight');
 await expect(dialog.getByRole('tab',{name:'Invitations',exact:true})).toBeFocused();await expect(dialog.getByLabel('Conversation invitation',{exact:true})).toBeVisible();
 await dialog.getByRole('tab',{name:'Contacts',exact:true}).click();await expect(dialog.getByLabel('Contact package',{exact:true})).toHaveValue('{"draft":"keep this"}');
 await dialog.getByRole('tab',{name:'Group changes',exact:true}).click();await expect(dialog.getByLabel('Group decision',{exact:true})).toBeVisible();
 await dialog.getByRole('tab',{name:'Devices',exact:true}).click();await request.getByLabel('Enter the fingerprint shown on the new device').fill(fingerprint);await expect(request.getByRole('button',{name:'Approve device',exact:true})).toBeEnabled();
 // The header and close action remain visible while the content scrolls.
 await page.locator('.security-body').evaluate(element=>{element.scrollTop=element.scrollHeight;});
 await expect(dialog.getByRole('button',{name:'Close dialog'})).toBeInViewport();await noOverflow(page,'.security-body');
 await page.screenshot({path:`/tmp/cirra-security-${width}.png`});
 await page.keyboard.press('Escape');await expect(dialog).toHaveCount(0);
});
for(const width of [320,390])test(`enrollment codes wrap and can be copied at ${width}px`,async({page,context})=>{
 await page.setViewportSize({width,height:844});await context.grantPermissions(['clipboard-read','clipboard-write']);await fixture(page,true);
 await expect(page.getByRole('heading',{name:'Sign in to Cirra'})).toBeVisible();
 await noOverflow(page,'.login-card');
 await page.getByRole('button',{name:'Copy account enrollment id'}).click();expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(accountId);
 await page.getByRole('button',{name:'Copy device fingerprint'}).click();expect((await page.evaluate(()=>navigator.clipboard.readText())).replace(/\s/g,'')).toBe(fingerprint);
 await page.screenshot({path:`/tmp/cirra-enrollment-${width}.png`});
});
