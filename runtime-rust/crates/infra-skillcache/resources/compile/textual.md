# Textual form

Use this form when the request for it says the hybrid form did not qualify.
Nothing is executed for a textual capsule: the agent reads the procedure
instead of the skill, so it must be shorter than the skill and complete for
this kind of request.

Return `{"form": "textual", "capsule": {"mu": …, "source": …}, "markdown": "…"}`
where `markdown` follows this shape:

```markdown
# <What the procedure does>

When to use: <the kind of request it covers, in one sentence>.

## Inputs

- `<parameter>` — <what it is and where the request states it>

## Steps

1. <First step, as an imperative sentence, with the exact command or tool.>
2. <…>

## Verify

1. <A check the request or the skill requires, or that the observed run did.>

## If something differs

Read `<skill path>`, section "<section>", for <what it covers>.
```

- Apply the same four rules: only the steps the result depended on, parameters
  instead of this run's values, consecutive mechanical steps merged into one
  step, and only grounded checks.
- At most 24 KiB. One `#` title, the steps as a numbered list, every fenced
  block closed.
- No observed value, no secret, no file content.
