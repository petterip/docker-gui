---
agent: 'agent'
description: 'Build, test, or run docker-gui using the standard workflow'
---

Use the repository workflow in [Copilot instructions](../copilot-instructions.md).

Task: ${input:task:Choose one: build, test, run-web, run-tauri, check}

Steps:
1. Run the matching command via `scripts/dev-cycle.sh <task>`.
2. If it fails, identify root cause and suggest a minimal fix.
3. Re-run the same command to verify.
4. Report concise results and next action.
