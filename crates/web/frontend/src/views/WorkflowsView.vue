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

const { api } = useApi()
const workflows = ref([])
const personas = ref([])
const templates = ref([])
const selectedWorkflow = ref(null)
const showSaveDialog = ref(false)
const showTemplateMenu = ref(false)
const workflowName = ref('')
const workflowDescription = ref('')

const { onConnect, addEdges, getNodes, getEdges } = useVueFlow()

const nodes = ref([])
const edges = ref([])

const nodeTypes = {
  agent: AgentNode,
  evaluator: EvaluatorNode,
  router: RouterNode,
  approval: ApprovalNode,
}

const contextMenu = ref({ show: false, x: 0, y: 0, type: null, targetId: null })

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
    }

    return step
  })

  return {
    name: workflowName.value || selectedWorkflow.value || 'new-workflow',
    description: workflowDescription.value || '',
    steps,
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
    <div class="p-4 border-b border-base-300 bg-base-200 flex items-center justify-between">
      <h2 class="text-lg font-bold">Workflow Designer</h2>
      <div class="flex gap-2">
        <div class="dropdown dropdown-end">
          <label tabindex="0" class="btn btn-sm btn-ghost" @click.stop="showTemplateMenu = !showTemplateMenu">New ▾</label>
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
        <button class="btn btn-sm btn-primary" @click="openSaveDialog" :disabled="!nodes.length">Save</button>
      </div>
    </div>

    <div class="flex flex-1 overflow-hidden">
      <div class="w-56 bg-base-200 border-r border-base-300 flex flex-col shrink-0 overflow-auto">
        <div class="p-3 border-b border-base-300">
          <h3 class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Workflows</h3>
          <div class="space-y-1">
            <button
              v-for="w in workflows" :key="w"
              class="btn btn-ghost btn-xs justify-start w-full font-mono text-xs"
              :class="{ 'btn-active': selectedWorkflow === w }"
              @click="loadWorkflow(w)"
            >{{ w }}</button>
          </div>
          <div v-if="!workflows.length" class="text-xs text-base-content/40">No workflows</div>
        </div>

        <div class="p-3 border-b border-base-300">
          <h3 class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Special Nodes</h3>
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
          </div>
        </div>

        <div class="p-3">
          <h3 class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Agent Palette</h3>
          <p class="text-xs text-base-content/40 mb-2">Click to add a node</p>
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
        class="w-full text-left px-3 py-1.5 text-sm hover:bg-primary/10 text-base-content"
        @click="editFromContextMenu"
      >
        ✏️ Edit Node
      </button>
      <div v-if="contextMenu.type === 'node'" class="divider my-0.5 h-px bg-base-content/10 mx-2"></div>
      <button class="w-full text-left px-3 py-1.5 text-sm hover:bg-error/20 text-error" @click="deleteFromContextMenu">
        🗑 Delete {{ contextMenu.type === 'node' ? 'Node' : 'Edge' }}
      </button>
    </div>

    <!-- Node Editor Modal -->
    <dialog :open="showNodeEditor" class="modal" :class="{ 'modal-open': showNodeEditor }">
      <div class="modal-box max-w-2xl bg-base-200">
        <h3 class="font-bold text-lg">
          Edit Node
          <span class="text-sm font-normal text-base-content/50 ml-2">
            <span v-if="editingNodeType === 'evaluator'" class="text-orange-400">evaluator</span>
            <span v-else-if="editingNodeType === 'router'" class="text-purple-400">router</span>
            <span v-else-if="editingNodeType === 'approval'" class="text-amber-400">approval gate</span>
            <span v-else class="text-primary">agent</span>
          </span>
        </h3>

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
              <span class="label-text-alt text-xs text-base-content/40">Use <code class="font-mono bg-base-300 px-1 rounded">&#123;&#123;task&#125;&#125;</code> and output keys from previous steps</span>
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
        </div>

        <div class="modal-action">
          <button class="btn btn-sm btn-ghost" @click="showNodeEditor = false">Cancel</button>
          <button class="btn btn-sm btn-primary" @click="saveNodeEdit">Apply Changes</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showNodeEditor = false">close</button></form>
    </dialog>

    <!-- Save dialog -->
    <dialog :open="showSaveDialog" class="modal" :class="{ 'modal-open': showSaveDialog }">
      <div class="modal-box bg-base-200">
        <h3 class="font-bold text-lg">Save Workflow</h3>
        <div class="mt-4 space-y-3">
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Name</span></label>
            <input v-model="workflowName" class="input input-sm input-bordered font-mono" placeholder="my-workflow" />
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Description</span></label>
            <input v-model="workflowDescription" class="input input-sm input-bordered" />
          </div>
        </div>
        <div class="modal-action">
          <button class="btn btn-sm btn-ghost" @click="showSaveDialog = false">Cancel</button>
          <button class="btn btn-sm btn-primary" @click="saveWorkflow">Save</button>
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
