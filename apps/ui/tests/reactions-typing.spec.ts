import { expect, test } from '@playwright/test';

const targetText = 'A quieter place for our next big idea.';

test('reactions add, aggregate, replace, remove, persist, and queue offline', async ({ page, browser }) => {
  await page.goto('/');
  const aliceMessage = page.getByRole('article').filter({ hasText: targetText });
  await aliceMessage.locator('.message-bubble').click({ button: 'right' });
  await page.getByRole('button', { name: 'React with 👍', exact: true }).click();
  await expect(aliceMessage.getByRole('button', { name: /Remove your 👍 reaction \(1\)/ })).toBeVisible();
  const bob = await browser.newPage();
  await bob.goto('http://127.0.0.1:3107/?as=bob');
  const bobMessage = bob.getByRole('article').filter({ hasText: targetText });
  await bobMessage.getByRole('button', { name: /Add 👍 reaction \(1\)/ }).click();
  await expect(aliceMessage.getByRole('button', { name: /Remove your 👍 reaction \(2\)/ })).toBeVisible();
  await bobMessage.locator('.message-bubble').click({ button: 'right' });
  await bob.getByRole('button', { name: 'React with ❤️', exact: true }).click();
  await expect(aliceMessage.getByRole('button', { name: /Add ❤️ reaction \(1\)/ })).toBeVisible();
  await expect(aliceMessage.getByRole('button', { name: /Remove your 👍 reaction \(1\)/ })).toBeVisible();
  await aliceMessage.getByRole('button', { name: /Remove your 👍 reaction/ }).click();
  await expect(bobMessage.getByRole('button', { name: /👍 reaction/ })).toHaveCount(0);
  await page.reload();
  await expect(aliceMessage.getByRole('button', { name: /Add ❤️ reaction \(1\)/ })).toBeVisible();
  await bob.getByRole('button', { name: 'Choose test account', exact: true }).click();
  await bob.getByRole('button', { name: /Go offline/ }).click();
  await bob.getByRole('button', { name: 'Close account menu', exact: true }).click();
  await bobMessage.locator('.message-bubble').click({ button: 'right' });
  await bob.getByRole('button', { name: 'React with 🎉', exact: true }).click();
  await expect(bobMessage.locator('.reaction-chip.queued')).toContainText('🎉');
  await expect(aliceMessage.getByRole('button', { name: /🎉 reaction/ })).toHaveCount(0);
  await bob.getByRole('button', { name: 'Choose test account', exact: true }).click();
  await bob.getByRole('button', { name: /Reconnect/ }).click();
  await bob.getByRole('button', { name: 'Close account menu', exact: true }).click();
  await expect(aliceMessage.getByRole('button', { name: /Add 🎉 reaction \(1\)/ })).toBeVisible();
  await expect(bobMessage.locator('.reaction-chip.queued')).toHaveCount(0);
  await bobMessage.getByRole('button', { name: /Remove your 🎉 reaction/ }).click();
  await expect(aliceMessage.locator('.reaction-chip')).toHaveCount(0);
  await bob.close();
});

test('typing indicators start, stop, expire, and never expose unsent text', async ({ page, browser }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Launch team', exact: true })).toBeVisible();
  const before = await page.getByRole('article').count();
  const bob = await browser.newPage();
  await bob.goto('http://127.0.0.1:3107/?as=bob');
  const editor = bob.getByRole('textbox', { name: 'Message', exact: true });
  await editor.fill('An unsent thought, only on Bob’s device.');
  await expect(page.locator('.typing-indicator')).toHaveText('Bob is typing…');
  await expect(page.getByText('An unsent thought, only on Bob’s device.', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('article')).toHaveCount(before);
  await editor.fill('');
  await expect(page.locator('.typing-indicator')).toBeEmpty();
  await editor.fill('A completed thought from Bob.');
  await expect(page.locator('.typing-indicator')).toHaveText('Bob is typing…');
  await editor.press('Enter');
  await expect(page.locator('.message-bubble').filter({ hasText: 'A completed thought from Bob.' })).toBeVisible();
  await expect(page.locator('.typing-indicator')).toBeEmpty();
  // Simulate a tab disappearing before its stop signal reaches the server.
  await bob.route('**/local-api/bob/typing', async route => {
    if (route.request().postDataJSON().active === false) await route.abort();
    else await route.continue();
  });
  await editor.fill('This draft is never sent.');
  await expect(page.locator('.typing-indicator')).toHaveText('Bob is typing…');
  await expect(page.locator('.typing-indicator')).toBeEmpty({ timeout: 10_000 });
  await expect(page.getByRole('article')).toHaveCount(before + 1);
  await bob.close();
});

test('reaction picker supports desktop context menus and dismisses with Escape', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?as=maya');
  await page.getByRole('button', { name: 'Open Launch team', exact: true }).click();
  const message = page.getByRole('article').filter({ hasText: targetText });
  await message.locator('.message-bubble').click({ button: 'right' });
  await expect(page.getByRole('dialog', { name: 'Choose a reaction' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'React with 👍', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Choose a reaction' })).toHaveCount(0);
  await message.locator('.message-bubble').click({ button: 'right' });
  await page.getByRole('button', { name: 'React with 😂', exact: true }).click();
  await expect(message.getByRole('button', { name: /Remove your 😂 reaction/ })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
