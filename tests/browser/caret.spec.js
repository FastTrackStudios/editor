// @ts-check
// The caret on the web path. Insert mode uses the browser's own caret in the
// contenteditable, so the check is where the DOM selection sits. Normal mode
// paints the editor's block caret (or its empty-cell widget), so the check is
// that the painted element is there and has a box.
//
// The document is seeded through `?seed=`, the same query the web playground
// reads, so each case starts from a known text and caret.

const { test, expect } = require("@playwright/test");

const editor = (page) => page.locator("[data-editor-id]").first();

/** Open the playground seeded with `text`, focused, in normal mode. */
async function open(page, text) {
  await page.goto(`/?seed=${encodeURIComponent(text)}`);
  await editor(page).click();
  await page.keyboard.press("Escape");
}

/** The mirror of `EditorState` the debug panel shows. */
async function readHead(page) {
  return Number(await page.locator("#dbg-head").textContent());
}

test("normal mode paints a block caret over a character", async ({ page }) => {
  await open(page, "hello");
  const caret = page.locator(".ed-modal-caret-block").first();
  await expect(caret).toBeAttached();
  const box = await caret.boundingBox();
  expect(box, "the block caret has a box").not.toBeNull();
  expect(box.width).toBeGreaterThan(4);
  expect(box.height).toBeGreaterThan(8);
});

test("normal mode paints the empty-cell caret on a blank line", async ({ page }) => {
  await open(page, "hello\n\nworld");
  await page.keyboard.press("j"); // down onto the blank line
  const caret = page.locator(".ed-modal-caret-empty").first();
  await expect(caret).toBeAttached();
  const box = await caret.boundingBox();
  expect(box, "the empty-cell caret has a box").not.toBeNull();
  expect(box.width).toBeGreaterThan(2);
  expect(box.height).toBeGreaterThan(8);
});

test("insert mode puts the browser caret at the insertion point", async ({ page }) => {
  await open(page, "hello");
  await page.keyboard.press("i");
  await page.keyboard.type("X");
  expect(await readHead(page)).toBe(1);
});

test("append at end of document lands the caret after the last character", async ({ page }) => {
  await open(page, "hello");
  await page.keyboard.press("A");
  await page.keyboard.type("!");
  expect(await readHead(page)).toBe(6);
});
