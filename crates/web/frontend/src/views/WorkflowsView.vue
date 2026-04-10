<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { VueFlow, useVueFlow } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls } from '@vue-flow/controls'
import { MiniMap } from '@vue-flow/minimap'
import { useApi } from '../composables/useApi.js'
import AgentNode from '../components/AgentNode.vue'
import EvaluatorNode from '../components/EvaluatorNode.vue'
import RouterNode from '../components/RouterNode.vue'
import ApprovalNode from '../components/ApprovalNode.vue'
import OrchestratorNode from '../components/OrchestratorNode.vue'

const { api } = useApi()
const workflows = ref([])
const personas = ref([])
const templates = ref([])
const selectedWorkflow = ref(null)
const showSaveDialog = ref(false)
const showTemplateMenu = ref(false)
const validating = ref(false)
const validationResult = ref(null)
const workflowName = ref('')
const workflowDescription = ref('')
const wfToast = ref(null)
const promotingWorkflow = ref(null)
const deletingWorkflow = ref(null)

let wfToastTimer = null
function showWfToast(message, type = 'success') {
  clearTimeout(wfToastTimer)
  wfToast.value = { message, type }
  wfToastTimer = setTimeout(() => { wfToast.value = null }, 3500)
}

async function promoteWorkflow(name) {
  promotingWorkflow.value = name
  try {
    const { ok, status, body } = await api.promoteWorkflow(name)
    if (status === 409) {
      showWfToast('Already exists globally', 'info')
    } else if (status === 404) {
      showWfToast('Not a project-local workflow', 'error')
    } else if (ok) {
      showWfToast('Promoted to global!', 'success')
    } else {
      showWfToast(body?.error || `Error ${status}`, 'error')
    }
  } catch (e) {
    showWfToast(e.message || 'Promote failed', 'error')
  } finally {
    promotingWorkflow.value = null
  }
}

async function deleteWorkflow(name) {
  if (!confirm(`Delete workflow "${name}"?\nThis will remove the file from disk.`)) return
  deletingWorkflow.value = name
  try {
    const { ok, body } = await api.deleteWorkflow(name)
    if (ok) {
      showWfToast(`Deleted "${name}"`, 'success')
      workflows.value = workflows.value.filter(w => w.name !== name)
      if (selectedWorkflow.value === name) selectedWorkflow.value = null
    } else {
      showWfToast(body?.error || 'Delete failed', 'error')
    }
  } catch (e) {
    showWfToast(e.message || 'Delete failed', 'error')
  } finally {
    deletingWorkflow.value = null
  }
}

const { onConnect, addEdges, getNodes, getEdges } = useVueFlow()

const nodes = ref([])
const edges = ref([])

const nodeTypes = {
  agent: AgentNode,
  evaluator: EvaluatorNode,
  router: RouterNode,
  approval: ApprovalNode,
  orchestrator: OrchestratorNode,
}

const contextMenu = ref({ show: false, x: 0, y: 0, type: null, targetId: null })

// Prompt templates keyed by agent/persona name and node type
// Available variables: {{task}}, {{memory.project}}, {{rag_context}}, {{<output_var>}} from prior steps
const PROMPT_TEMPLATES = {
  // By persona name
  Researcher:          'Research and gather comprehensive context for: {{task}}\n\nInclude: background, prior art, relevant patterns, constraints, and key facts needed to proceed.\n\n{{rag_context}}',
  Architect:           'Design the architecture for: {{task}}\n\nPrevious context: {{research_output}}\n\nDeliver: component breakdown, interfaces, data flow, technology choices, and trade-offs.\n\nProject decisions:\n{{memory.project}}',
  Coder:               'Implement the following based on the design:\n\n{{design_output}}\n\nOriginal task: {{task}}\n\nWrite clean, well-structured code with error handling.\n\nProject context:\n{{memory.project}}',
  ExpertCoder:         'Implement with senior-level expertise:\n\n{{design_output}}\n\nOriginal task: {{task}}\n\nApply language idioms, patterns, and best practices.\n\nProject context:\n{{memory.project}}',
  CodeReviewer:        'Review the following code for quality, correctness, and security:\n\n{{code}}\n\nCheck: logic errors, edge cases, security issues, performance, readability.\nRespond JSON: {"verdict": "pass" or "retry", "issues": [...], "suggestions": [...]}\n\nProject standards:\n{{memory.project}}',
  SecurityReviewer:    'Perform a security audit on:\n\n{{task}}\n\nCheck for: injection vulnerabilities, auth flaws, insecure data handling, dependency risks, OWASP Top 10.\nOutput: severity-ranked findings with remediation steps.',
  PerformanceEngineer: 'Analyze performance characteristics of:\n\n{{task}}\n\nIdentify: algorithmic complexity, memory usage, blocking operations, N+1 queries, missing caching.\nOutput: ranked issues with optimization recommendations.',
  UIUXDesigner:        'Design the user interface for:\n\n{{task}}\n\nContext: {{design_output}}\n\nDeliver: component structure, user flows, accessibility considerations, and implementation-ready specs.',
  DevOpsEngineer:      'Design the infrastructure and deployment pipeline for:\n\n{{task}}\n\nCover: containerization, CI/CD, scaling strategy, monitoring, rollback plan.',
  TestDesigner:        'Write a comprehensive test suite for:\n\n{{task}}\n\nCode under test: {{code}}\n\nInclude: unit tests, edge cases, error paths, integration tests. Use the red-green-refactor pattern.',
  DebugAgent:          'Investigate the following issue:\n\n{{task}}\n\nStep 1: Reproduce. Step 2: Hypothesize root causes. Step 3: Isolate. Step 4: Fix.\n\nPrior debugging notes:\n{{rag_context}}',
  // By node type
  evaluator:           'Evaluate the output against the acceptance criteria.\n\nOutput to review: {{step_output}}\nOriginal requirement: {{task}}\n\nRespond JSON: {"verdict": "pass" or "retry", "score": 0-10, "reason": "...", "fixes": "..."}',
  router:              'Classify this task into one of the available categories.\n\nTask: {{task}}\n\nRespond with exactly one of the category labels (no extra text).',
  approval:            'Review the following for human approval before proceeding.\n\nWork product: {{step_output}}\nOriginal task: {{task}}\n\nSummarize what was done and flag any concerns.',
}

