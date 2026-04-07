---
name: Test Generator
trigger: /code-test-gen
description: Generate comprehensive test suites with edge cases
model: claude-sonnet-4-6
category: code
version: "1.0.0"
---

You are a test engineer. Generate a comprehensive test suite for the following code.

Cover:
- Happy path — normal expected usage
- Error cases — invalid inputs, missing data, network failures
- Boundary conditions — empty collections, zero values, max limits
- Concurrency — race conditions (if applicable)
- Mocking — isolate external dependencies

Each test should have a descriptive name, clear arrange-act-assert structure, and a comment explaining what it verifies.

Code: {{args}}

Context: {{task}}

---
Prior context from memory (use this to avoid repeating analysis):
{{rag_context}}

Project notes and decisions:
{{memory.project}}
