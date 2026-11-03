// ABI loader only. Application state, requests and rendering are implemented by the compiled module.
import init from './assets/infra_ui.js';
try {
  await init({ module_or_path: new URL('./assets/infra_ui_bg.wasm', import.meta.url) });
} catch (error) {
  console.error('Application startup failed.', error);
  let node = document.getElementById('connection-message');
  if (!node) {
    node = document.createElement('p');
    node.className = 'error-text';
    node.setAttribute('role', 'alert');
    (document.querySelector('main') || document.body).append(node);
  }
  node.textContent = 'The application could not start. Restart the local application and reload this page.';
  const banner = document.getElementById('connection-banner');
  if (banner) banner.hidden = false;
}
