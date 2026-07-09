---
title: Don't Use Color as the Only Indicator
impact: HIGH
impactDescription: color-blind users cannot perceive color-only cues
tags: color, indicator, color-blind
related: [forms-error-messages, color-contrast-text]
---

## Don't Use Color as the Only Indicator

**Impact: HIGH (color-blind users cannot perceive color-only cues)**

Information conveyed by color must also be available through text, icons, or patterns. WCAG 1.4.1.

**Incorrect: color-only status**

```html
function Status({ isOnline }) {
  return <span style={{ color: isOnline ? 'green' : 'red' }}>●</span>
}
```

**Correct: color plus text**

```html
function Status({ isOnline }) {
  return (
    <span>
      <span style={{ color: isOnline ? 'green' : 'red' }} aria-hidden="true">●</span>
      {' '}{isOnline ? 'Online' : 'Offline'}
    </span>
  )
}
```

Common patterns: status badges use icons + text, chart lines use patterns + colors, links use underline + color, errors use icon + text + border.

This is especially critical for [[forms-error-messages]] — don't rely only on red text for errors. Ensure any color used also meets [[color-contrast-text]] requirements.
