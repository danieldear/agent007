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
const showNodeGuide = ref(false)

// ── resizable sidebar ────────────────────────────────────────────────────
const sidebarWidth = ref(224) // default w-56 = 224px
const isResizing = ref(false)

function startResize(e) {
  isResizing.value = true
  const startX = e.clientX
  const startW = sidebarWidth.value

  function onMove(ev) {
    const delta = ev.clientX - startX
    sidebarWidth.value = Math.min(400, Math.max(160, startW + delta))
  }
  function onUp() {
    isResizing.value = false
    window.removeEventListener('mousemove', onMove)
    window.removeEventListener('mouseup', onUp)
  }
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
}

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
const skills = ref([])
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
  const [wf, ps, tpl, sk] = await Promise.all([
    api.listWorkflows(),
    api.listPersonas(),
    api.listTemplates(),
    api.listSkills(),
  ])
  if (wf) workflows.value = wf
  if (ps) personas.value = ps
  if (tpl) templates.value = tpl
  if (sk) skills.value = sk
})

function workflowScopeChips(item) {
  const variants = Array.isArray(item?.variants) ? item.variants : []
  if (!variants.length) {
    return [item?.source === 'global' ? 'global' : 'proj']
  }
  return variants.map(v => (v?.source === 'global' ? 'global' : 'proj'))
}

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
  showTemplateMenu.value = false
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
  <div class="flex flex-col h-full" :class="{ 'select-none': isResizing }" @click="hideContextMenu">
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
          <!-- New: directly clears canvas -->
          <button class="btn btn-sm btn-ghost font-mono text-xs" @click="newWorkflow">New</button>
          <!-- Templates: separate dropdown -->
          <div class="relative">
            <button class="btn btn-sm btn-ghost font-mono text-xs" @click.stop="showTemplateMenu = !showTemplateMenu">Templates ▾</button>
            <ul
              v-if="showTemplateMenu"
              class="absolute top-full right-0 mt-1 z-50 min-w-[14rem] bg-base-300 border border-base-content/10 rounded-lg shadow-xl p-1.5 space-y-0.5"
            >
              <li v-for="tpl in templates" :key="tpl.name">
                <button
                  class="w-full text-left px-3 py-1.5 rounded hover:bg-base-content/10 transition-colors"
                  @click="loadTemplate(tpl.name)"
                >
                  <div class="font-mono text-xs text-base-content">{{ tpl.name }}</div>
                  <div class="text-[10px] text-base-content/40 mt-0.5">{{ tpl.description?.slice(0, 40) }}</div>
                </button>
              </li>
              <li v-if="!templates.length" class="px-3 py-2 text-[11px] text-base-content/30 font-mono">No templates</li>
            </ul>
          </div>
          <!-- Guide button -->
          <button
            class="btn btn-sm btn-ghost border border-base-content/15 font-mono text-xs px-2"
            title="Node type guide"
            @click="showNodeGuide = true"
          >? Guide</button>
          <button
            class="btn btn-sm btn-ghost border border-base-content/20 gap-1 font-mono text-xs"
            :class="{ 'loading loading-spinner': validating }"
            :disabled="!nodes.length || validating"
            @click="validateWorkflow"
          >
            <span v-if="!validating">⊙ Validate</span>
            <span v-else>Validating…</span>
          </button>
          <button class="btn btn-sm btn-primary font-mono text-xs" @click="openSaveDialog" :disabled="!nodes.length">Save</button>
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
      <!-- resizable sidebar -->
      <div
        class="bg-base-200 border-r border-base-300 flex flex-col shrink-0 overflow-auto relative"
        :style="{ width: sidebarWidth + 'px' }"
      >
        <div class="p-3 border-b border-base-300">
          <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-2">Workflows</div>
          <div class="space-y-0.5">
            <div v-for="w in workflows" :key="w.name" class="group/wf relative">
              <button
                class="w-full text-left px-2 py-2 rounded font-mono text-xs transition-colors"
                :class="selectedWorkflow === w.name
                  ? 'bg-primary/10 text-primary border-l-2 border-primary pl-1.5'
                  : 'hover:bg-base-300/50 text-base-content/80'"
                :title="w.name"
                @click="loadWorkflow(w.name)"
              >
                <!-- name: full, wraps to 2 lines -->
                <span class="block leading-snug break-all line-clamp-2 pr-8">{{ w.name }}</span>
                <!-- scope chips on second row -->
                <span class="flex items-center gap-1 mt-1">
                  <span
                    v-for="chip in workflowScopeChips(w)"
                    :key="`${w.name}:${chip}`"
                    class="text-[9px] font-mono uppercase tracking-wider px-1 py-0.5 rounded border border-base-content/20 text-base-content/50 bg-base-content/5"
                  >{{ chip }}</span>
                </span>
              </button>
              <!-- hover actions — top-right corner of the row -->
              <div class="absolute top-1.5 right-1 flex gap-0.5 opacity-0 group-hover/wf:opacity-100 transition-opacity">
                <button
                  v-if="w.source === 'project'"
                  class="btn btn-ghost btn-xs px-1 font-mono text-base-content/50 hover:text-warning"
                  :class="{ 'loading loading-spinner': promotingWorkflow === w.name }"
                  :disabled="promotingWorkflow === w.name"
                  title="Promote to global ~/.agent007/workflows/"
                  @click.stop="promoteWorkflow(w.name)"
                >↑</button>
                <button
                  class="btn btn-ghost btn-xs px-1 font-mono text-base-content/40 hover:text-error"
                  :class="{ 'loading loading-spinner': deletingWorkflow === w.name }"
                  :disabled="deletingWorkflow === w.name"
                  title="Delete workflow"
                  @click.stop="deleteWorkflow(w.name)"
                >✕</button>
              </div>
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

        <!-- Node Legend removed — accessible via "? Guide" button in toolbar -->

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

      <!-- drag handle -->
      <div
        class="w-1 shrink-0 cursor-col-resize hover:bg-primary/40 active:bg-primary/60 transition-colors z-10"
        :class="isResizing ? 'bg-primary/60' : 'bg-base-300'"
        @mousedown.prevent="startResize"
        title="Drag to resize"
      />

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

    <!-- Node Editor Modal — matches Agents/Skills popup style -->
    <dialog :open="showNodeEditor" class="modal" :class="{ 'modal-open': showNodeEditor }">
      <div class="modal-box max-w-2xl bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden">
        <!-- Header bar -->
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50 flex items-center gap-2">
            Edit Node ·
            <span v-if="editingNodeType === 'evaluator'" class="text-orange-400">Evaluator</span>
            <span v-else-if="editingNodeType === 'router'" class="text-purple-400">Router</span>
            <span v-else-if="editingNodeType === 'approval'" class="text-amber-400">Approval Gate</span>
            <span v-else-if="editingNodeType === 'orchestrator'" class="text-teal-400">Orchestrator</span>
            <span v-else class="text-primary">Agent</span>
          </span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="showNodeEditor = false">✕</button>
        </div>

        <!-- Body -->
        <div class="p-5 space-y-4 max-h-[70vh] overflow-y-auto">
          <!-- Agent + Output Key row -->
          <div class="grid grid-cols-2 gap-4">
            <div>
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">Agent / Persona</div>
              <select v-model="nodeEditorForm.agent" class="wf-input w-full">
                <option v-for="p in personas" :key="p.name" :value="p.name">{{ p.name }}</option>
                <option :value="nodeEditorForm.agent" v-if="nodeEditorForm.agent && !personas.find(p => p.name === nodeEditorForm.agent)">
                  {{ nodeEditorForm.agent }} (custom)
                </option>
              </select>
            </div>
            <div>
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">Output Key</div>
              <input v-model="nodeEditorForm.output" class="wf-input w-full" placeholder="step_output" />
            </div>
          </div>

          <!-- Prompt -->
          <div>
            <div class="flex items-center justify-between mb-1.5">
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest">Prompt</div>
              <div class="flex items-center gap-2">
                <span class="text-[10px] font-mono text-base-content/30">Use <code class="bg-base-300 px-1 rounded">&#123;&#123;task&#125;&#125;</code> and output keys from previous steps</span>
                <button
                  type="button"
                  class="px-2 py-0.5 text-[10px] font-mono rounded border border-primary/30 text-primary/70 hover:border-primary hover:text-primary transition-colors flex items-center gap-1"
                  @click="autogeneratePrompt"
                >⚡ Autogenerate</button>
              </div>
            </div>
            <textarea
              v-model="nodeEditorForm.prompt"
              class="w-full bg-base-200 border border-base-content/15 rounded text-[13px] font-mono text-base-content/80 p-3 h-36 resize-y focus:outline-none focus:border-primary/50 transition-colors leading-relaxed"
              placeholder="Describe what this agent should do. Use {{task}} for the workflow input."
            />
          </div>

          <!-- Resources: persona details + available skills -->
          <div v-if="editingNodeType === 'agent' || editingNodeType === 'evaluator' || editingNodeType === 'router'" class="space-y-2 rounded-lg border border-base-content/10 bg-base-200/50 p-4">
            <div class="text-[10px] font-mono font-bold text-base-content/40 uppercase tracking-wider mb-2">Resources</div>
            <!-- Persona info -->
            <template v-if="nodeEditorForm.agent">
              <div v-for="p in personas.filter(p => p.name === nodeEditorForm.agent)" :key="p.name" class="space-y-1.5">
                <div class="text-[10px] font-mono text-base-content/50">
                  <span class="text-base-content/30">Persona:</span> {{ p.name }}
                  <template v-if="p.preferred_model"> · <span class="text-accent/70">{{ p.preferred_model }}</span></template>
                </div>
                <div v-if="p.description" class="text-[10px] font-mono text-base-content/40 italic">{{ p.description }}</div>
                <div v-if="p.allowed_tools?.length" class="flex flex-wrap gap-1 mt-1">
                  <span class="text-[9px] font-mono text-base-content/30 mr-1">tools:</span>
                  <span v-for="t in p.allowed_tools" :key="t"
                    class="px-1.5 py-0.5 rounded text-[9px] font-mono bg-base-300 text-base-content/55">{{ t }}</span>
                </div>
              </div>
            </template>
            <div v-else class="text-[10px] font-mono text-base-content/30 italic">Select a persona above to see its details</div>
            <!-- Skills list -->
            <div v-if="skills.length" class="mt-2 pt-2 border-t border-base-content/10">
              <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-wider mb-1.5">Available Skills</div>
              <div class="flex flex-wrap gap-1">
                <span v-for="s in skills.slice(0, 20)" :key="s.trigger"
                  class="px-1.5 py-0.5 rounded text-[9px] font-mono"
                  :class="s.source === 'project' ? 'bg-primary/10 text-primary/70' : 'bg-base-300 text-base-content/50'"
                  :title="s.description || s.trigger">{{ s.trigger }}</span>
                <span v-if="skills.length > 20" class="text-[9px] font-mono text-base-content/30 italic">+{{ skills.length - 20 }} more</span>
              </div>
            </div>
          </div>

          <!-- Evaluator-specific config -->
          <div v-if="editingNodeType === 'evaluator'" class="space-y-3 border border-orange-500/30 rounded-lg p-4 bg-orange-500/5">
            <div class="text-[10px] font-mono font-bold text-orange-400 uppercase tracking-wider">Evaluator Config</div>
            <p class="text-[11px] font-mono text-base-content/40">The prompt must return JSON with a decision field. On <span class="text-green-400">pass</span> continues; otherwise retries.</p>
            <div class="grid grid-cols-3 gap-3">
              <div>
                <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">Decision Field</div>
                <input v-model="nodeEditorForm.evaluate.decision_field" class="wf-input w-full" placeholder="verdict" />
              </div>
              <div>
                <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">On Pass → step</div>
                <input v-model="nodeEditorForm.evaluate.on_pass" class="wf-input w-full" placeholder="deploy" />
              </div>
              <div>
                <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">On Fail → step</div>
                <input v-model="nodeEditorForm.evaluate.on_fail" class="wf-input w-full" placeholder="implement" />
              </div>
            </div>
            <div class="w-32">
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">Max Retries</div>
              <input v-model.number="nodeEditorForm.evaluate.max_retries" type="number" min="1" max="10" class="wf-input w-full" />
            </div>
          </div>

          <!-- Router-specific config -->
          <div v-if="editingNodeType === 'router'" class="space-y-3 border border-purple-500/30 rounded-lg p-4 bg-purple-500/5">
            <div class="flex items-center justify-between">
              <div class="text-[10px] font-mono font-bold text-purple-400 uppercase tracking-wider">Routes</div>
              <button class="px-2 py-0.5 text-[10px] font-mono rounded border border-purple-400/30 text-purple-400/70 hover:border-purple-400 hover:text-purple-400 transition-colors" @click="addRouteRow">+ Add Route</button>
            </div>
            <p class="text-[11px] font-mono text-base-content/40">The prompt must output a classification string. Each route matches on that string and jumps to the target step.</p>
            <div class="space-y-2">
              <div v-for="(route, i) in nodeEditorForm.routes" :key="i" class="flex gap-2 items-center">
                <input v-if="!route.default" v-model="route.when" class="wf-input flex-1" placeholder="frontend" />
                <span v-else class="text-[11px] font-mono text-base-content/40 italic flex-1">default</span>
                <span class="text-base-content/40 text-xs">→</span>
                <input v-model="route.goto" class="wf-input flex-1" placeholder="step-id" />
                <button class="btn btn-xs btn-ghost text-error px-1" @click="removeRouteRow(i)" :disabled="nodeEditorForm.routes.length <= 1">✕</button>
              </div>
            </div>
          </div>

          <!-- Approval note -->
          <div v-if="editingNodeType === 'approval'" class="border border-amber-500/30 rounded-lg p-4 bg-amber-500/5">
            <div class="text-[10px] font-mono font-bold text-amber-400 uppercase tracking-wider mb-2">Approval Gate</div>
            <p class="text-[11px] font-mono text-base-content/40">This node pauses the workflow and waits for a human decision via <code class="bg-base-300 px-1 rounded">POST /api/runs/&#123;id&#125;/approval</code> before continuing.</p>
          </div>

          <!-- Orchestrator config -->
          <div v-if="editingNodeType === 'orchestrator'" class="space-y-3 border border-teal-500/30 rounded-lg p-4 bg-teal-500/5">
            <div class="flex items-center justify-between">
              <div class="text-[10px] font-mono font-bold text-teal-400 uppercase tracking-wider">Worker Steps</div>
              <button class="px-2 py-0.5 text-[10px] font-mono rounded border border-teal-400/30 text-teal-400/70 hover:border-teal-400 hover:text-teal-400 transition-colors" @click="nodeEditorForm.workers = [...(nodeEditorForm.workers || []), '']">+ Add Worker</button>
            </div>
            <p class="text-[11px] font-mono text-base-content/40">Step IDs to fan-out to in parallel. Results are aggregated and passed to the next step.</p>
            <div class="space-y-2">
              <div v-for="(w, i) in (nodeEditorForm.workers || [])" :key="i" class="flex gap-2 items-center">
                <input :value="w" @input="nodeEditorForm.workers[i] = $event.target.value" class="wf-input flex-1" placeholder="worker-step-id" />
                <button class="btn btn-xs btn-ghost text-error px-1" @click="nodeEditorForm.workers.splice(i, 1)">✕</button>
              </div>
            </div>
          </div>
        </div>

        <!-- Footer -->
        <div class="flex items-center justify-end gap-2 px-5 py-3 bg-base-200 border-t border-base-300">
          <button class="btn btn-sm btn-ghost font-mono text-xs px-4" @click="showNodeEditor = false">Cancel</button>
          <button class="btn btn-sm btn-primary font-mono text-xs px-4" @click="saveNodeEdit">Apply</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showNodeEditor = false">close</button></form>
    </dialog>

    <!-- Save dialog -->
    <dialog :open="showSaveDialog" class="modal" :class="{ 'modal-open': showSaveDialog }">
      <div class="modal-box bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden max-w-sm">
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">Save Workflow</span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="showSaveDialog = false">✕</button>
        </div>
        <div class="p-5 space-y-4">
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">Name</div>
            <input v-model="workflowName" class="wf-input w-full" placeholder="my-workflow" />
          </div>
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">Description</div>
            <input v-model="workflowDescription" class="wf-input w-full" placeholder="What this workflow does" />
          </div>
        </div>
        <div class="flex items-center justify-end gap-2 px-5 py-3 bg-base-200 border-t border-base-300">
          <button class="btn btn-sm btn-ghost font-mono text-xs px-4" @click="showSaveDialog = false">Cancel</button>
          <button class="btn btn-sm btn-primary font-mono text-xs px-4" :disabled="!workflowName" @click="saveWorkflow">Save</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showSaveDialog = false">close</button></form>
    </dialog>

    <!-- Node Guide popup -->
    <dialog :open="showNodeGuide" class="modal" :class="{ 'modal-open': showNodeGuide }">
      <div class="modal-box max-w-lg bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden">
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">Node Guide</span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="showNodeGuide = false">✕</button>
        </div>
        <div class="p-5 space-y-4 max-h-[70vh] overflow-y-auto">
          <div class="space-y-4 text-[12px] font-mono text-base-content/60">
            <div class="border border-primary/20 rounded-lg p-3 bg-primary/3">
              <div class="font-bold text-primary mb-1.5">◉ Agent</div>
              <p class="leading-relaxed">Runs a single persona step. Connects to other nodes via <code class="bg-base-300 px-1 rounded text-base-content/80">depends_on</code> edges. Output is stored in the named output key variable and available to downstream steps.</p>
              <p class="mt-2 text-[11px] text-base-content/40">Example: <code class="bg-base-300 px-1 rounded">agent: Researcher, output: research_output</code></p>
            </div>
            <div class="border border-orange-500/20 rounded-lg p-3 bg-orange-500/3">
              <div class="font-bold text-orange-400 mb-1.5">↺ Evaluator</div>
              <p class="leading-relaxed">Runs a step and checks its JSON verdict. On <span class="text-green-400 font-bold">pass</span>, the workflow continues forward. On <span class="text-orange-400 font-bold">fail</span>, retries the target step up to <code class="bg-base-300 px-1 rounded text-base-content/80">max_retries</code> times.</p>
              <p class="mt-2 text-[11px] text-base-content/40">Prompt must return: <code class="bg-base-300 px-1 rounded">&#123;"verdict": "pass" | "retry", "reason": "..."&#125;</code></p>
            </div>
            <div class="border border-purple-500/20 rounded-lg p-3 bg-purple-500/3">
              <div class="font-bold text-purple-400 mb-1.5">⑂ Router</div>
              <p class="leading-relaxed">Classifies input and branches to a different step based on named <code class="bg-base-300 px-1 rounded text-base-content/80">routes</code>. Each route has a <code class="bg-base-300 px-1 rounded text-base-content/80">when</code> condition (matched against prompt output) and a <code class="bg-base-300 px-1 rounded text-base-content/80">goto</code> target step ID.</p>
              <p class="mt-2 text-[11px] text-base-content/40">Prompt must output exactly one of the route <code class="bg-base-300 px-1 rounded">when</code> labels.</p>
            </div>
            <div class="border border-amber-500/20 rounded-lg p-3 bg-amber-500/3">
              <div class="font-bold text-amber-400 mb-1.5">⏸ Approval Gate</div>
              <p class="leading-relaxed">Pauses the workflow and waits for a human to approve or deny before continuing. The run enters <code class="bg-base-300 px-1 rounded text-base-content/80">waiting_approval</code> state.</p>
              <p class="mt-2 text-[11px] text-base-content/40">Resume via: <code class="bg-base-300 px-1 rounded">POST /api/runs/&#123;id&#125;/approval &#123;"decision": "approve"&#125;</code></p>
            </div>
            <div class="border border-teal-500/20 rounded-lg p-3 bg-teal-500/3">
              <div class="font-bold text-teal-400 mb-1.5">⬡ Orchestrator</div>
              <p class="leading-relaxed">Fan-out coordinator that dispatches the same task to multiple named <code class="bg-base-300 px-1 rounded text-base-content/80">workers</code> in parallel and aggregates their results into a single output.</p>
              <p class="mt-2 text-[11px] text-base-content/40">Workers are step IDs in the same workflow that run concurrently.</p>
            </div>
          </div>
          <div class="pt-2 border-t border-base-300">
            <div class="text-[10px] font-mono text-base-content/30 uppercase tracking-wider mb-2">Tips</div>
            <ul class="text-[11px] font-mono text-base-content/50 space-y-1.5 list-none">
              <li>◦ Double-click any node to edit its prompt and config</li>
              <li>◦ Right-click a node or edge for context menu options</li>
              <li>◦ Delete / Backspace removes selected nodes or edges</li>
              <li>◦ Use <code class="bg-base-300 px-1 rounded">&#123;&#123;task&#125;&#125;</code> for the workflow input, <code class="bg-base-300 px-1 rounded">&#123;&#123;step_output&#125;&#125;</code> for prior step results</li>
            </ul>
          </div>
        </div>
        <div class="flex items-center justify-end px-5 py-3 bg-base-200 border-t border-base-300">
          <button class="btn btn-sm btn-primary font-mono text-xs px-4" @click="showNodeGuide = false">Got it</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showNodeGuide = false">close</button></form>
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

.wf-input {
  background: var(--color-base-300, var(--b3));
  border: 1px solid color-mix(in oklch, var(--color-base-content, var(--bc)) 20%, transparent);
  border-radius: 0.375rem;
  padding: 0.375rem 0.625rem;
  font-size: 0.8125rem;
  font-family: ui-monospace, 'Cascadia Code', monospace;
  color: var(--color-base-content, var(--bc));
  outline: none;
  transition: border-color 0.15s;
}
.wf-input:focus {
  border-color: color-mix(in oklch, var(--color-primary, var(--p)) 50%, transparent);
}
.wf-input:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
