---
name: performance-optimization
description: "Spawn performance-optimization agent to baseline, profile, fix the top hotspots (N+1/PDO, caching, memory, OPcache/JIT), and re-measure a native PHP performance problem."
---

# Performance Optimization

Spawn performance-optimization agent to baseline, profile, fix the top hotspots (N+1/PDO, caching, memory, OPcache/JIT), and re-measure a native PHP performance problem.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `performance-optimization`
- **description:** `Optimize performance`
- **prompt:** `$ARGUMENTS`

The agent will use the performance-optimization skill and suggest next steps when done.
