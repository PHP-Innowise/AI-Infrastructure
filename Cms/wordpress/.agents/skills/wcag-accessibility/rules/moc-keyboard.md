---
title: Keyboard & Focus
type: moc
impact: CRITICAL
description: All functionality must be operable via keyboard alone. Many users cannot use a mouse and rely entirely on keyboard navigation.
---

# Keyboard & Focus

All interactive functionality must be keyboard-accessible. Users who cannot use a mouse depend entirely on keyboard navigation.

The most visible requirement is [[keyboard-focus-visible]] — every interactive element needs a visible focus indicator. Without it, keyboard users are navigating blind. Focus indicators should maintain [[color-contrast-ui]] contrast ratios (see [[moc-color]]).

[[keyboard-tab-order]] ensures focus moves in a logical sequence matching visual layout. Avoid positive `tabIndex` values — they break natural flow. All interactive elements must respond to keyboard events per [[keyboard-interactive-elements]], which connects to [[semantic-button-link]] for using the right HTML elements.

For modals and dialogs, [[keyboard-focus-trap]] implements focus trapping so keyboard users can't tab behind the overlay, while [[aria-expanded-states]] communicates the open/closed state. Every page needs a [[keyboard-skip-link]] to jump past repeated navigation blocks.
