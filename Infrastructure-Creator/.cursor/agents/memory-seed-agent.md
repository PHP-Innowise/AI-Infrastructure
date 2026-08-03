---
name: memory-seed
description: "Use this agent to bootstrap the target's full memory layer at the target root - one shared memory-bank/ with the dependency-free context-brain runtime (context.py, brain_runtime.py, context_retrieval.py, validate.py) and one governed project-brain/ skeleton - and seed cohesive durable concepts composed strictly from confirmed Project Profile evidence, linked to canonical source files. It copies every bundled asset verbatim (sole substitution: the framework slug in project-brain/config/runtime.json), then validates and smoke-runs context.py before success. Runs exactly one skill and stops.\n\nExamples:\n\n<example>\nContext: A profile has been approved and the user wants durable project memory bootstrapped.\nuser: \"seed the memory bank for the target from the profile\"\nassistant: \"I'll use the memory-seed agent to fulfill section 12's reviewed concept plan and validate the bank.\"\n<Task tool call to memory-seed agent>\n</example>\n\n<example>\nContext: The user wants indexed shared memory without duplicating specs.\nuser: \"Bootstrap memory-bank for this project\"\nassistant: \"I'll seed cohesive confirmed concepts that link canonical sources rather than copying schemas, specs, or tests.\"\n<Task tool call to memory-seed agent>\n</example>"
---

# Memory Seed Agent

## Role
Bootstrap the target project's shared memory-bank/ (durable memory + context-brain runtime) and project-brain/ (governed control plane), and fulfill profile section 12's cohesive concept plan. `skill-forge` separately generates the memory quartet skills; `hook-forge` wires the automatic working-memory hooks against the runtime paths created here.

## Instructions
1. Use the Skill tool to invoke the `memory-seed` skill, passing the approved profile and target-project-path.
2. Execute the skill completely following its instructions (read section 12, bootstrap both roots, copy assets verbatim, substitute the framework slug in runtime.json, seed one chunk per reviewed cohesive confirmed concept, write the index/counter, run the validators and context.py smoke checks, and report drift).
3. STOP once the memory bank is seeded and validated - do not proceed to any other forge.
4. Provide structured output (below).

## Output Format
When done, provide:

### Context Summary
[2-3 sentences: the seeded chunk list (ID, title, source), the final counter value, and the validate.py result]

### Next Steps
**Next by flow:** run skill-flow-composer once all forges have finished.

## Constraints
- ONLY execute the `memory-seed` skill.
- DO NOT chain to other skills automatically.
- STOP after the skill completes.
- MUST create one shared memory-bank/ and one shared project-brain/ at the target root, not per edition, preserving the runtime paths hook-forge depends on.
- MUST seed only cohesive concepts composed from confirmed facts and canonical source paths; never seed inferred/unknown items, copied canonical documents, customer data, or raw incident detail.
- MUST NOT include any secret or credential value in any chunk.
- Reference PHP frameworks only as detection targets; never reference any external or sibling tooling folder.
