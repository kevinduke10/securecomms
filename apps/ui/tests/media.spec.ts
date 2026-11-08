import {expect,test} from '@playwright/test';
import {readFile} from 'node:fs/promises';
test('large ciphertext attachment survives refresh and downloads unchanged without descriptor keys',async({page})=>{
 await page.goto('/');await expect(page.getByRole('heading',{name:'Launch team',exact:true})).toBeVisible();
 const name=`large-${Date.now()}.html`;const bytes=Buffer.from('<script>throw new Error("never run attached HTML")</script>\n'.repeat(2048));
 await page.getByLabel('Choose an attachment').setInputFiles({name,mimeType:'text/html',buffer:bytes});await page.getByRole('button',{name:'Send message',exact:true}).click();
 await expect(page.getByRole('button',{name:`Download ${name}`,exact:true})).toBeVisible();await expect(page.getByRole('button',{name:`Preview ${name}`,exact:true})).toHaveCount(0);
 await page.reload();await expect(page.getByRole('button',{name:`Download ${name}`,exact:true})).toBeVisible();
 const pending=page.waitForEvent('download');await page.getByRole('button',{name:`Download ${name}`,exact:true}).click();const download=await pending;expect(await readFile((await download.path())!)).toEqual(bytes);
 const state=await page.evaluate(async()=>fetch('/local-api/state?profile=alice',{headers:{'x-croaker-local':'1'}}).then(r=>r.json()));const media=state.conversations.flatMap((c:{messages:unknown[]})=>c.messages).find((m:{media?:{name:string}})=>m.media?.name===name).media;
 expect(Object.keys(media).sort()).toEqual(['mime','name','object_id','size']);
});