function autogeneratePrompt() {
  const agent = nodeEditorForm.value.agent
  const type = editingNodeType.value
  // Try persona name first, then node type, then generic
  const template = PROMPT_TEMPLATES[agent] || PROMPT_TEMPLATES[type] ||
    `You are a ${agent || 'specialist agent'}. Complete the following task:\n\n{{task}}\n\nBe thorough, accurate, and output clearly structured results.`
  nodeEditorForm.value.prompt = template
}

// Node editor state
const showNodeEditor = ref(false)
const editingNodeId = ref(null)
const nodeEditorForm = ref({
  agent: '',
  prompt: '',
  output: '',
  // evaluator fields
  evaluate: { decision_field: 'verdict', on_pass: '', on_fail: '', max_retries: 3 },
  // router fields
  routes: [],
  // orchestrator fields
  workers: [],
})
const editingNodeType = ref('agent')

onMounted(async () => {
  const [wf, ps, tpl] = await Promise.all([
    api.listWorkflows(),
    api.listPersonas(),
    api.listTemplates(),
  ])
  if (wf) workflows.value = wf
  if (ps) personas.value = ps
  if (tpl) templates.value = tpl
})

onConnect((params) => {
  const sourceNode = nodes.value.find(n => n.id === params.source)
  let edgeStyle = { stroke: '#39d0c8' }
  let label = ''

  if (sourceNode?.type === 'evaluator') {
    const isRetry = params.sourceHandle === 'retry'
    edgeStyle = { stroke: isRetry ? '#f97316' : '#4ade80' }
    label = isRetry ? 'retry' : 'pass'
  } else if (sourceNode?.type === 'router') {
    edgeStyle = { stroke: '#a855f7' }
    label = params.sourceHandle || ''
  }

  addEdges([{ ...params, animated: true, style: edgeStyle, label }])
})

function handleKeydown(e) {
  if (e.key === 'Delete' || e.key === 'Backspace') {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return
    const selectedNodes = nodes.value.filter(n => n.selected)
    const selectedEdges = edges.value.filter(e => e.selected)
    if (selectedNodes.length) {
      const nodeIds = new Set(selectedNodes.map(n => n.id))
      nodes.value = nodes.value.filter(n => !nodeIds.has(n.id))
      edges.value = edges.value.filter(e => !nodeIds.has(e.source) && !nodeIds.has(e.target))
    }
    if (selectedEdges.length) {
      const edgeIds = new Set(selectedEdges.map(e => e.id))
      edges.value = edges.value.filter(e => !edgeIds.has(e.id))
    }
  }
}

onMounted(() => document.addEventListener('keydown', handleKeydown))
onUnmounted(() => document.removeEventListener('keydown', handleKeydown))

function showContextMenu(event, type, targetId) {
  event.preventDefault()
  contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type, targetId }
}

function hideContextMenu() {
  contextMenu.value = { show: false, x: 0, y: 0, type: null, targetId: null }
}

function deleteFromContextMenu() {
  const { type, targetId } = contextMenu.value
  if (type === 'node') {
    nodes.value = nodes.value.filter(n => n.id !== targetId)
    edges.value = edges.value.filter(e => e.source !== targetId && e.target !== targetId)
  } else if (type === 'edge') {
    edges.value = edges.value.filter(e => e.id !== targetId)
  }
  hideContextMenu()
}

function editFromContextMenu() {
  const { targetId } = contextMenu.value
  hideContextMenu()
  openNodeEditor(targetId)
}

