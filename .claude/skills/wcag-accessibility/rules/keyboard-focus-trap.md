---
title: Modal Dialogs Must Trap Focus
impact: HIGH
impactDescription: keyboard users can get lost behind modals
tags: keyboard, focus, modal
related: [keyboard-tab-order, keyboard-focus-visible]
---

## Modal Dialogs Must Trap Focus

**Impact: HIGH**

When a modal is open, keyboard focus must stay inside it until the modal is closed.

**Incorrect: modal without dialog semantics or focus handling**

```html
<div class="overlay">
  <div class="modal"><button type="button">Close</button></div>
</div>
```

**Correct: semantic dialog with focusable controls**

```html
<dialog aria-labelledby="dialog-title">
  <h2 id="dialog-title">Confirm action</h2>
  <p>This action cannot be undone.</p>
  <button type="button">Cancel</button>
  <button type="button">Confirm</button>
</dialog>
```

Use native `<dialog>` where possible, or a tested accessible dialog implementation. In Livewire/Alpine/Inertia, verify Tab, Shift+Tab, Escape, and focus restoration.
