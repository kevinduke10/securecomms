import { expect,test } from '@playwright/test';

test('an owner admits an approved replacement; it receives new messages without old history',async({browser})=>{
  const aliceContext=await browser.newContext();const bobContext=await browser.newContext();
  try {
    const alice=await aliceContext.newPage();const bob=await bobContext.newPage();
    await alice.goto('http://127.0.0.1:3108');await alice.getByRole('button',{name:'Sign in as Alice Morgan',exact:true}).click();
    await bob.goto('http://127.0.0.1:3108');await bob.getByRole('button',{name:'Sign in as Bob Chen',exact:true}).click();
    await alice.getByRole('textbox',{name:'Message',exact:true}).fill('History before this replacement joined');await alice.getByRole('button',{name:'Send message',exact:true}).click();
    await expect(alice.getByRole('article').filter({hasText:'History before this replacement joined'})).toBeVisible();
    await bob.getByRole('button',{name:'Devices',exact:true}).click();
    const devices=bob.getByRole('dialog',{name:'Devices & security',exact:true});
    await devices.getByLabel('New device name').fill('Bob group browser replacement');await devices.getByRole('button',{name:'Request replacement device',exact:true}).click();
    const pending=devices.locator('.device-enrollment').filter({hasText:'Bob group browser replacement'});
    const identity=(await pending.locator('.device-fingerprint').textContent())!.replace(/\s/g,'');
    await pending.getByLabel('Enter the fingerprint shown on the new device').fill(identity);await pending.getByRole('button',{name:'Approve device',exact:true}).click();
    await pending.getByRole('button',{name:'Use this new device',exact:true}).click();await expect(bob.getByRole('heading',{name:'Device enrolled',exact:true})).toBeVisible();
    await alice.getByRole('button',{name:'Conversation details',exact:true}).click();await alice.getByRole('button',{name:'Manage group members',exact:true}).click();
    const group=alice.getByRole('dialog',{name:'Group members',exact:true});
    await group.getByRole('combobox',{name:'Device',exact:true}).selectOption({label:'Bob Chen — Bob group browser replacement'});
    const confirm=group.getByRole('button',{name:'Add device to group',exact:true});await expect(confirm).toBeDisabled();
    await group.getByLabel('Enter the fingerprint shown on the device',{exact:true}).fill(identity);await confirm.click();
    await expect(group.getByRole('status')).toContainText('Device added.');
    await group.getByRole('button',{name:'Close dialog',exact:true}).click();
    await alice.getByRole('button',{name:'Close details',exact:true}).click();
    await expect(bob.getByRole('heading',{name:'Launch team',exact:true})).toBeVisible();
    await expect(bob.getByRole('article').filter({hasText:'History before this replacement joined'})).toHaveCount(0);
    await alice.getByRole('textbox',{name:'Message',exact:true}).fill('Message after replacement group admission');await alice.getByRole('button',{name:'Send message',exact:true}).click();
    await expect(bob.getByRole('article').filter({hasText:'Message after replacement group admission'})).toBeVisible();
    await bob.reload();await expect(bob.getByRole('article').filter({hasText:'Message after replacement group admission'})).toBeVisible();
    await expect(bob.getByRole('article').filter({hasText:'History before this replacement joined'})).toHaveCount(0);
  }finally{await Promise.allSettled([aliceContext.close(),bobContext.close()]);}
});