function openNodeEditor(nodeId) {
  const node = nodes.value.find(n => n.id === nodeId)
  if (!node) return

  editingNodeId.value = nodeId
  editingNodeType.value = node.type || 'agent'
  nodeEditorForm.value = {
    agent: node.data.agent || '',
    prompt: node.data.prompt || '',
    output: node.data.output || '',
    evaluate: node.data.evaluate
      ? { ...node.data.evaluate }
      : { decision_field: 'verdict', on_pass: '', on_fail: '', max_retries: 3 },
    routes: node.data.routes
      ? node.data.routes.map(r => ({ ...r }))
      : [{ when: '', goto: '' }, { goto: '', default: true }],
    workers: node.data.workers ? [...node.data.workers] : [],
  }
  showNodeEditor.value = true
}

function saveNodeEdit() {
  const idx = nodes.value.findIndex(n => n.id === editingNodeId.value)
  if (idx === -1) return

  const updated = { ...nodes.value[idx] }
  updated.data = {
    ...updated.data,
    agent: nodeEditorForm.value.agent,
    prompt: nodeEditorForm.value.prompt,
    output: nodeEditorForm.value.output,
  }

  if (editingNodeType.value === 'evaluator') {
    updated.data.evaluate = { ...nodeEditorForm.value.evaluate }
  } else if (editingNodeType.value === 'router') {
    updated.data.routes = nodeEditorForm.value.routes.map(r => ({ ...r }))
  } else if (editingNodeType.value === 'orchestrator') {
    updated.data.workers = [...(nodeEditorForm.value.workers || [])]
  }

  nodes.value = [
    ...nodes.value.slice(0, idx),
    updated,
    ...nodes.value.slice(idx + 1),
  ]
  showNodeEditor.value = false
  editingNodeId.value = null
}

function addRouteRow() {
  nodeEditorForm.value.routes.push({ when: '', goto: '' })
}

function removeRouteRow(i) {
  nodeEditorForm.value.routes.splice(i, 1)
}

async function loadWorkflow(name) {
  const data = await api.getWorkflow(name)
  if (!data) return
  selectedWorkflow.value = name
  graphFromSteps(data.steps || [])
}

function graphFromSteps(steps) {
  const stepNodes = steps.map((step, i) => {
    let type = 'agent'
    if (step.type === 'evaluator') type = 'evaluator'
    else if (step.type === 'router') type = 'router'
    else if (step.requires_approval) type = 'approval'

    return {
      id: step.id,
      type,
      position: { x: 100 + (i % 3) * 300, y: 80 + Math.floor(i / 3) * 200 },
      data: {
        label: step.id,
        agent: step.agent,
        prompt: step.prompt,
        output: step.output || '',
        evaluate: step.evaluate || null,
        routes: step.routes || [],
        requires_approval: step.requires_approval || false,
      },
    }
  })

  const stepEdges = []
  for (const step of steps) {
    for (const dep of step.depends_on || []) {
      const sourceNode = stepNodes.find(n => n.id === dep)
      let style = { stroke: '#39d0c8' }
      let label = ''

      if (sourceNode?.type === 'evaluator') {
        style = { stroke: '#4ade80' }
        label = 'pass'
      } else if (sourceNode?.type === 'router') {
        style = { stroke: '#a855f7' }
      } else if (sourceNode?.type === 'approval') {
        style = { stroke: '#f59e0b' }
        label = 'approved'
      }

      stepEdges.push({
        id: `${dep}->${step.id}`,
        source: dep,
        target: step.id,
        animated: true,
        style,
        label,
      })
    }
  }

  nodes.value = stepNodes
  edges.value = stepEdges
}

let nodeCounter = 0

function addAgentNode(persona) {
  nodeCounter++
  const id = `step-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'agent',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: { label: id, agent: persona.name, prompt: '{{task}}', output: `${id}_output` },
  })
}

function addEvaluatorNode() {
  nodeCounter++
  const id = `eval-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'evaluator',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: {
      label: id,
      agent: 'CodeReviewer',
      prompt: 'Review: {{code}}. Respond JSON: {"verdict":"pass" or "retry","reason":"..."}',
      output: `${id}_result`,
      evaluate: { decision_field: 'verdict', on_pass: '', on_fail: '', max_retries: 3 },
    },
  })
}

function addRouterNode() {
  nodeCounter++
  const id = `router-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'router',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: {
      label: id,
      agent: 'Researcher',
      prompt: 'Classify this task: {{task}}',
      output: `${id}_classification`,
      routes: [
        { when: 'frontend', goto: '' },
        { when: 'backend', goto: '' },
        { goto: '', default: true },
      ],
    },
  })
}

function addApprovalNode() {
  nodeCounter++
  const id = `approval-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'approval',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: {
      label: id,
      agent: 'Architect',
      prompt: 'Review and approve the output: {{task}}',
      output: `${id}_approved`,
      requires_approval: true,
    },
  })
}

