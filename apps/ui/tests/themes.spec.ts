import { expect, test } from '@playwright/test';

test('themes preview live, persist, and synchronize across account tabs', async ({ page, context }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Choose theme', exact: true }).click();
  const appearance = page.getByRole('dialog', { name: 'Appearance', exact: true });
  for (const name of ['Ocean', 'Rose', 'Sand', 'Midnight', 'Forest']) {
    await appearance.getByRole('radio', { name: new RegExp(name) }).check();
    await expect(page.locator('html')).toHaveAttribute('data-theme', name.toLowerCase());
  }
  await appearance.getByRole('radio', { name: /Midnight/ }).check();
  await appearance.getByRole('button', { name: 'Done', exact: true }).click();
  await expect(page.locator('.chat-panel')).toHaveCSS('background-color', 'rgb(20, 30, 46)');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'midnight');
  const bob = await context.newPage();
  await bob.goto('/?as=bob');
  await expect(bob.locator('html')).toHaveAttribute('data-theme', 'midnight');
  await bob.getByRole('button', { name: 'Choose theme', exact: true }).click();
  await bob.getByRole('radio', { name: /Rose/ }).check();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'rose');
  await bob.evaluate(() => localStorage.clear());
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'forest');
  await bob.close();
});

test('mobile theme picker is keyboard accessible and covers the reaction portal', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Choose theme', exact: true }).click();
  const ocean = page.getByRole('radio', { name: /Ocean/ });
  await ocean.focus();
  await page.keyboard.press('Space');
  await expect(ocean).toBeChecked();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('radio', { name: /Rose/ })).toBeChecked();
  await page.getByRole('radio', { name: /Midnight/ }).check();
  await page.getByRole('button', { name: 'Done', exact: true }).click();
  await page.getByRole('button', { name: /Launch team/ }).click();
  const bubble = page.getByRole('article').filter({ hasText: 'A quieter place for our next big idea.' }).locator('.message-bubble');
  await bubble.focus();
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: 'More emoji', exact: true }).click();
  await expect(page.locator('.reaction-picker')).toHaveCSS('background-color', 'rgb(30, 43, 64)');
  await page.getByRole('searchbox', { name: 'Search emoji', exact: true }).fill('octopus');
  await expect(page.getByRole('button', { name: 'React with 🐙 — octopus', exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('invalid or unavailable stored preferences fall back to Forest', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('croaker:theme', 'unknown-theme'));
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'forest');
  await page.addInitScript(() => {
    Storage.prototype.getItem = () => { throw new Error('Storage blocked'); };
    Storage.prototype.setItem = () => { throw new Error('Storage blocked'); };
  });
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'forest');
  await page.getByRole('button', { name: 'Choose theme', exact: true }).click();
  await page.getByRole('radio', { name: /Sand/ }).check();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'sand');
});
