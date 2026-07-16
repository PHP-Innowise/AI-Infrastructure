# Agent-Based Skill Execution

This directory contains agent definitions that execute skills in isolation and suggest next steps.

## Architecture: Command -> Agent -> Skill

```
User runs: /requirements-analyst [prompt]
              │
              ▼
    ┌─────────────────────┐
    │   Command Loaded    │  (commands/requirements-analyst.md)
    │   Spawns Agent      │
    └─────────────────────┘
              │
              ▼
    ┌─────────────────────┐
    │   Agent Executes    │  (agents/requirements-analyst-agent.md)
    │   Uses Skill Tool   │
    └─────────────────────┘
              │
              ▼
    ┌─────────────────────┐
    │   Skill Invoked     │  (skills/requirements-analyst/SKILL.md)
    │   Work Performed    │
    └─────────────────────┘
              │
              ▼
    ┌─────────────────────┐
    │   Output with       │
    │   Next Step         │
    │   Suggestions       │
    └─────────────────────┘
```

## Key Principles

### 1. Isolation
Each agent runs in its own context to prevent main conversation pollution and hallucination.

### 2. Stop After Completion
Agents complete their task and STOP. They do NOT automatically chain to next skills.

### 3. Context Handoff
Agents provide a short context summary that can be passed to the next command.

### 4. Next Step Suggestions
Every agent output ends with suggested next commands based on:
- The flow defined in `skills/SKILL FLOW.md`
- Alternative relevant commands

### 5. Time Budget Awareness
If you've made >3 tool calls without measurable progress toward the goal, pause and reassess your approach. Consider:
- Is the current strategy working?
- Should you try an alternative approach?
- Should you escalate to `/debugger`?

## Agent Template

Every agent follows this structure:

```markdown
# [Skill Name] Agent

## Role
[Single sentence describing the agent's purpose]

## Instructions

1. Use the Skill tool to invoke `[skill-name]` skill
2. Execute the skill completely following its instructions
3. STOP when the skill work is done
4. Provide structured output (see below)

## Output Format

When done, provide:

### Context Summary
[2-3 sentences summarizing what was accomplished and key decisions/artifacts]

### Next Steps

**Next by flow:** `/[next-command]` - [one sentence why]

**Alternatives:**
- `/[alt-command-1]` - [one sentence why]
- `/[alt-command-2]` - [one sentence why]

## Constraints
- ONLY execute the [skill-name] skill
- DO NOT chain to other skills automatically
- DO NOT make workflow decisions
- STOP after skill completion and output suggestions
```

## Agent List

### Understanding Phase
| Agent | Skill | Next by Flow |
|-------|-------|--------------|
| `requirements-analyst-agent` | requirements-analyst | brainstorm |
| `researcher-agent` | researcher | council |
| `brainstorming-agent` | brainstorming | architect |

### Planning Phase
| Agent | Skill | Next by Flow |
|-------|-------|--------------|
| `council-agent` | council | architect |
| `architect-agent` | architect | api-designer |
| `database-designer-agent` | database-designer | architecture-implementer |
| `api-designer-agent` | api-designer | frontend-design |
| `frontend-design-agent` | frontend-design | writing-plans |
| `writing-plans-agent` | writing-plans | git-worktrees |

### Implementation Phase
| Agent | Skill | Next by Flow |
|-------|-------|--------------|
| `using-git-worktrees-agent` | using-git-worktrees | coder / coder-frontend |
| `architecture-implementer-agent` | architecture-implementer | coder |
| `coder-agent` | coder | code-reviewer |
| `coder-frontend-agent` | coder-frontend | code-reviewer |
| `refactorer-agent` | refactorer | verify |

### Quality Phase
| Agent | Skill | Next by Flow |
|-------|-------|--------------|
| `review-pr-agent` | review-pr | debugger |
| `code-reviewer-agent` | code-reviewer | test-generator |
| `security-reviewer-agent` | security-reviewer | verify |
| `test-generator-agent` | test-generator | debugger (if failures) / finishing-branch |
| `systematic-debugger-agent` | systematic-debugger | test-generator |
| `performance-optimization-agent` | performance-optimization | verify |
| `verify-agent` | verify | finishing-branch |
| `finishing-branch-agent` | finishing-branch | (end) |

### Finalization Phase
| Agent | Skill | Next by Flow |
|-------|-------|--------------|
| `documentation-generator-agent` | documentation-generator | release / finishing-branch |
| `release-agent` | release | finishing-branch |

### Utility
| Agent | Skill | Next by Flow |
|-------|-------|--------------|
| `memory-bank-agent` | memory-bank | (standalone) |
| `reflect-agent` | reflect | (standalone) |
| `skill-creator-agent` | skill-creator | (standalone) |
| `dependency-manager-agent` | dependency-manager | verify |

## Flow Reference

See `skills/SKILL FLOW.md` for the complete visual flow.

## Context Splitting

Using agents splits context because:
1. Each agent runs as a subagent with fresh context
2. Only the context summary is returned to main conversation
3. Main agent stays clean without skill execution details
4. Prevents hallucination from context overload
