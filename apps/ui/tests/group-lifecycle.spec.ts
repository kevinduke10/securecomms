import { expect,test } from '@playwright/test';
test('owner explicitly transfers ownership and new owner removes a device',async({page})=>{
 await page.goto('/');
 const title=`Lifecycle ${Date.now()}`;
 await page.getByRole('button',{name:'New conversation',exact:true}).click();await page.getByLabel('Group name').fill(title);await page.getByRole('checkbox',{name:'Bob Chen'}).check();await page.getByRole('checkbox',{name:'Maya Patel'}).check();await page.getByRole('button',{name:'Start conversation',exact:true}).click();await expect(page.getByRole('heading',{name:title,exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Conversation details',exact:true}).click();await page.getByRole('button',{name:'Manage group members',exact:true}).click();
 const group=page.getByRole('dialog',{name:'Group members',exact:true});const lifecycle=group.locator('.group-lifecycle');
 await lifecycle.getByRole('combobox',{name:'Current member',exact:true}).selectOption('bob');
 const code=(await lifecycle.locator('code').textContent())!.replace(/\s/g,'');const transfer=lifecycle.getByRole('button',{name:'Transfer ownership',exact:true});await expect(transfer).toBeDisabled();await lifecycle.getByLabel('Verify the device fingerprint',{exact:true}).fill(code);await transfer.click();
 await expect(group.getByText('Conversation owner',{exact:false}).first()).toBeVisible();await expect(lifecycle).toHaveCount(0);
 await group.getByRole('button',{name:'Close dialog',exact:true}).click();await page.getByRole('button',{name:'Close details',exact:true}).click();
 await page.goto('/?as=bob');await page.getByRole('button',{name:`Open ${title}`,exact:true}).click();
 await page.getByRole('button',{name:'Conversation details',exact:true}).click();await page.getByRole('button',{name:'Manage group members',exact:true}).click();
 const ownerGroup=page.getByRole('dialog',{name:'Group members',exact:true});const ownerLifecycle=ownerGroup.locator('.group-lifecycle');
 await ownerLifecycle.getByRole('combobox',{name:'Action',exact:true}).selectOption('remove');await ownerLifecycle.getByRole('combobox',{name:'Current member',exact:true}).selectOption('maya');
 const removedFingerprint=(await ownerLifecycle.locator('code').textContent())!.replace(/\s/g,'');await ownerLifecycle.getByLabel('Verify the device fingerprint',{exact:true}).fill(removedFingerprint);await ownerLifecycle.getByRole('button',{name:'Remove device',exact:true}).click();await expect(ownerLifecycle.getByRole('status')).toContainText('Device removed.');
 await expect(ownerGroup.locator('.group-member-list')).not.toContainText('Maya Patel');
 await page.goto('/?as=maya');await expect(page.getByRole('heading',{name:'Launch team',exact:true})).toBeVisible();await expect(page.getByRole('button',{name:`Open ${title}`,exact:true})).toHaveCount(0);
});
