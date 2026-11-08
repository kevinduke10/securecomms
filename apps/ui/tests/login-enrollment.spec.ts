import { expect,test } from '@playwright/test';
test('sign-in gates history, scopes accounts, and logout removes the workspace',async({page})=>{
  await page.goto('/?as=alice');await expect(page.getByRole('heading',{name:'Sign in to Cirra'})).toBeVisible();
  expect((await page.request.get('/local-api/state?profile=alice',{headers:{'x-croaker-local':'1'}})).status()).toBe(401);
  await page.getByRole('button',{name:'Sign in as Bob Chen',exact:true}).click();await expect(page.getByRole('heading',{name:'Launch team',exact:true})).toBeVisible();
  expect((await page.request.get('/local-api/state?profile=alice',{headers:{'x-croaker-local':'1'}})).status()).toBe(403);
  await page.reload();await expect(page.getByRole('heading',{name:'Launch team',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Choose test account'}).click();
  await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Sign in to Cirra'})).toBeVisible();
  await expect(page.getByRole('article')).toHaveCount(0);
});
test('a replacement needs fingerprint approval and opens with fresh keys and no old chats',async({page,context})=>{
  await page.goto('/');await page.getByRole('button',{name:'Sign in as Alice Morgan',exact:true}).click();
  await page.getByRole('button',{name:'Devices',exact:true}).click();const dialog=page.getByRole('dialog',{name:'Devices & security',exact:true});
  await dialog.getByLabel('New device name',{exact:true}).fill('Replacement test laptop');await dialog.getByRole('button',{name:'Request replacement device',exact:true}).click();
  const enrollment=dialog.locator('.device-enrollment').filter({hasText:'Replacement test laptop'});await expect(enrollment).toBeVisible();
  const fingerprint=(await enrollment.locator('.device-fingerprint').textContent())!.replace(/\s/g,'');
  await expect(enrollment.getByRole('button',{name:'Approve device',exact:true})).toBeDisabled();
  await enrollment.getByLabel('Enter the fingerprint shown on the new device').fill('wrong');await expect(enrollment.getByRole('button',{name:'Approve device',exact:true})).toBeDisabled();
  await enrollment.getByLabel('Enter the fingerprint shown on the new device').fill(fingerprint);await enrollment.getByRole('button',{name:'Approve device',exact:true}).click();
  await enrollment.getByRole('button',{name:'Use this new device',exact:true}).click();await expect(page.getByRole('heading',{name:'Device enrolled',exact:true})).toBeVisible();
  await expect(page.getByRole('article')).toHaveCount(0);await page.reload();await expect(page.getByRole('article')).toHaveCount(0);
  await page.getByRole('button',{name:'Devices',exact:true}).click();await expect(dialog.getByText('Current device',{exact:true})).toBeVisible();
  // Another account cannot choose Alice's new local client, even with its alias.
  const session=await (await page.request.get('/local-auth/me')).json();const other=await context.browser()!.newContext();const maya=await other.newPage();
  await maya.goto('http://127.0.0.1:3108');await maya.getByRole('button',{name:'Sign in as Maya Patel',exact:true}).click();
  const denied=await maya.request.get(`/local-api/state?profile=${session.profile}`,{headers:{'x-croaker-local':'1'}});expect(denied.status()).toBe(403);await other.close();
});
test('mobile login and rejected enrollment remain usable',async({page})=>{
  await page.setViewportSize({width:390,height:844});await page.goto('/');await page.getByRole('button',{name:'Sign in as Maya Patel',exact:true}).click();
  await page.getByRole('button',{name:'Devices',exact:true}).click();const dialog=page.getByRole('dialog',{name:'Devices & security',exact:true});
  await dialog.getByLabel('New device name').fill('Rejected guest device');await dialog.getByRole('button',{name:'Request replacement device',exact:true}).click();
  const request=dialog.locator('.device-enrollment').filter({hasText:'Rejected guest device'});await request.getByRole('button',{name:'Reject',exact:true}).click();await expect(request.getByText('rejected',{exact:true})).toBeVisible();
  await expect(request.getByRole('button',{name:'Use this new device',exact:true})).toHaveCount(0);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});
