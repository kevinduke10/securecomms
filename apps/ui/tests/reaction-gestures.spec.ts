import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';

const targetText = 'A quieter place for our next big idea.';
async function center(bubble: Locator) {
  await bubble.scrollIntoViewIfNeeded();
  const rect = (await bubble.boundingBox())!;
  return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
}
async function hold(page: Page, bubble: Locator) {
  const point = await center(bubble);
  await page.mouse.move(point.x,point.y);
  await page.mouse.down();
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toBeVisible();
  await page.mouse.up();
}

test('hold opens quick reactions; plus searches the full library and syncs a result', async ({ page, browser }) => {
  await page.goto('/');
  const message = page.getByRole('article').filter({hasText:targetText});
  const bubble = message.locator('.message-bubble');
  await expect(page.locator('.reaction-trigger')).toHaveCount(0);
  await bubble.click();
  await page.waitForTimeout(650); // A short click must not leave a delayed hold timer.
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toHaveCount(0);
  await hold(page,bubble);
  await expect(page.getByRole('button',{name:'React with 👍',exact:true})).toBeFocused();
  await page.getByRole('button',{name:'More emoji',exact:true}).click();
  const search = page.getByRole('searchbox',{name:'Search emoji',exact:true});
  await expect(search).toBeFocused();
  await search.fill('not-an-emoji-at-all-928475');
  await expect(page.getByText('No emoji found. Try a different word.',{exact:true})).toBeVisible();
  await search.fill('octopus');
  await page.getByRole('button',{name:'React with 🐙 — octopus',exact:true}).click();
  await expect(message.getByRole('button',{name:/Remove your 🐙 reaction \(1\)/})).toBeVisible();
  const bob = await browser.newPage();
  await bob.goto('http://127.0.0.1:3107/?as=bob');
  await expect(bob.getByRole('article').filter({hasText:targetText}).getByRole('button',{name:/Add 🐙 reaction \(1\)/})).toBeVisible();
  await page.reload();
  await expect(message.getByRole('button',{name:/Remove your 🐙 reaction \(1\)/})).toBeVisible();
  await message.getByRole('button',{name:/Remove your 🐙 reaction/}).click();
  await bob.close();
});

test('moving or cancelling a press does not react; keyboard opens the same picker', async ({ page }) => {
  await page.goto('/');
  const bubble = page.getByRole('article').filter({hasText:targetText}).locator('.message-bubble');
  const point = await center(bubble);
  await page.mouse.move(point.x,point.y); await page.mouse.down();
  await page.mouse.move(point.x + 35,point.y + 20); await page.mouse.up();
  await page.waitForTimeout(650);
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toHaveCount(0);
  await bubble.dispatchEvent('pointerdown',{pointerId:12,pointerType:'touch',isPrimary:true,button:0,clientX:point.x,clientY:point.y});
  await bubble.dispatchEvent('pointercancel',{pointerId:12,pointerType:'touch'});
  await page.waitForTimeout(650);
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toHaveCount(0);
  await bubble.focus(); await bubble.press('Enter');
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(bubble).toBeFocused();
  await bubble.press('Space');
  await page.getByRole('button',{name:'More emoji',exact:true}).click();
  await page.getByRole('searchbox',{name:'Search emoji'}).fill('rocket');
  await page.getByRole('searchbox',{name:'Search emoji'}).press('ArrowDown');
  await expect(page.locator('.emoji-choice').first()).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(bubble).toBeFocused();
});

test('real touch hold opens reactions and a swipe keeps scrolling without opening them', async ({ browser }) => {
  const context = await browser.newContext({hasTouch:true,isMobile:true,viewport:{width:390,height:844}});
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:3107/?as=maya');
  await page.getByRole('button',{name:'Open Launch team',exact:true}).click();
  const bubble = page.getByRole('article').filter({hasText:targetText}).locator('.message-bubble');
  const client = await context.newCDPSession(page);
  let point = await center(bubble);
  await client.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{...point,id:1}]});
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toBeVisible();
  await client.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await page.getByRole('button',{name:'More emoji',exact:true}).click();
  await page.getByRole('searchbox',{name:'Search emoji'}).fill('pizza');
  await page.getByRole('button',{name:'React with 🍕 — pizza',exact:true}).click();
  const reaction = page.getByRole('article').filter({hasText:targetText}).getByRole('button',{name:/Remove your 🍕 reaction/});
  await expect(reaction).toBeVisible(); await reaction.click();
  point = await center(bubble);
  await client.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{...point,id:2}]});
  await client.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:point.x,y:point.y-80,id:2}]});
  await client.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await page.waitForTimeout(650);
  await expect(page.getByRole('dialog',{name:'Choose a reaction'})).toHaveCount(0);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await context.close();
});
