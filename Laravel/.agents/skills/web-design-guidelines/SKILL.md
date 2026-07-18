---
name: web-design-guidelines
description: Review UI code for general Web Interface Guidelines compliance (interaction, layout, visual, UX polish). Use when asked to "review my UI", "audit design", "review UX", or "check my site against best practices". For accessibility/WCAG/a11y specifically, use wcag-accessibility instead.
metadata:
  author: vercel
  version: "1.0.0"
  argument-hint: <file-or-pattern>
---

# Web Interface Guidelines

Review files for compliance with Web Interface Guidelines.

**Supplementary, not required:** this is a periodic utility that fetches an external, framework-agnostic UX checklist — it is not a required step in the main Laravel skill flow and carries no embedded rules of its own. For day-to-day Laravel frontend work (Blade/Livewire/Inertia forms, loading/empty/error states, general UI polish), `frontend-design`'s own "Frontend Best Practices" section is the primary, self-sufficient source. Reach for this skill occasionally to spot-check against the latest external consensus, not on every frontend task.

## Scope Boundary

This skill covers **general UX and interface quality** (interaction, layout, visual polish, content). For **accessibility-specific** review (WCAG 2.2, screen readers, keyboard, ARIA, contrast, focus), use `wcag-accessibility`, which contains the dedicated 30-rule ruleset. The two are complementary: run `wcag-accessibility` for a11y depth and this skill for broader UX.

## How It Works

1. Fetch the latest guidelines from the source URL below
2. Read the specified files (or prompt user for files/pattern)
3. Check against all rules in the fetched guidelines
4. Output findings in the terse `file:line` format

## Guidelines Source

Fetch fresh guidelines before each review:

```
https://raw.githubusercontent.com/vercel-labs/web-interface-guidelines/main/command.md
```

Use WebFetch to retrieve the latest rules. The fetched content contains all the rules and output format instructions.

## Usage

When a user provides a file or pattern argument:
1. Fetch guidelines from the source URL above
2. Read the specified files
3. Apply all rules from the fetched guidelines
4. Output findings using the format specified in the guidelines

If no files specified, ask the user which files to review.
