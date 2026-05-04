<script setup>
import { ref } from 'vue'

const activeSection = ref('concepts')
const sections = [
  { id: 'concepts',   label: 'Core Concepts' },
  { id: 'quickstart', label: 'Quick Start' },
  { id: 'invoke',     label: 'Invoking Skills & Tools' },
  { id: 'mcp',        label: 'MCP Servers' },
  { id: 'extensions', label: 'Extensions' },
  { id: 'rag',        label: 'RAG Sources' },
  { id: 'tools',      label: 'Tool Registry' },
  { id: 'nodes',      label: 'Workflow Nodes' },
  { id: 'cli',        label: 'CLI Reference' },
]

const cliCommands = [
  { cmd: 'agent007 init', desc: 'Initialize .agent007/ in the current project. Seeds built-in skills, workflows, personas, and installs slash commands for Claude, Codex, and Zed.' },
  { cmd: 'agent007 init --global', desc: 'Initialize or refresh ~/.agent007/ (global). Use this to update built-in skills without touching project directories.' },
  { cmd: 'agent007 serve', desc: 'Start the MCP server (default port 8007). Exposes all agent007 tools to your editor via the Model Context Protocol.' },
  { cmd: 'agent007 serve --standalone', desc: 'Start in standalone mode with a built-in LLM provider. Enables running tasks and skills directly from the web dashboard, with full output persisted in the selected run.' },
  { cmd: 'agent007 run "<task>"', desc: 'Run a task through the full agent stack with the TUI progress view. Useful for one-off tasks from the terminal.' },
  { cmd: 'agent007 memory list', desc: 'List all keys stored in project memory.' },
  { cmd: 'agent007 memory set <key> <value>', desc: 'Store a value in project memory. Available as {{memory.<key>}} in skill/workflow prompts.' },
]

