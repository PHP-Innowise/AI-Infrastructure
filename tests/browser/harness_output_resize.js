// Run with Playwright browser_run_code_unsafe({filename: this file's absolute path}).
// Uses an existing local Harness session; never submits a prompt or starts a model.
async (page) => {
  const check = (condition,message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({width:1440,height:1000});
  await page.goto('http://127.0.0.1:8766/');
  const handle = page.getByRole('separator',{name:'AI output height'});
  await handle.waitFor();
  await page.locator('.history-item').first().click();
  await page.locator('#events .message').first().waitFor();
  const messages = await page.locator('#events').textContent();
  const original = await page.locator('#conversation').boundingBox();
  const bar = await handle.boundingBox();
  await page.mouse.move(bar.x + bar.width / 2,bar.y + bar.height / 2);
  await page.mouse.down();
  await page.mouse.move(bar.x + bar.width / 2,bar.y - 160,{steps:12});
  await page.mouse.up();
  const expanded = await page.locator('#conversation').boundingBox();
  check(expanded.height > original.height + 100,'Dragging up must expand the AI output');
  await handle.press('ArrowDown');
  const shrunk = await page.locator('#conversation').boundingBox();
  check(shrunk.height < expanded.height - 30,'ArrowDown must reduce output height');
  await handle.press('ArrowUp');
  check((await page.locator('#conversation').boundingBox()).height > shrunk.height + 30,'ArrowUp must expand output');
  await handle.press('Home');
  check((await page.locator('#conversation').boundingBox()).height > expanded.height,'Home must maximize output');
  await handle.press('End');
  check((await page.locator('#conversation').boundingBox()).height >= 99,'Output must remain visible at the minimum');
  check(await page.locator('#events').textContent() === messages,'Resizing must preserve messages');
  await handle.dblclick();
  check(!await page.locator('#sessions-view').evaluate(e => e.classList.contains('output-resized')),'Double-click must restore automatic layout');
  await page.setViewportSize({width:390,height:844});
  await handle.press('Home');
  await page.locator('#prompt').fill('Unsent resize check');
  await handle.press('End');
  await page.setViewportSize({width:390,height:600});
  const mobile = await page.evaluate(() => {
    const box = document.getElementById('composer-area').getBoundingClientRect();
    return {composerBottom:box.bottom,height:innerHeight,width:document.documentElement.scrollWidth,viewport:innerWidth};
  });
  check(mobile.composerBottom <= mobile.height + 1,'The composer must remain accessible on a short screen');
  check(mobile.width <= mobile.viewport,'Resizing must not introduce horizontal overflow');
  check(await page.locator('#prompt').inputValue() === 'Unsent resize check','Resizing must preserve the draft');
  await page.locator('#prompt').fill('');
  await handle.dblclick();
  await page.setViewportSize({width:1440,height:1000});
  return {ok:true,checks:['mouse drag','keyboard','limits','reset','message and draft preservation','mobile and short viewport']};
}