function addOrchestratorNode() {
  nodeCounter++
  const id = `orchestrator-${nodeCounter}`
  nodes.value.push({
    id,
    type: 'orchestrator',
    position: { x: 150 + (nodeCounter % 4) * 250, y: 100 + Math.floor(nodeCounter / 4) * 180 },
    data: {
      label: id,
      agent: 'Planner',
      prompt: 'Coordinate the following task across your workers: {{task}}',
      output: `${id}_result`,
      workers: [],
    },
  })
}

function exportYaml() {
  const currentNodes = getNodes.value
  const currentEdges = getEdges.value

  const steps = currentNodes.map(n => {
    const deps = currentEdges.filter(e => e.target === n.id).map(e => e.source)

    const step = {
      id: n.id,
      agent: n.data.agent,
      prompt: n.data.prompt,
      output: n.data.output || undefined,
      depends_on: deps.length ? deps : undefined,
    }

    if (n.type === 'evaluator') {
      step.type = 'evaluator'
      step.evaluate = n.data.evaluate
    } else if (n.type === 'router') {
      step.type = 'router'
      step.routes = n.data.routes
    } else if (n.type === 'approval') {
      step.requires_approval = true
    } else if (n.type === 'orchestrator') {
      step.type = 'orchestrator'
      if (n.data.workers?.length) step.workers = n.data.workers
    }

    return step
  })

  return {
    name: workflowName.value || selectedWorkflow.value || 'new-workflow',
    description: workflowDescription.value || '',
    steps,
  }
}

async function validateWorkflow() {
  if (!nodes.value.length) return
  validating.value = true
  validationResult.value = null
  try {
    const payload = exportYaml()
    validationResult.value = await api.validateWorkflow(payload)
  } catch (e) {
    validationResult.value = { valid: false, _error: e.message }
  } finally {
    validating.value = false
  }
}

async function saveWorkflow() {
  const yaml = exportYaml()
  await api.saveWorkflow(yaml)
  showSaveDialog.value = false
  const wf = await api.listWorkflows()
  if (wf) workflows.value = wf
}

function openSaveDialog() {
  workflowName.value = selectedWorkflow.value || ''
  workflowDescription.value = ''
  showSaveDialog.value = true
}

function newWorkflow() {
  selectedWorkflow.value = null
  nodes.value = []
  edges.value = []
  nodeCounter = 0
  showTemplateMenu.value = false
}

async function loadTemplate(tplName) {
  const tpl = await api.getTemplate(tplName)
  if (!tpl) return
  selectedWorkflow.value = null
  graphFromSteps(tpl.steps || [])
  showTemplateMenu.value = false
}
</script>