const promptVars = [
  { name: '{{args}}',          desc: 'Arguments passed to the skill from the slash command.' },
  { name: '{{task}}',          desc: 'The task description passed to the workflow.' },
  { name: '{{memory.project}}', desc: 'Contents of project memory.' },
  { name: '{{rag_context}}',   desc: 'RAG-retrieved context from memory store.' },
  { name: '{{step_id_output}}', desc: 'Output of a prior workflow step (replace step_id with actual ID).' },
]
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">
    <!-- Header -->
    <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <div>
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Guide</span>
        <p class="text-[11px] font-mono text-base-content/30 mt-0.5">concepts, quick start &amp; reference</p>
      </div>
    </div>

    <div class="flex flex-1 overflow-hidden">

      <!-- Section nav -->
      <aside class="w-44 bg-base-200 border-r border-base-300 flex flex-col py-4 shrink-0">
        <button
          v-for="s in sections"
          :key="s.id"
          class="text-left px-4 py-2 text-[11px] font-mono tracking-wide transition-colors"
          :class="activeSection === s.id
            ? 'text-primary bg-primary/8 border-l-2 border-primary'
            : 'text-base-content/50 hover:text-base-content/80 hover:bg-base-300/40'"
          @click="activeSection = s.id"
        >
          {{ s.label }}
        </button>
      </aside>

      <!-- Content -->
      <div class="flex-1 overflow-y-auto p-6 space-y-8">

        <!-- ═══════════════════════════════════════════════════════════
             CORE CONCEPTS
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'concepts'" class="max-w-4xl space-y-8">

          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">Core Concepts</h2>
            <p class="text-sm text-base-content/50">agent007 has three building blocks. Understanding how they differ makes everything click.</p>
          </div>

          <!-- Three concept cards -->
          <div class="grid grid-cols-1 md:grid-cols-3 gap-5">

            <!-- Personas -->
            <div class="card bg-base-200 border border-primary/25 shadow-sm">
              <div class="card-body p-5">
                <div class="flex items-center gap-2 mb-3">
                  <div class="w-7 h-7 rounded-lg bg-primary/15 flex items-center justify-center shrink-0">
                    <span class="text-primary text-sm">◉</span>
                  </div>
                  <div>
                    <h3 class="font-bold font-mono text-sm text-primary">Personas</h3>
                    <span class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider">WHO</span>
                  </div>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-4">
                  A <strong class="text-base-content/80">persona</strong> is a named role — it defines a system prompt, preferred model, and allowed tools. Think of it as a job description for an AI agent.
                </p>
                <div class="rounded-lg bg-base-300 p-3 text-[11px] font-mono text-base-content/70 leading-relaxed">
                  <div class="text-base-content/40 mb-1"># ~/.agent007/personas/</div>
                  <div><span class="text-primary">name</span> = <span class="text-success">"CodeReviewer"</span></div>
                  <div><span class="text-primary">preferred_model</span> = <span class="text-success">"claude-sonnet"</span></div>
                  <div class="mt-1"><span class="text-primary">system_prompt</span> = <span class="text-success">"""</span></div>
                  <div class="text-base-content/50 pl-2">Review code for security,</div>
                  <div class="text-base-content/50 pl-2">correctness, and performance.</div>
                  <div><span class="text-success">"""</span></div>
                </div>
                <div class="mt-3 flex items-start gap-2">
                  <span class="badge badge-primary badge-xs mt-0.5 shrink-0">used in</span>
                  <span class="text-[11px] text-base-content/50 font-mono">workflow steps via <code class="bg-base-300 px-1 rounded">agent: CodeReviewer</code></span>
                </div>
              </div>
            </div>

            <!-- Skills -->
            <div class="card bg-base-200 border border-yellow-500/25 shadow-sm">
              <div class="card-body p-5">
                <div class="flex items-center gap-2 mb-3">
                  <div class="w-7 h-7 rounded-lg bg-yellow-500/15 flex items-center justify-center shrink-0">
                    <span class="text-yellow-400 text-sm">⚡</span>
                  </div>
                  <div>
                    <h3 class="font-bold font-mono text-sm text-yellow-400">Skills</h3>
                    <span class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider">WHAT</span>
                  </div>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-4">
                  A <strong class="text-base-content/80">skill</strong> is a reusable prompt template with a slash-command trigger. Invoke it directly from your editor — one task, one call.
                </p>
                <div class="rounded-lg bg-base-300 p-3 text-[11px] font-mono text-base-content/70 leading-relaxed">
                  <div class="text-base-content/40 mb-1"># ~/.agent007/skills/code-review.md</div>
                  <div class="text-base-content/50">---</div>
                  <div><span class="text-yellow-400">trigger</span>: <span class="text-success">/code-review</span></div>
                  <div><span class="text-yellow-400">name</span>: <span class="text-success">Code Reviewer</span></div>
                  <div><span class="text-yellow-400">model</span>: <span class="text-success">claude-sonnet</span></div>
                  <div class="text-base-content/50">---</div>
                  <div class="mt-1 text-base-content/60">Review this code: <span class="text-info">&#123;&#123;args&#125;&#125;</span></div>
                </div>
                <div class="mt-3 flex items-start gap-2">
                  <span class="badge badge-warning badge-xs mt-0.5 shrink-0 text-warning-content">invoked via</span>
                  <span class="text-[11px] text-base-content/50 font-mono">slash command <code class="bg-base-300 px-1 rounded">/code-review</code> in editor</span>
                </div>
              </div>
            </div>

            <!-- Workflows -->
            <div class="card bg-base-200 border border-teal-500/25 shadow-sm">
              <div class="card-body p-5">
                <div class="flex items-center gap-2 mb-3">
                  <div class="w-7 h-7 rounded-lg bg-teal-500/15 flex items-center justify-center shrink-0">
                    <span class="text-teal-400 text-sm">⬡</span>
                  </div>
                  <div>
                    <h3 class="font-bold font-mono text-sm text-teal-400">Workflows</h3>
                    <span class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider">HOW</span>
                  </div>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-4">
                  A <strong class="text-base-content/80">workflow</strong> is a multi-step pipeline of persona steps. Steps can run in parallel or in sequence, passing outputs to each other.
                </p>
                <div class="rounded-lg bg-base-300 p-3 text-[11px] font-mono text-base-content/70 leading-relaxed">
                  <div class="text-base-content/40 mb-1"># ~/.agent007/workflows/review.yaml</div>
                  <div><span class="text-teal-400">steps</span>:</div>
                  <div class="pl-2">- <span class="text-teal-400">id</span>: security</div>
                  <div class="pl-4"><span class="text-teal-400">agent</span>: <span class="text-success">SecurityAuditor</span></div>
                  <div class="pl-4"><span class="text-teal-400">prompt</span>: <span class="text-success">Audit &#123;&#123;task&#125;&#125;</span></div>
                  <div class="pl-2 mt-1">- <span class="text-teal-400">id</span>: summary</div>
                  <div class="pl-4"><span class="text-teal-400">depends_on</span>: [security]</div>
                  <div class="pl-4"><span class="text-teal-400">agent</span>: <span class="text-success">TechLead</span></div>
                </div>
                <div class="mt-3 flex items-start gap-2">
                  <span class="badge badge-ghost badge-xs mt-0.5 shrink-0">invoked via</span>
                  <span class="text-[11px] text-base-content/50 font-mono">MCP tool <code class="bg-base-300 px-1 rounded">agent007_workflow_run</code></span>
                </div>
              </div>
            </div>
          </div>

          <!-- Comparison table -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-5">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-4">When to use which</h3>
              <div class="overflow-x-auto">
                <table class="table table-sm text-xs font-mono">
                  <thead>
                    <tr class="text-base-content/40 text-[10px] uppercase tracking-wider">
                      <th class="bg-transparent">Scenario</th>
                      <th class="bg-transparent">Use</th>
                      <th class="bg-transparent">Why</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td class="text-base-content/70">"Review this file quickly"</td>
                      <td><span class="badge badge-warning badge-sm text-warning-content">⚡ Skill</span></td>
                      <td class="text-base-content/50">One-shot, no orchestration needed</td>
                    </tr>
                    <tr>
                      <td class="text-base-content/70">"Review security + performance + style in parallel"</td>
                      <td><span class="badge badge-ghost badge-sm">⬡ Workflow</span></td>
                      <td class="text-base-content/50">Multiple specialists run concurrently</td>
                    </tr>
                    <tr>
                      <td class="text-base-content/70">"Create a specialized agent for a workflow step"</td>
                      <td><span class="badge badge-primary badge-sm">◉ Persona</span></td>
                      <td class="text-base-content/50">Defines who handles each step</td>
                    </tr>
                    <tr>
                      <td class="text-base-content/70">"Share my custom skills with a teammate"</td>
                      <td><span class="badge badge-secondary badge-sm">⇅ Bundle</span></td>
                      <td class="text-base-content/50">Export as .a7bundle, import anywhere</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
          </div>

          <!-- Source badges explainer -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-5">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Source Badges (PROJ vs GLOBAL)</h3>
              <div class="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
                <div class="flex items-start gap-3">
                  <span class="badge badge-warning badge-sm shrink-0 mt-0.5 text-warning-content font-mono">PROJ</span>
                  <div class="text-base-content/60 leading-relaxed">
                    Skill or workflow lives in <code class="bg-base-300 px-1 rounded">.agent007/</code> inside your project folder.
                    It's project-specific. Use <strong class="text-base-content/80">↑ Promote</strong> to move it to global so all projects can use it.
                  </div>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-ghost badge-sm shrink-0 mt-0.5 font-mono">GLOBAL</span>
                  <div class="text-base-content/60 leading-relaxed">
                    Lives in <code class="bg-base-300 px-1 rounded">~/.agent007/</code> — available across all projects and instances.
                    Built-in skills and workflows installed by <code class="bg-base-300 px-1 rounded">agent007 init</code> are always global.
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             QUICK START
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'quickstart'" class="max-w-3xl space-y-6">

          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">Quick Start</h2>
            <p class="text-sm text-base-content/50">Get up and running in minutes.</p>
          </div>

          <ul class="steps steps-vertical w-full">
            <li class="step step-primary">
              <div class="text-left pl-4">
                <p class="font-bold text-sm font-mono text-base-content/80">Install agent007</p>
                <div class="mt-2 rounded-lg bg-base-300 px-4 py-2 font-mono text-xs text-base-content/70 w-fit">
                  cargo install agent007
                </div>
              </div>
            </li>
            <li class="step step-primary">
              <div class="text-left pl-4">
                <p class="font-bold text-sm font-mono text-base-content/80">Initialize in your project</p>
                <div class="mt-2 rounded-lg bg-base-300 px-4 py-2 font-mono text-xs text-base-content/70 w-fit">
                  cd my-project &amp;&amp; agent007 init
                </div>
                <p class="text-xs text-base-content/40 mt-1">Seeds built-in skills, workflows, and slash commands for your editor.</p>
              </div>
            </li>
            <li class="step step-primary">
              <div class="text-left pl-4">
                <p class="font-bold text-sm font-mono text-base-content/80">Start the MCP server</p>
                <div class="mt-2 rounded-lg bg-base-300 px-4 py-2 font-mono text-xs text-base-content/70 w-fit">
                  agent007 serve
                </div>
                <p class="text-xs text-base-content/40 mt-1">Exposes MCP tools to your editor (Claude, Copilot, Cursor, Codex).</p>
              </div>
            </li>
            <li class="step">
              <div class="text-left pl-4">
                <p class="font-bold text-sm font-mono text-base-content/80">Use a skill from your editor</p>
                <div class="mt-2 rounded-lg bg-base-300 px-4 py-2 font-mono text-xs text-base-content/70 w-fit">
                  /code-review src/main.rs
                </div>
                <p class="text-xs text-base-content/40 mt-1">The host LLM routes to the skill via MCP and returns structured results.</p>
              </div>
            </li>
            <li class="step">
              <div class="text-left pl-4">
                <p class="font-bold text-sm font-mono text-base-content/80">Run a full workflow</p>
                <div class="mt-2 rounded-lg bg-base-300 px-4 py-2 font-mono text-xs text-base-content/70 w-fit">
                  agent007_workflow_run name="code-review" task="..."
                </div>
                <p class="text-xs text-base-content/40 mt-1">Multiple specialists run in parallel; results are synthesized by a lead persona.</p>
              </div>
            </li>
            <li class="step">
              <div class="text-left pl-4">
                <p class="font-bold text-sm font-mono text-base-content/80">Build your own skill</p>
                <p class="text-xs text-base-content/50 mt-1">
                  Go to <strong class="font-mono text-base-content/70">Skills → + New</strong>, set a trigger like
                  <code class="bg-base-300 px-1 rounded">/my-skill</code>, write your prompt template using
                  <code class="bg-base-300 px-1 rounded">&#123;&#123;args&#125;&#125;</code>, and save. agent007 now auto-syncs
                  Claude slash commands when skills/workflows are created or imported.
                </p>
              </div>
            </li>
          </ul>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             WORKFLOW NODES
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'nodes'" class="max-w-4xl space-y-6">

          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">Workflow Node Types</h2>
            <p class="text-sm text-base-content/50">Each node type controls how a step behaves in the workflow graph.</p>
          </div>

          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">

            <!-- Agent -->
            <div class="card bg-base-200 border border-primary/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-primary/15 flex items-center justify-center text-primary text-sm font-bold">◉</span>
                  <h3 class="font-bold font-mono text-sm text-primary">Agent</h3>
                  <span class="badge badge-primary badge-xs ml-auto">default</span>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  The standard step. Runs a single persona with a prompt and stores the result in an output variable. Most workflow steps are agents.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60 w-full overflow-x-auto">
                  <div>- <span class="text-primary">id</span>: analyze</div>
                  <div class="pl-2"><span class="text-primary">agent</span>: Researcher</div>
                  <div class="pl-2"><span class="text-primary">prompt</span>: Analyze &#123;&#123;task&#125;&#125;</div>
                  <div class="pl-2"><span class="text-primary">output</span>: analysis_result</div>
                </div>
                <div class="mt-3 flex gap-2 flex-wrap">
                  <span class="badge badge-ghost badge-xs">connects via depends_on</span>
                  <span class="badge badge-ghost badge-xs">outputs to variable</span>
                </div>
              </div>
            </div>

            <!-- Evaluator -->
            <div class="card bg-base-200 border border-orange-500/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-orange-500/15 flex items-center justify-center text-orange-400 text-sm font-bold">↺</span>
                  <h3 class="font-bold font-mono text-sm text-orange-400">Evaluator</h3>
                  <span class="badge badge-xs ml-auto" style="border-color: rgb(249 115 22 / 0.3); color: rgb(249 115 22)">type: evaluator</span>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Runs a step and checks its JSON output for a decision field. If the verdict is <code class="bg-base-300 px-1 rounded">pass</code>, the workflow moves forward; otherwise it retries up to <strong>max_retries</strong> times.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60 w-full overflow-x-auto">
                  <div>- <span class="text-orange-400">id</span>: check</div>
                  <div class="pl-2"><span class="text-orange-400">type</span>: evaluator</div>
                  <div class="pl-2"><span class="text-orange-400">evaluate</span>:</div>
                  <div class="pl-4">decision_field: verdict</div>
                  <div class="pl-4">on_pass: deploy</div>
                  <div class="pl-4">on_fail: fix</div>
                  <div class="pl-4">max_retries: 3</div>
                </div>
                <div class="mt-3 flex gap-2 flex-wrap">
                  <span class="badge badge-xs text-green-400 border-green-500/30">pass → next step</span>
                  <span class="badge badge-xs text-orange-400 border-orange-500/30">fail → retry</span>
                </div>
              </div>
            </div>

            <!-- Router -->
            <div class="card bg-base-200 border border-purple-500/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-purple-500/15 flex items-center justify-center text-purple-400 text-sm font-bold">⑂</span>
                  <h3 class="font-bold font-mono text-sm text-purple-400">Router</h3>
                  <span class="badge badge-xs ml-auto" style="border-color: rgb(168 85 247 / 0.3); color: rgb(168 85 247)">type: router</span>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Classifies input and branches to a different step based on named routes. The prompt should output a classification string that matches a <code class="bg-base-300 px-1 rounded">when</code> condition.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60 w-full overflow-x-auto">
                  <div>- <span class="text-purple-400">id</span>: classify</div>
                  <div class="pl-2"><span class="text-purple-400">type</span>: router</div>
                  <div class="pl-2"><span class="text-purple-400">routes</span>:</div>
                  <div class="pl-4">- when: frontend</div>
                  <div class="pl-6">goto: ui-specialist</div>
                  <div class="pl-4">- when: backend</div>
                  <div class="pl-6">goto: api-specialist</div>
                </div>
                <div class="mt-3 flex gap-2 flex-wrap">
                  <span class="badge badge-xs text-purple-400 border-purple-500/30">classify → branch</span>
                  <span class="badge badge-ghost badge-xs">supports default route</span>
                </div>
              </div>
            </div>

            <!-- Approval Gate -->
            <div class="card bg-base-200 border border-amber-500/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-amber-500/15 flex items-center justify-center text-amber-400 text-sm font-bold">⏸</span>
                  <h3 class="font-bold font-mono text-sm text-amber-400">Approval Gate</h3>
                  <span class="badge badge-xs ml-auto" style="border-color: rgb(245 158 11 / 0.3); color: rgb(245 158 11)">requires_approval</span>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Pauses the workflow and waits for a human decision before continuing. Externally initiated runs surface that approval back to the initiating client; dashboard-owned standalone runs can still be resumed in the dashboard. The run stays in <code class="bg-base-300 px-1 rounded">pending_approval</code> state until approved or denied via the API.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60 w-full overflow-x-auto">
                  <div>- <span class="text-amber-400">id</span>: gate</div>
                  <div class="pl-2"><span class="text-amber-400">agent</span>: Architect</div>
                  <div class="pl-2"><span class="text-amber-400">requires_approval</span>: true</div>
                  <div class="pl-2 text-base-content/40"># POST /api/runs/{'{'}id{'}'}/approval</div>
                </div>
                <div class="mt-3 flex gap-2 flex-wrap">
                  <span class="badge badge-xs text-amber-400 border-amber-500/30">approve → continue</span>
                  <span class="badge badge-xs text-error border-error/30">deny → abort</span>
                </div>
              </div>
            </div>

            <!-- Orchestrator (full width) -->
            <div class="card bg-base-200 border border-teal-500/25 md:col-span-2">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-teal-500/15 flex items-center justify-center text-teal-400 text-sm font-bold">⬡</span>
                  <h3 class="font-bold font-mono text-sm text-teal-400">Orchestrator</h3>
                  <span class="badge badge-xs ml-auto" style="border-color: rgb(20 184 166 / 0.3); color: rgb(20 184 166)">type: orchestrator</span>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  A fan-out coordinator. Dispatches the same task to multiple named <strong>worker steps</strong> in parallel and aggregates their results into a single output. Useful when you want several specialists to tackle the same problem independently.
                </p>
                <div class="grid grid-cols-2 gap-3">
                  <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60 w-full overflow-x-auto">
                    <div>- <span class="text-teal-400">id</span>: orchestrate</div>
                    <div class="pl-2"><span class="text-teal-400">type</span>: orchestrator</div>
                    <div class="pl-2"><span class="text-teal-400">agent</span>: Planner</div>
                    <div class="pl-2"><span class="text-teal-400">workers</span>:</div>
                    <div class="pl-4">- security-audit</div>
                    <div class="pl-4">- perf-check</div>
                    <div class="pl-4">- style-review</div>
                    <div class="pl-2"><span class="text-teal-400">output</span>: all_results</div>
                  </div>
                  <div class="space-y-2 text-xs text-base-content/50 self-center">
                    <div class="flex items-center gap-2"><span class="text-teal-400">⬡</span> Orchestrator dispatches to workers</div>
                    <div class="flex items-center gap-2"><span class="text-primary">◉</span> Workers run in parallel</div>
                    <div class="flex items-center gap-2"><span class="text-primary">◉</span> Results aggregated into output</div>
                    <div class="flex items-center gap-2"><span class="text-base-content/30">→</span> Next step reads combined output</div>
                  </div>
                </div>
                <div class="mt-3 flex gap-2 flex-wrap">
                  <span class="badge badge-xs text-teal-400 border-teal-500/30">fan-out to workers</span>
                  <span class="badge badge-xs text-teal-400 border-teal-500/30">parallel execution</span>
                  <span class="badge badge-ghost badge-xs">aggregated result</span>
                </div>
              </div>
            </div>

          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             INVOKING SKILLS & TOOLS
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'invoke'" class="max-w-4xl space-y-6">
          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">Invoking Skills &amp; Tools</h2>
            <p class="text-sm text-base-content/50">How to actually call the things you've configured.</p>
          </div>

          <!-- Invocation methods grid -->
          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">

            <div class="card bg-base-200 border border-yellow-500/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-yellow-500/15 flex items-center justify-center text-yellow-400 text-sm">⚡</span>
                  <h3 class="font-bold font-mono text-sm text-yellow-400">Skills — slash commands</h3>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Type a slash command directly in your editor (Claude, Codex, Cursor). agent007 intercepts it via MCP and runs the skill's prompt template.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/70 space-y-1">
                  <div><span class="text-yellow-400">/code-review</span> src/main.rs</div>
                  <div><span class="text-yellow-400">/dev-tdd</span> implement login feature</div>
                  <div><span class="text-yellow-400">/my-skill</span> any argument here</div>
                </div>
                <p class="text-[10px] font-mono text-base-content/35 mt-2">Skills → Skills tab to create or edit</p>
              </div>
            </div>

            <div class="card bg-base-200 border border-primary/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-primary/15 flex items-center justify-center text-primary text-sm">⬡</span>
                  <h3 class="font-bold font-mono text-sm text-primary">Workflows — MCP tool call</h3>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Workflows run via the MCP tool <code class="bg-base-300 px-1 rounded">agent007_workflow_run</code>. Call it from any LLM that has the agent007 MCP server connected.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/70 space-y-1">
                  <div><span class="text-primary">agent007_workflow_run</span></div>
                  <div class="pl-2">name=<span class="text-success">"code-review"</span></div>
                  <div class="pl-2">task=<span class="text-success">"review src/main.rs"</span></div>
                </div>
                <p class="text-[10px] font-mono text-base-content/35 mt-2">Workflows → Workflows tab to manage</p>
              </div>
            </div>

            <div class="card bg-base-200 border border-teal-500/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-teal-500/15 flex items-center justify-center text-teal-400 text-sm">⬡</span>
                  <h3 class="font-bold font-mono text-sm text-teal-400">MCP tools — call by name</h3>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Once an MCP server is <strong class="text-base-content/80">connected</strong> and <strong class="text-base-content/80">approved</strong>, its tools are exposed to the LLM automatically. Ask the LLM to use them by name, or generate a companion skill for one-click invocation.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/70 space-y-1">
                  <div class="text-base-content/40"># LLM can call directly:</div>
                  <div><span class="text-teal-400">read_file</span>({ path: "..." })</div>
                  <div><span class="text-teal-400">search_github</span>({ q: "..." })</div>
                  <div class="text-base-content/40 mt-1"># Or via a generated skill:</div>
                  <div><span class="text-yellow-400">/github-search</span> agent007</div>
                </div>
                <p class="text-[10px] font-mono text-base-content/35 mt-2">MCP tab → Connect server → ⚡ skill per tool</p>
              </div>
            </div>

            <div class="card bg-base-200 border border-orange-500/25">
              <div class="card-body p-4">
                <div class="flex items-center gap-2 mb-2">
                  <span class="w-6 h-6 rounded bg-orange-500/15 flex items-center justify-center text-orange-400 text-sm">🛠</span>
                  <h3 class="font-bold font-mono text-sm text-orange-400">Local tools — called by the agent</h3>
                </div>
                <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                  Tools (scripts/binaries on PATH or imported via Tools tab) are executed as subprocesses by the agent. Reference them in workflow step prompts or generate a skill to wrap them.
                </p>
                <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/70 space-y-1">
                  <div class="text-base-content/40"># In a workflow prompt:</div>
                  <div>Run <span class="text-orange-400">my-linter</span> on &#123;&#123;task&#125;&#125;</div>
                  <div class="text-base-content/40 mt-1"># As a skill:</div>
                  <div><span class="text-yellow-400">/lint</span> src/main.rs</div>
                </div>
                <p class="text-[10px] font-mono text-base-content/35 mt-2">Tools tab → Import to register local binaries</p>
              </div>
            </div>
          </div>

          <!-- Flow diagram -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Request flow</h3>
              <div class="flex items-center gap-2 flex-wrap text-xs font-mono text-base-content/60">
                <span class="bg-base-300 px-2 py-1 rounded">Editor / LLM</span>
                <span class="text-base-content/30">→ MCP call →</span>
                <span class="bg-base-300 px-2 py-1 rounded">agent007 server</span>
                <span class="text-base-content/30">→ resolves skill/workflow →</span>
                <span class="bg-base-300 px-2 py-1 rounded">persona + prompt</span>
                <span class="text-base-content/30">→ LLM API →</span>
                <span class="bg-base-300 px-2 py-1 rounded">result back to editor</span>
              </div>
            </div>
          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             MCP SERVERS
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'mcp'" class="max-w-4xl space-y-6">
          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">MCP Servers</h2>
            <p class="text-sm text-base-content/50">Model Context Protocol servers extend your AI assistant with external tools.</p>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">What is an MCP server?</h3>
              <p class="text-xs text-base-content/60 leading-relaxed">
                An MCP server is a subprocess that speaks the Model Context Protocol. Once connected, it exposes a list of <strong class="text-base-content/80">tools</strong> (functions) that the LLM can call — for example: reading files, querying databases, searching the web, or interacting with APIs. agent007 acts as the broker between your editor's LLM and any number of MCP servers.
              </p>
            </div>
          </div>

          <!-- Steps -->
          <div class="space-y-3">
            <div class="card bg-base-200 border border-primary/20">
              <div class="card-body p-4">
                <div class="flex items-start gap-3">
                  <span class="w-6 h-6 rounded-full bg-primary/15 flex items-center justify-center text-primary text-xs font-bold shrink-0">1</span>
                  <div>
                    <p class="font-bold font-mono text-sm text-base-content/80 mb-1">Register the server</p>
                    <p class="text-xs text-base-content/55 leading-relaxed mb-2">Go to <strong class="font-mono">MCP → + Add Server</strong>. For npm packages enter the package name and agent007 auto-configures <code class="bg-base-300 px-1 rounded">npx -y &lt;package&gt;</code>. For local binaries, specify the command.</p>
                    <div class="bg-base-300 rounded p-2 text-[11px] font-mono text-base-content/60">
                      Source kind: npm · Package: @modelcontextprotocol/server-github
                    </div>
                  </div>
                </div>
              </div>
            </div>
            <div class="card bg-base-200 border border-primary/20">
              <div class="card-body p-4">
                <div class="flex items-start gap-3">
                  <span class="w-6 h-6 rounded-full bg-primary/15 flex items-center justify-center text-primary text-xs font-bold shrink-0">2</span>
                  <div>
                    <p class="font-bold font-mono text-sm text-base-content/80 mb-1">Connect to discover tools</p>
                    <p class="text-xs text-base-content/55 leading-relaxed">Click <strong class="font-mono">⚡ Connect</strong> — agent007 launches the subprocess, performs an MCP handshake, and populates the tools list. Tools are stored in the registry so they survive restarts.</p>
                  </div>
                </div>
              </div>
            </div>
            <div class="card bg-base-200 border border-primary/20">
              <div class="card-body p-4">
                <div class="flex items-start gap-3">
                  <span class="w-6 h-6 rounded-full bg-primary/15 flex items-center justify-center text-primary text-xs font-bold shrink-0">3</span>
                  <div>
                    <p class="font-bold font-mono text-sm text-base-content/80 mb-1">Approve for LLM use</p>
                    <p class="text-xs text-base-content/55 leading-relaxed">Review the discovered tools, then click <strong class="font-mono">🛡 Approve</strong>. Only approved servers are made available to the LLM in agent workflows. This prevents accidentally exposing unreviewed tools.</p>
                  </div>
                </div>
              </div>
            </div>
            <div class="card bg-base-200 border border-primary/20">
              <div class="card-body p-4">
                <div class="flex items-start gap-3">
                  <span class="w-6 h-6 rounded-full bg-primary/15 flex items-center justify-center text-primary text-xs font-bold shrink-0">4</span>
                  <div>
                    <p class="font-bold font-mono text-sm text-base-content/80 mb-1">Generate skills (optional)</p>
                    <p class="text-xs text-base-content/55 leading-relaxed">Click <strong class="font-mono">⚡ skill</strong> next to any discovered tool to auto-generate a slash-command skill that wraps it. This gives you one-click invocation from your editor without knowing the tool's schema.</p>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Registry file location</h3>
              <code class="text-xs font-mono text-primary">~/.agent007/mcp/registry.json</code>
              <p class="text-xs text-base-content/50 mt-1">JSON array of server entries. Scope "project" servers are also stored in <code class="bg-base-300 px-1 rounded">.agent007/mcp/registry.json</code> inside your project.</p>
            </div>
          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             EXTENSIONS
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'extensions'" class="max-w-4xl space-y-6">
          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">Extensions</h2>
            <p class="text-sm text-base-content/50">Bundles that install skills, tools, MCP servers, and RAG sources in one step.</p>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-2">Extensions vs. MCP Servers</h3>
              <div class="overflow-x-auto">
                <table class="table table-sm text-xs font-mono">
                  <thead>
                    <tr class="text-base-content/40 text-[10px] uppercase tracking-wider">
                      <th class="bg-transparent"></th>
                      <th class="bg-transparent">MCP Server</th>
                      <th class="bg-transparent">Extension</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td class="text-base-content/60">What it is</td>
                      <td>A single subprocess with tools</td>
                      <td>A bundle of multiple components</td>
                    </tr>
                    <tr>
                      <td class="text-base-content/60">Can contain</td>
                      <td>Tools only</td>
                      <td>Skills + Tools + Workflows + MCP + RAG</td>
                    </tr>
                    <tr>
                      <td class="text-base-content/60">Install via</td>
                      <td>MCP tab → Add Server</td>
                      <td>Extensions tab → Browse or Import</td>
                    </tr>
                    <tr>
                      <td class="text-base-content/60">Source</td>
                      <td>npm, local binary, github</td>
                      <td>npm, GitHub, local path, OpenAPI, Claude</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <p class="text-[11px] font-mono text-base-content/40 mt-3">An extension can include MCP servers as one of its components — it handles registering them for you.</p>
            </div>
          </div>

          <!-- Component types -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Component types</h3>
              <div class="space-y-2 text-xs font-mono">
                <div class="flex items-start gap-3">
                  <span class="badge badge-warning badge-xs shrink-0 mt-0.5">skills</span>
                  <span class="text-base-content/60">Installed to <code class="bg-base-300 px-1 rounded">~/.agent007/skills/</code> — slash commands in your editor</span>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-info badge-xs shrink-0 mt-0.5">tools</span>
                  <span class="text-base-content/60">Installed to <code class="bg-base-300 px-1 rounded">~/.agent007/tools/</code> — scripts/binaries callable by agents</span>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-ghost badge-xs shrink-0 mt-0.5">workflows</span>
                  <span class="text-base-content/60">Installed to <code class="bg-base-300 px-1 rounded">~/.agent007/workflows/</code> — multi-step pipelines</span>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-accent badge-xs shrink-0 mt-0.5">MCP</span>
                  <span class="text-base-content/60">Registered in the MCP registry — auto-approved if the manifest says so</span>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-success badge-xs shrink-0 mt-0.5">RAG</span>
                  <span class="text-base-content/60">Registered as RAG sources — indexed automatically after install</span>
                </div>
              </div>
            </div>
          </div>

          <!-- Compat grades -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Compatibility grades</h3>
              <div class="space-y-2 text-xs font-mono">
                <div class="flex items-center gap-3">
                  <span class="badge badge-success font-bold shrink-0">A</span>
                  <span class="text-base-content/60">Fully verified — has an <code class="bg-base-300 px-1 rounded">agent007.json</code> manifest and all components are valid</span>
                </div>
                <div class="flex items-center gap-3">
                  <span class="badge badge-warning font-bold shrink-0">B</span>
                  <span class="text-base-content/60">Compatible but review recommended — npm package or partial manifest</span>
                </div>
                <div class="flex items-center gap-3">
                  <span class="badge badge-error font-bold shrink-0">C</span>
                  <span class="text-base-content/60">Partial — some components could not be parsed or verified</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             RAG SOURCES
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'rag'" class="max-w-4xl space-y-6">
          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">RAG Sources</h2>
            <p class="text-sm text-base-content/50">Index external knowledge so agents can retrieve it via <code class="bg-base-300 px-1 rounded">&#123;&#123;rag_context&#125;&#125;</code>.</p>
          </div>

          <!-- Status lifecycle -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Indexing lifecycle</h3>
              <div class="flex items-center gap-2 flex-wrap text-xs font-mono">
                <span class="badge badge-ghost">pending</span>
                <span class="text-base-content/30">→ added, not yet indexed →</span>
                <span class="badge badge-warning">indexing</span>
                <span class="text-base-content/30">→ reading content →</span>
                <span class="badge badge-success">ready</span>
                <span class="text-base-content/30">or</span>
                <span class="badge badge-error">error</span>
              </div>
              <p class="text-[11px] font-mono text-base-content/40 mt-3">Only <strong>ready</strong> sources appear in query results and are injected into prompt context. Click <strong>↺ reindex</strong> to re-process after the source content changes.</p>
            </div>
          </div>

          <!-- Source kinds -->
          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Supported source kinds</h3>
              <div class="space-y-3 text-xs font-mono">
                <div class="flex items-start gap-3">
                  <span class="badge badge-outline badge-xs shrink-0 mt-0.5">url</span>
                  <div class="text-base-content/60">
                    HTTP/HTTPS URL. agent007 fetches and chunks the page content. Useful for public documentation, API references, or any web content.
                    <div class="bg-base-300 rounded px-2 py-1 mt-1 text-[10px]">https://docs.example.com/api</div>
                  </div>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-outline badge-xs shrink-0 mt-0.5">file</span>
                  <div class="text-base-content/60">
                    A single file on disk. Supports any text format — Markdown, plain text, code files.
                    <div class="bg-base-300 rounded px-2 py-1 mt-1 text-[10px]">/path/to/architecture.md</div>
                  </div>
                </div>
                <div class="flex items-start gap-3">
                  <span class="badge badge-outline badge-xs shrink-0 mt-0.5">directory</span>
                  <div class="text-base-content/60">
                    A directory of files. agent007 walks the tree and indexes each readable file as a chunk group. Good for entire docs folders.
                    <div class="bg-base-300 rounded px-2 py-1 mt-1 text-[10px]">/path/to/docs/</div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-2">Using RAG in prompts</h3>
              <p class="text-xs text-base-content/60 leading-relaxed mb-2">Add <code class="bg-base-300 px-1 rounded">&#123;&#123;rag_context&#125;&#125;</code> to any skill or workflow prompt template. agent007 retrieves the most relevant chunks from ready sources and injects them at that position.</p>
              <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60">
                <div>Review the following code:</div>
                <div class="text-info">&#123;&#123;args&#125;&#125;</div>
                <div class="mt-1">Relevant project context:</div>
                <div class="text-success">&#123;&#123;rag_context&#125;&#125;</div>
              </div>
              <p class="text-[10px] font-mono text-base-content/35 mt-2">Add sources in Memory → RAG Sources tab. Test with the query box before using in production.</p>
            </div>
          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             TOOL REGISTRY
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'tools'" class="max-w-4xl space-y-6">
          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">Tool Registry</h2>
            <p class="text-sm text-base-content/50">Local scripts, binaries, and external APIs that agents can execute as subprocesses.</p>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">PATH discovery</h3>
              <p class="text-xs text-base-content/60 leading-relaxed">
                agent007 scans your <code class="bg-base-300 px-1 rounded">$PATH</code> and the project's <code class="bg-base-300 px-1 rounded">.agent007/tools/</code> directory on startup. Any executable found is registered as an available tool. The Tools tab shows all discovered tools grouped by source.
              </p>
            </div>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Importing local scripts</h3>
              <p class="text-xs text-base-content/60 leading-relaxed mb-3">
                Use <strong class="font-mono">Tools → Import</strong> to register a local script or binary. Provide a name and path — agent007 copies or symlinks it into <code class="bg-base-300 px-1 rounded">.agent007/tools/</code> and optionally generates a companion skill so it has a slash-command trigger.
              </p>
              <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60 space-y-1">
                <div>Name: <span class="text-primary">my-linter</span></div>
                <div>Path: <span class="text-success">/usr/local/bin/eslint</span></div>
                <div>Generate companion skill: <span class="text-warning">✓ yes → /lint</span></div>
              </div>
            </div>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">TOOL.yaml manifest</h3>
              <p class="text-xs text-base-content/60 leading-relaxed mb-2">
                Tools stored in <code class="bg-base-300 px-1 rounded">.agent007/tools/&lt;name&gt;/TOOL.yaml</code> get a rich manifest with schema validation, argument descriptions, and safety flags. Extensions install tools in this format automatically.
              </p>
              <div class="bg-base-300 rounded-lg p-3 text-[11px] font-mono text-base-content/60">
                <div><span class="text-primary">name</span>: my-linter</div>
                <div><span class="text-primary">command</span>: eslint</div>
                <div><span class="text-primary">description</span>: Lint JavaScript files</div>
                <div><span class="text-primary">args</span>:</div>
                <div class="pl-2">- name: <span class="text-success">target</span></div>
                <div class="pl-4">description: File or glob to lint</div>
                <div class="pl-4">required: <span class="text-warning">true</span></div>
              </div>
            </div>
          </div>
        </div>

        <!-- ═══════════════════════════════════════════════════════════
             CLI REFERENCE
        ════════════════════════════════════════════════════════════ -->
        <div v-if="activeSection === 'cli'" class="max-w-3xl space-y-6">

          <div>
            <h2 class="text-base font-bold font-mono uppercase tracking-widest text-base-content/70 mb-1">CLI Reference</h2>
            <p class="text-sm text-base-content/50">Common agent007 commands.</p>
          </div>

          <div class="space-y-3">
            <div v-for="cmd in cliCommands" :key="cmd.cmd" class="card bg-base-200 border border-base-300">
              <div class="card-body p-4">
                <code class="text-sm font-mono text-primary">{{ cmd.cmd }}</code>
                <p class="text-xs text-base-content/55 mt-1 leading-relaxed">{{ cmd.desc }}</p>
              </div>
            </div>
          </div>

          <div class="card bg-base-200 border border-base-300">
            <div class="card-body p-4">
              <h3 class="font-bold font-mono text-xs uppercase tracking-widest text-base-content/50 mb-3">Prompt Variables</h3>
              <div class="space-y-2 text-xs font-mono">
                <div v-for="v in promptVars" :key="v.name" class="flex items-start gap-3">
                  <code class="bg-base-300 px-1.5 py-0.5 rounded text-info shrink-0">{{ v.name }}</code>
                  <span class="text-base-content/55 leading-relaxed">{{ v.desc }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>

      </div>
    </div>
  </div>
</template>
