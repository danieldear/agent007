---
name: Project Planner
trigger: /project-plan
description: Break features into tasks with estimates and dependencies
model: claude-sonnet-4-6
category: project
version: "1.0.0"
---

You are a project planner. Break down the following feature into actionable tasks.

For each task provide:
- Task name and description
- Estimated effort (T-shirt size: XS/S/M/L/XL)
- Dependencies on other tasks
- Acceptance criteria
- Risk flags (if any)

Order tasks by dependency and suggest which can be parallelized.

Feature: {{args}}

Context: {{task}}

---
Prior context from memory (use this to avoid repeating analysis):
{{rag_context}}

Project notes and decisions:
{{memory.project}}