<template>
  <div class="flex flex-col h-full" @click="hideContextMenu">
    <!-- Toast -->
    <div v-if="wfToast" class="toast toast-top toast-end z-50 pointer-events-none">
      <div class="alert alert-sm shadow-lg" :class="{
        'alert-success': wfToast.type === 'success',
        'alert-info': wfToast.type === 'info',
        'alert-error': wfToast.type === 'error',
      }">
        <span class="text-sm">{{ wfToast.message }}</span>
      </div>
    </div>
    <div class="border-b border-base-300 bg-base-200">
      <div class="px-5 py-3.5 flex items-center justify-between">
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Workflow Designer</span>
        <div class="flex gap-2 items-center">
          <div class="dropdown dropdown-end">
            <label tabindex="0" class="btn btn-sm btn-ghost font-mono text-xs" @click.stop="showTemplateMenu = !showTemplateMenu">new ▾</label>
            <ul v-if="showTemplateMenu" tabindex="0" class="dropdown-content z-50 menu p-2 shadow bg-base-300 rounded-box w-56">
              <li><a @click="newWorkflow">Empty Canvas</a></li>
              <li class="menu-title"><span>Templates</span></li>
              <li v-for="tpl in templates" :key="tpl.name">
                <a @click="loadTemplate(tpl.name)">
                  <span class="font-mono text-xs">{{ tpl.name }}</span>
                  <span class="text-base-content/40 text-[10px]">{{ tpl.description?.slice(0, 30) }}</span>
                </a>
              </li>
            </ul>
          </div>
          <button
            class="btn btn-sm btn-ghost border border-base-content/20 gap-1 font-mono text-xs"
            :class="{ 'loading loading-spinner': validating }"
            :disabled="!nodes.length || validating"
            @click="validateWorkflow"
          >
            <span v-if="!validating">⊙ validate</span>
            <span v-else>validating…</span>
          </button>
          <button class="btn btn-sm btn-primary font-mono text-xs" @click="openSaveDialog" :disabled="!nodes.length">save</button>
        </div>
      </div>

      <!-- Validation result panel -->
      <div v-if="validationResult" class="px-4 pb-3">
        <div
          class="rounded-lg border text-xs font-mono overflow-hidden"
          :class="validationResult._error ? 'border-error/40 bg-error/5' :
                  validationResult.valid ? 'border-success/40 bg-success/5' : 'border-error/40 bg-error/5'"
        >
          <!-- Header -->
          <div
            class="flex items-center justify-between px-3 py-2 border-b"
            :class="validationResult._error ? 'border-error/20' :
                    validationResult.valid ? 'border-success/20' : 'border-error/20'"
          >
            <div class="flex items-center gap-2">
              <span v-if="validationResult._error" class="text-error font-bold">✗ Validation error</span>
              <span v-else-if="validationResult.valid" class="text-success font-bold">✓ Valid</span>
              <span v-else class="text-error font-bold">✗ Invalid</span>
              <span v-if="validationResult.structural?.warnings?.length" class="text-warning">
                {{ validationResult.structural.warnings.length }} warning{{ validationResult.structural.warnings.length !== 1 ? 's' : '' }}
              </span>
              <!-- LLM score badge -->
              <span
                v-if="validationResult.llm?.score != null"
                class="badge badge-sm font-mono"
                :class="validationResult.llm.score >= 7 ? 'badge-success' : validationResult.llm.score >= 4 ? 'badge-warning' : 'badge-error'"
              >LLM {{ validationResult.llm.score }}/10</span>
              <span v-if="validationResult.llm?.available === false" class="text-base-content/40 text-[10px]">
                (LLM validation requires standalone mode)
              </span>
            </div>
            <button class="btn btn-xs btn-ghost opacity-50 hover:opacity-100" @click="validationResult = null">✕</button>
          </div>

          <!-- Body -->
          <div class="p-3 space-y-2 max-h-48 overflow-y-auto">
            <!-- Network/fetch error -->
            <p v-if="validationResult._error" class="text-error">{{ validationResult._error }}</p>

            <!-- Structural errors -->
            <div v-if="validationResult.structural?.errors?.length">
              <p class="text-error/70 uppercase tracking-wider text-[10px] mb-1">Errors</p>
              <ul class="space-y-0.5">
                <li v-for="e in validationResult.structural.errors" :key="e" class="text-error flex gap-1.5">
                  <span class="shrink-0">✗</span><span>{{ e }}</span>
                </li>
              </ul>
            </div>

            <!-- Structural warnings -->
            <div v-if="validationResult.structural?.warnings?.length">
              <p class="text-warning/70 uppercase tracking-wider text-[10px] mb-1">Warnings</p>
              <ul class="space-y-0.5">
                <li v-for="w in validationResult.structural.warnings" :key="w" class="text-warning flex gap-1.5">
                  <span class="shrink-0">⚠</span><span>{{ w }}</span>
                </li>
              </ul>
            </div>

            <!-- LLM summary -->
            <div v-if="validationResult.llm?.summary">
              <p class="text-base-content/50 uppercase tracking-wider text-[10px] mb-1">LLM Summary</p>
              <p class="text-base-content/80">{{ validationResult.llm.summary }}</p>
            </div>

            <!-- LLM issues -->
            <div v-if="validationResult.llm?.issues?.length">
              <p class="text-error/70 uppercase tracking-wider text-[10px] mb-1">LLM Issues</p>
              <ul class="space-y-0.5">
                <li v-for="issue in validationResult.llm.issues" :key="issue" class="text-error/80 flex gap-1.5">
                  <span class="shrink-0">→</span><span>{{ issue }}</span>
                </li>
              </ul>
            </div>

            <!-- LLM suggestions -->
            <div v-if="validationResult.llm?.suggestions?.length">
              <p class="text-primary/70 uppercase tracking-wider text-[10px] mb-1">Suggestions</p>
              <ul class="space-y-0.5">
                <li v-for="s in validationResult.llm.suggestions" :key="s" class="text-primary/80 flex gap-1.5">
                  <span class="shrink-0">→</span><span>{{ s }}</span>
                </li>
              </ul>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="flex flex-1 overflow-hidden">
      <div class="w-56 bg-base-200 border-r border-base-300 flex flex-col shrink-0 overflow-auto">
        <div class="p-3 border-b border-base-300">
          <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-2">Workflows</div>
          <div class="space-y-1">
            <div v-for="w in workflows" :key="w.name" class="flex items-center gap-0.5 group/wf">
              <button
                class="btn btn-ghost btn-xs justify-start flex-1 font-mono text-xs overflow-hidden"
                :class="{ 'border-l-2 border-primary text-primary': selectedWorkflow === w.name }"
                @click="loadWorkflow(w.name)"
              >{{ w.name }}</button>
              <button
                v-if="w.source === 'project'"
                class="btn btn-ghost btn-xs px-1 text-xs opacity-0 group-hover/wf:opacity-60 hover:!opacity-100 transition-opacity shrink-0 font-mono"
                :class="{ 'loading loading-spinner': promotingWorkflow === w.name }"
                :disabled="promotingWorkflow === w.name"
                title="Copy to global ~/.agent007/workflows/"
                @click.stop="promoteWorkflow(w.name)"
              >↑</button>
              <button
                class="btn btn-ghost btn-xs px-1 text-xs opacity-0 group-hover/wf:opacity-100 hover:!opacity-100 transition-opacity shrink-0 font-mono text-error/60 hover:text-error"
                :class="{ 'loading loading-spinner': deletingWorkflow === w.name }"
                :disabled="deletingWorkflow === w.name"
                title="Delete this workflow"
                @click.stop="deleteWorkflow(w.name)"
              >✕</button>
            </div>
          </div>
          <div v-if="!workflows.length" class="text-[11px] font-mono text-base-content/30">no workflows</div>
        </div>

        <div class="p-3 border-b border-base-300">
          <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-2">Special Nodes</div>
          <div class="space-y-1">
            <button class="btn btn-ghost btn-xs justify-start w-full text-xs" @click="addEvaluatorNode">
              <span class="text-orange-400">↺</span> Evaluator
              <span class="ml-auto text-base-content/30 text-[10px]">pass/retry</span>
            </button>
            <button class="btn btn-ghost btn-xs justify-start w-full text-xs" @click="addRouterNode">
              <span class="text-purple-400">⑂</span> Router
              <span class="ml-auto text-base-content/30 text-[10px]">classify</span>
            </button>
            <button class="btn btn-ghost btn-xs justify-start w-full text-xs" @click="addApprovalNode">
              <span class="text-amber-400">⏸</span> Approval Gate
              <span class="ml-auto text-base-content/30 text-[10px]">human</span>
            </button>
            <button class="btn btn-ghost btn-xs justify-start w-full text-xs" @click="addOrchestratorNode">
              <span class="text-teal-400">⬡</span> Orchestrator
              <span class="ml-auto text-base-content/30 text-[10px]">fan-out</span>
            </button>
          </div>
        </div>

        <!-- Node Legend / Documentation -->
        <div class="p-3 border-b border-base-300">
          <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-2">Node Guide</div>
          <div class="space-y-2 text-[10px] font-mono text-base-content/50">
            <div>
              <span class="text-primary font-bold">◉ Agent</span>
              <p class="mt-0.5">Runs a single persona step. Connects via <code class="text-base-content/70">depends_on</code> edges. Output stored in the named variable.</p>
            </div>
            <div>
              <span class="text-orange-400 font-bold">↺ Evaluator</span>
              <p class="mt-0.5">Runs a step and checks its JSON verdict. On <span class="text-green-400">pass</span> continues; on <span class="text-orange-400">fail</span> retries up to <code class="text-base-content/70">max_retries</code> times.</p>
            </div>
            <div>
              <span class="text-purple-400 font-bold">⑂ Router</span>
              <p class="mt-0.5">Classifies input and branches to a different step based on named <code class="text-base-content/70">routes</code>. Each route has a <code class="text-base-content/70">when</code> condition and <code class="text-base-content/70">goto</code> target.</p>
            </div>
            <div>
              <span class="text-amber-400 font-bold">⏸ Approval Gate</span>
              <p class="mt-0.5">Pauses the workflow and waits for a human to approve or deny before continuing.</p>
            </div>
            <div>
              <span class="text-teal-400 font-bold">⬡ Orchestrator</span>
              <p class="mt-0.5">Fan-out coordinator: dispatches subtasks to named <code class="text-base-content/70">workers</code> in parallel and aggregates their results.</p>
            </div>
          </div>
        </div>

        <div class="p-3">
          <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-2">Agent Palette</div>
          <p class="text-[10px] font-mono text-base-content/30 mb-2">click to add a node</p>
          <div class="space-y-1">
            <button
              v-for="p in personas" :key="p.name"
              class="btn btn-ghost btn-xs justify-start w-full text-xs"
              @click="addAgentNode(p)"
            >
              <span class="text-primary">◉</span> {{ p.name }}
            </button>
          </div>
        </div>
      </div>

      <div class="flex-1 relative">
        <VueFlow
          v-model:nodes="nodes"
          v-model:edges="edges"
          :node-types="nodeTypes"
          :default-viewport="{ zoom: 0.9, x: 0, y: 0 }"
          :edges-updatable="true"
          :nodes-draggable="true"
          :select-nodes-on-drag="false"
          fit-view-on-init
          class="h-full"
          @node-context-menu="({ event, node }) => showContextMenu(event, 'node', node.id)"
          @edge-context-menu="({ event, edge }) => showContextMenu(event, 'edge', edge.id)"
          @node-double-click="({ node }) => openNodeEditor(node.id)"
        >
          <Background variant="dots" :gap="20" :size="1" />
          <Controls position="bottom-right" />
          <MiniMap position="bottom-left" />
        </VueFlow>

        <div v-if="!nodes.length" class="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div class="text-center text-base-content/30">
            <div class="text-4xl mb-2">⬡</div>
            <p class="text-sm">Load a workflow, pick a template, or add agents from the palette</p>
            <p class="text-xs mt-1 text-base-content/20">Double-click a node to edit its prompt. Right-click for options.</p>
          </div>
        </div>
      </div>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      class="fixed z-50 bg-base-300 border border-base-content/10 rounded-lg shadow-xl py-1 min-w-36"
      :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }"
      @click.stop
    >
      <button
        v-if="contextMenu.type === 'node'"
        class="w-full text-left px-3 py-1.5 text-xs font-mono hover:bg-primary/10 text-base-content"
        @click="editFromContextMenu"
      >
        ◦ edit node
      </button>
      <div v-if="contextMenu.type === 'node'" class="divider my-0.5 h-px bg-base-content/10 mx-2"></div>
      <button class="w-full text-left px-3 py-1.5 text-xs font-mono hover:bg-error/20 text-error" @click="deleteFromContextMenu">
        ✕ delete {{ contextMenu.type === 'node' ? 'node' : 'edge' }}
      </button>
    </div>

    <!-- Node Editor Modal -->
    <dialog :open="showNodeEditor" class="modal" :class="{ 'modal-open': showNodeEditor }">
      <div class="modal-box max-w-2xl bg-base-200 border border-base-300">
        <div class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40 flex items-center gap-2 mb-4">
          edit node ·
          <span v-if="editingNodeType === 'evaluator'" class="text-orange-400">evaluator</span>
          <span v-else-if="editingNodeType === 'router'" class="text-purple-400">router</span>
          <span v-else-if="editingNodeType === 'approval'" class="text-amber-400">approval gate</span>
          <span v-else-if="editingNodeType === 'orchestrator'" class="text-teal-400">orchestrator</span>
          <span v-else class="text-primary">agent</span>
        </div>

        <div class="mt-4 space-y-3">
          <!-- Agent -->
          <div class="grid grid-cols-2 gap-3">
            <div class="form-control">
              <label class="label"><span class="label-text text-xs">Agent / Persona</span></label>
              <select v-model="nodeEditorForm.agent" class="select select-sm select-bordered">
                <option v-for="p in personas" :key="p.name" :value="p.name">{{ p.name }}</option>
                <option :value="nodeEditorForm.agent" v-if="nodeEditorForm.agent && !personas.find(p => p.name === nodeEditorForm.agent)">
                  {{ nodeEditorForm.agent }} (custom)
                </option>
              </select>
            </div>
            <div class="form-control">
              <label class="label"><span class="label-text text-xs">Output Key</span></label>
              <input v-model="nodeEditorForm.output" class="input input-sm input-bordered font-mono" placeholder="step_output" />
            </div>
          </div>

          <!-- Prompt -->
          <div class="form-control">
            <label class="label">
              <span class="label-text text-xs">Prompt</span>
              <span class="label-text-alt flex items-center gap-2">
                <span class="text-xs text-base-content/40">Use <code class="font-mono bg-base-300 px-1 rounded">&#123;&#123;task&#125;&#125;</code> and output keys from previous steps</span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost text-primary border border-primary/30 hover:border-primary gap-1"
                  title="Autogenerate prompt based on selected agent"
                  @click="autogeneratePrompt"
                >⚡ Autogenerate</button>
              </span>
            </label>
            <textarea
              v-model="nodeEditorForm.prompt"
              class="textarea textarea-bordered text-sm font-mono h-36 resize-y"
              placeholder="Describe what this agent should do. Use {{task}} for the workflow input."
            />
          </div>

          <!-- Evaluator-specific config -->
          <div v-if="editingNodeType === 'evaluator'" class="space-y-3 border border-orange-500/30 rounded-lg p-3 bg-orange-500/5">
            <h4 class="text-xs font-bold text-orange-400 uppercase tracking-wider">Evaluator Config</h4>
            <p class="text-xs text-base-content/40">The prompt must return JSON with a decision field. If the verdict is "pass", the workflow moves forward; otherwise it retries.</p>
            <div class="grid grid-cols-3 gap-3">
              <div class="form-control">
                <label class="label"><span class="label-text text-xs">Decision Field</span></label>
                <input v-model="nodeEditorForm.evaluate.decision_field" class="input input-xs input-bordered font-mono" placeholder="verdict" />
              </div>
              <div class="form-control">
                <label class="label"><span class="label-text text-xs">On Pass → step</span></label>
                <input v-model="nodeEditorForm.evaluate.on_pass" class="input input-xs input-bordered font-mono" placeholder="deploy" />
              </div>
              <div class="form-control">
                <label class="label"><span class="label-text text-xs">On Fail → step</span></label>
                <input v-model="nodeEditorForm.evaluate.on_fail" class="input input-xs input-bordered font-mono" placeholder="implement" />
              </div>
            </div>
            <div class="form-control w-32">
              <label class="label"><span class="label-text text-xs">Max Retries</span></label>
              <input v-model.number="nodeEditorForm.evaluate.max_retries" type="number" min="1" max="10" class="input input-xs input-bordered" />
            </div>
          </div>

          <!-- Router-specific config -->
          <div v-if="editingNodeType === 'router'" class="space-y-3 border border-purple-500/30 rounded-lg p-3 bg-purple-500/5">
            <div class="flex items-center justify-between">
              <h4 class="text-xs font-bold text-purple-400 uppercase tracking-wider">Routes</h4>
              <button class="btn btn-xs btn-ghost text-purple-400" @click="addRouteRow">+ Add Route</button>
            </div>
            <p class="text-xs text-base-content/40">The prompt must output a classification string. Each route matches on that string and jumps to the target step.</p>
            <div class="space-y-2">
              <div v-for="(route, i) in nodeEditorForm.routes" :key="i" class="flex gap-2 items-center">
                <input
                  v-if="!route.default"
                  v-model="route.when"
                  class="input input-xs input-bordered font-mono flex-1"
                  placeholder="frontend"
                />
                <span v-else class="text-xs text-base-content/40 italic flex-1">default</span>
                <span class="text-base-content/40 text-xs">→</span>
                <input v-model="route.goto" class="input input-xs input-bordered font-mono flex-1" placeholder="step-id" />
                <button class="btn btn-xs btn-ghost text-error" @click="removeRouteRow(i)" :disabled="nodeEditorForm.routes.length <= 1">✕</button>
              </div>
            </div>
          </div>

          <!-- Approval note -->
          <div v-if="editingNodeType === 'approval'" class="border border-amber-500/30 rounded-lg p-3 bg-amber-500/5">
            <h4 class="text-xs font-bold text-amber-400 uppercase tracking-wider mb-1">Approval Gate</h4>
            <p class="text-xs text-base-content/40">This node will pause the workflow and wait for a human decision via <code class="font-mono bg-base-300 px-1 rounded">POST /api/runs/&#123;id&#125;/approval</code> before continuing.</p>
          </div>

          <!-- Orchestrator config -->
          <div v-if="editingNodeType === 'orchestrator'" class="space-y-3 border border-teal-500/30 rounded-lg p-3 bg-teal-500/5">
            <div class="flex items-center justify-between">
              <h4 class="text-xs font-bold text-teal-400 uppercase tracking-wider">Worker Steps</h4>
              <button class="btn btn-xs btn-ghost text-teal-400" @click="nodeEditorForm.workers = [...(nodeEditorForm.workers || []), '']">+ Add Worker</button>
            </div>
            <p class="text-xs text-base-content/40">List the step IDs that this orchestrator will fan-out to in parallel. Results are aggregated and passed to the next step.</p>
            <div class="space-y-2">
              <div v-for="(w, i) in (nodeEditorForm.workers || [])" :key="i" class="flex gap-2 items-center">
                <input
                  :value="w"
                  @input="nodeEditorForm.workers[i] = $event.target.value"
                  class="input input-xs input-bordered font-mono flex-1"
                  placeholder="worker-step-id"
                />
                <button class="btn btn-xs btn-ghost text-error" @click="nodeEditorForm.workers.splice(i, 1)">✕</button>
              </div>
            </div>
          </div>
        </div>

        <div class="modal-action">
          <button class="btn btn-sm btn-ghost font-mono text-xs" @click="showNodeEditor = false">cancel</button>
          <button class="btn btn-sm btn-primary font-mono text-xs" @click="saveNodeEdit">apply</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showNodeEditor = false">close</button></form>
    </dialog>

    <!-- Save dialog -->
    <dialog :open="showSaveDialog" class="modal" :class="{ 'modal-open': showSaveDialog }">
      <div class="modal-box bg-base-200 border border-base-300">
        <div class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-4">save workflow</div>
        <div class="space-y-3">
          <div class="form-control">
            <label class="label py-1"><span class="label-text text-[11px] font-mono text-base-content/50">name</span></label>
            <input v-model="workflowName" class="input input-sm input-bordered font-mono" placeholder="my-workflow" />
          </div>
          <div class="form-control">
            <label class="label py-1"><span class="label-text text-[11px] font-mono text-base-content/50">description</span></label>
            <input v-model="workflowDescription" class="input input-sm input-bordered font-mono" />
          </div>
        </div>
        <div class="modal-action">
          <button class="btn btn-sm btn-ghost font-mono text-xs" @click="showSaveDialog = false">cancel</button>
          <button class="btn btn-sm btn-primary font-mono text-xs" @click="saveWorkflow">save</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showSaveDialog = false">close</button></form>
    </dialog>
  </div>
</template>

<style>
@import '@vue-flow/core/dist/style.css';
@import '@vue-flow/core/dist/theme-default.css';
@import '@vue-flow/controls/dist/style.css';
@import '@vue-flow/minimap/dist/style.css';

.vue-flow {
  background: oklch(0.2 0.01 260);
}
.vue-flow__minimap {
  background: oklch(0.15 0.01 260);
}
.vue-flow__edge-text {
  font-size: 10px;
  fill: oklch(0.7 0 0);
}
</style>
