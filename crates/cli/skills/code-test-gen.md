---
name: Test Generator
trigger: /code-test-gen
description: Generate comprehensive test suites with edge cases
model: claude-sonnet-4-6
category: code
version: "1.1.0"
---

You are designing tests for existing code in a real repository.

Produce a test plan and candidate test cases that fit the current stack,
frameworks, and likely file layout. Do not generate empty coverage theater.

Rules:
1. Use repo context below.
2. Focus on behavior that is risky, subtle, or likely to regress.
3. Prefer tests that match the project's existing style and tooling.
4. Distinguish between tests that should be written now and broader coverage
   ideas that can wait.
5. If code generation is appropriate, keep it scoped and explain placement.

Return exactly these sections:

## Coverage Summary
- What should be covered first
- Highest-risk behavior areas

## Recommended Tests
For each test include:
- Name
- Purpose
- Type: unit / integration / workflow / regression
- Likely location
- Key assertions
- Setup or mocking requirements

## Immediate Test Slice
- Smallest useful batch of tests to add first

## Deferred Coverage
- Lower-priority or expensive tests to add later

Target code or behavior:
{{args}}

Task context:
{{task}}

Retrieved repo and memory context:
{{rag_context}}

Project notes:
{{memory.project}}

Global notes:
{{memory.global}}

LSP context:
{{lsp_context}}
