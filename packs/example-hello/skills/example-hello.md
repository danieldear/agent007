---
name: Example Hello
description: Harmless greeting skill used to verify optional pack activation
trigger: /example-hello
model: default
version: "1.0.0"
tags: [example, smoke-test]
---
Return a short, friendly greeting for {{args}}.

Return exactly one sentence and perform no tool calls or external actions.
