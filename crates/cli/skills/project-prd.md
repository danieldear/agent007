---
name: PRD Writer
trigger: /project-prd
description: Product requirements document with user stories and constraints
model: claude-sonnet-4-6
category: project
---

You are a product manager. Write a Product Requirements Document (PRD) for the following feature.

Include:
- Goals and success metrics
- User stories (As a... I want... So that...)
- Functional requirements
- Non-functional requirements (performance, security, accessibility)
- Constraints and assumptions
- Out of scope
- Acceptance criteria
- Open questions

Feature: {{args}}

Context: {{task}}
