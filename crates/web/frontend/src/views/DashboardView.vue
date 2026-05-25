<script setup>
import { ref, onMounted, onUnmounted, computed, watch, nextTick } from 'vue'
import { useApi } from '../composables/useApi.js'
import { marked } from 'marked'
import DOMPurify from 'dompurify'

marked.setOptions({ gfm: true, breaks: true })

const props = defineProps({ events: Array, connected: Boolean, stats: Object })
const { api } = useApi()

const health = ref(null)
const metrics = ref(null)
const taskInput = ref('')
const taskStatus = ref('')
const runs = ref([])
const runtimeSessions = ref(null)
const providerStatus = ref(null)
const selectedRun = ref(null)
const selectedRunId = ref(null)
const selectedArtifactPath = ref('')
const selectedArtifactPreview = ref(null)
const artifactPreviewStatus = ref('')
const artifactPreviewRef = ref(null)
const runtimeMessages = ref([])
const runtimeMessageText = ref('')
const runtimeMessageKind = ref('note')
const runtimeMessageTo = ref('')
const runtimeMessageStatus = ref('')
const runtimeMessageBusy = ref(false)
const approvalStatus = ref('')
const approvalEditContent = ref('')
const resumeStatus = ref('')
const cleanupStatus = ref('')
const taskPanelOpen = ref(false)
const expandedRunId = ref(null)
const chatMessages = ref([])
const chatPending = ref(false)
const slashCommands = ref([])
const slashCommandsLoaded = ref(false)
const showSlashMenu = ref(false)
const slashFilter = ref('')
const slashMenuIndex = ref(0)
const slashMenuRef = ref(null)
const graphExplorerMode = ref('usage')
const graphExplorerSymbol = ref('')
const graphExplorerFrom = ref('')
const graphExplorerTo = ref('')
const graphExplorerQuery = ref('')
const graphExplorerDepth = ref(2)
const graphExplorerPaths = ref('')
const graphExplorerBusy = ref(false)
const graphExplorerStatus = ref('')
const graphExplorerResult = ref(null)
const graphExplorerMermaidRef = ref(null)

// ETR cache stats
const etrCacheStats = ref(null)

// Run list filter
const runFilter = ref('')
const runStatusFilter = ref('')

watch(slashMenuIndex, (idx) => {
  nextTick(() => {
    const items = slashMenuRef.value?.querySelectorAll('button[data-slash-item]')
    items?.[idx]?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  })
})

const filteredSlashCommands = computed(() => {
  const q = slashFilter.value.toLowerCase()
  const all = slashCommands.value
  if (!q) return all.slice(0, 12)
  return all.filter(c =>
    c.trigger.toLowerCase().includes(q) ||
    c.name?.toLowerCase().includes(q) ||
    c.description?.toLowerCase().includes(q)
  ).slice(0, 12)
})

async function loadSlashCommands() {
  if (slashCommandsLoaded.value) return
  slashCommandsLoaded.value = true
  try {
    const [skills, workflows] = await Promise.all([
      api.listSkills(),
      api.listWorkflows(),
    ])
    const cmds = []
    for (const s of skills || []) {
      if (s.trigger) cmds.push({
        type: 'skill',
        trigger: s.trigger,
        name: s.name || s.trigger,
        description: s.description || '',
        source: s.source || 'global',
      })
    }
    for (const w of workflows || []) {
      cmds.push({
        type: 'workflow',
        trigger: `/workflow:${w.name}`,
        name: w.name,
        description: 'Run workflow',
        source: w.source || 'global',
      })
    }
    slashCommands.value = cmds.sort((a, b) => a.trigger.localeCompare(b.trigger))
  } catch { slashCommandsLoaded.value = false }
}

function onChatInput(e) {
  const val = taskInput.value
  const ta = e.target
  ta.style.height = 'auto'
  ta.style.height = Math.min(ta.scrollHeight, 144) + 'px'
  const m = val.match(/^\/(\S*)$/)
  if (m !== null) {
    slashFilter.value = m[1]
    showSlashMenu.value = true
    slashMenuIndex.value = 0
    loadSlashCommands()
  } else {
    showSlashMenu.value = false
  }
}

function onChatKeydown(e) {
  if (showSlashMenu.value) {
    if (e.key === 'ArrowDown') { e.preventDefault(); slashMenuIndex.value = Math.min(slashMenuIndex.value + 1, filteredSlashCommands.value.length - 1) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); slashMenuIndex.value = Math.max(slashMenuIndex.value - 1, 0) }
    else if (e.key === 'Escape') { e.preventDefault(); showSlashMenu.value = false }
    else if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
      const cmd = filteredSlashCommands.value[slashMenuIndex.value]
      if (cmd) { e.preventDefault(); selectSlashCommand(cmd) }
      else if (e.key === 'Enter') submitTask()
    }
    return
  }
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submitTask() }
}

function selectSlashCommand(cmd) {
  taskInput.value = cmd.trigger + ' '
  showSlashMenu.value = false
  slashFilter.value = ''
}
let refreshTimer = null
let detailTimer = null

onMounted(async () => {
  try { health.value = await api.health() } catch {}
  await refreshDashboardSnapshots()
  loadEtrCacheStats()
  refreshTimer = setInterval(refreshDashboardSnapshots, 5000)
  detailTimer = setInterval(async () => {
    if (expandedRunId.value) {
      try { selectedRun.value = await api.getRunDetail(expandedRunId.value) } catch { }
    }
  }, 1500)
})

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
  if (detailTimer) clearInterval(detailTimer)
})

watch(() => props.stats, (v) => {
  if (v) metrics.value = v
})

watch(() => props.events?.length, async () => {
  if (expandedRunId.value) {
    try { selectedRun.value = await api.getRunDetail(expandedRunId.value) } catch { }
  }
})

watch(graphExplorerMermaidSource, async () => {
  await renderGraphExplorerMermaid()
})

const providerReadiness = computed(() => providerStatus.value || {
  runtime_mode: m.value.runtime_mode || 'hosted-mcp',
  selected_provider: null,
  selected_model: m.value.model_provider || null,
  standalone_available: !!m.value.local_execution_available,
  providers: [],
  hints: [],
})

const providerCards = computed(() => providerReadiness.value.providers || [])

const runtime = computed(() => runtimeSessions.value || {
  generated_at: null,
  counts: { total: 0, active: 0, running: 0, blocked: 0, failed: 0, succeeded: 0 },
  sessions: [],
})

const runtimeFocusSessions = computed(() => {
  const sessions = runtime.value.sessions || []
  const active = sessions.filter(s => ['running', 'ready', 'blocked', 'attention'].includes(s.lifecycle))
  if (active.length) return active.slice(0, 6)
  return sessions.slice(0, 6)
})

const m = computed(() => metrics.value || {
  active_agents: 0, running_tasks: 0, awaiting_approvals: 0, completed_tasks: 0, failed_tasks: 0,
  total_tokens: 0, estimated_usd: 0, avg_reward: 0, session_requests: 0,
  scorecard_run_count: 0, success_rate: 0, avg_cost_usd: 0, avg_latency_ms: 0, total_retries: 0, avg_retries_per_run: 0,
  feedback_count: 0, prompt_improvements: 0,
  skills_count: 0, workflows_count: 0, personas_count: 0, memory_keys: 0,
  started_at: null, local_execution_available: false, runtime_mode: 'hosted-mcp', model_provider: 'unknown',
  repo_graph: null,
  recent_tasks: [], recent_scorecards: [],
})

function repoGraphState(graph) {
  if (!graph?.exists) return 'missing'
  if (graph?.stale) return 'stale'
  return 'ready'
}

function repoGraphBadgeClass(graph) {
  const state = repoGraphState(graph)
  return {
    'badge-error': state === 'missing',
    'badge-warning': state === 'stale',
    'badge-success': state === 'ready',
  }
}

const uptime = computed(() => {
  if (!m.value.started_at) return '—'
  const diff = Math.floor((Date.now() - new Date(m.value.started_at).getTime()) / 1000)
  if (diff < 60) return `${diff}s`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ${diff % 60}s`
  return `${Math.floor(diff / 3600)}h ${Math.floor((diff % 3600) / 60)}m`
})

function fmtTokens(n) {
  if (n == null) return '—'
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

function fmtBytes(n) {
  const bytes = Number(n || 0)
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${bytes} B`
}

function fmtAgeSeconds(seconds) {
  const secs = Number(seconds || 0)
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`
}

function runtimeLifecycleClass(lifecycle) {
  return {
    'border-info/50 bg-info/5': lifecycle === 'running',
    'border-primary/50 bg-primary/5': lifecycle === 'ready',
    'border-warning/60 bg-warning/10': lifecycle === 'blocked' || lifecycle === 'attention',
    'border-success/40 bg-success/5': lifecycle === 'complete',
    'border-error/50 bg-error/5': lifecycle === 'failed',
  }
}

function runtimeBadgeClass(lifecycle) {
  return {
    'badge-info': lifecycle === 'running',
    'badge-primary': lifecycle === 'ready',
    'badge-warning': lifecycle === 'blocked' || lifecycle === 'attention',
    'badge-success': lifecycle === 'complete',
    'badge-error': lifecycle === 'failed',
    'badge-ghost': !['running', 'ready', 'blocked', 'attention', 'complete', 'failed'].includes(lifecycle),
  }
}

function sessionBorderColor(lifecycle) {
  if (lifecycle === 'running') return 'border-l-info'
  if (lifecycle === 'ready') return 'border-l-primary'
  if (lifecycle === 'blocked' || lifecycle === 'attention') return 'border-l-warning'
  if (lifecycle === 'complete') return 'border-l-success'
  if (lifecycle === 'failed') return 'border-l-error'
  return 'border-l-base-content/20'
}

function workflowProgressColor(lifecycle) {
  if (lifecycle === 'running') return 'bg-info'
  if (lifecycle === 'blocked' || lifecycle === 'attention') return 'bg-warning'
  if (lifecycle === 'complete') return 'bg-success'
  if (lifecycle === 'failed') return 'bg-error'
  return 'bg-primary'
}

function providerCardClass(status) {
  return {
    'border-success/45 bg-success/5': status === 'ready',
    'border-warning/45 bg-warning/5': status === 'fallback' || status === 'unreachable',
    'border-base-content/15 bg-base-200': status === 'needs-config' || status === 'not-configured',
    'border-error/45 bg-error/5': status === 'error',
  }
}

function providerBadgeClass(status) {
  return {
    'badge-success': status === 'ready',
    'badge-warning': status === 'fallback' || status === 'unreachable',
    'badge-ghost': status === 'needs-config' || status === 'not-configured',
    'badge-error': status === 'error',
  }
}

function fmtMs(ms) {
  if (!ms) return '0ms'
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)}m`
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`
  return `${Math.round(ms)}ms`
}

function fmtLocalTime(str) {
  if (!str) return '—'
  try {
    return new Date(str).toLocaleString(undefined, {
      month: 'short', day: 'numeric',
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    })
  } catch { return str }
}

function stepDuration(step) {
  if (!step.started_at) return null
  const start = new Date(step.started_at).getTime()
  const end = step.finished_at ? new Date(step.finished_at).getTime() : Date.now()
  const ms = end - start
  if (ms < 0) return null
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  if (ms < 3600000) return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`
  return `${Math.floor(ms / 3600000)}h ${Math.floor((ms % 3600000) / 60000)}m`
}

const SLOW_STEP_MS = 60_000

function isSlowStep(step) {
  if (!step.started_at || !step.finished_at) return false
  return new Date(step.finished_at).getTime() - new Date(step.started_at).getTime() > SLOW_STEP_MS
}

const expandedSteps = ref(new Set())
const rawStepView = ref(new Set())

function renderMarkdown(raw) {
  if (!raw) return ''
  const dirty = marked.parse(raw)
  return DOMPurify.sanitize(dirty, { ADD_TAGS: ['pre', 'code'], ADD_ATTR: ['class'] })
}

let mermaidInstance = null
async function ensureMermaid() {
  if (mermaidInstance) return mermaidInstance
  const mermaid = (await import('mermaid')).default
  mermaid.initialize({
    startOnLoad: false,
    theme: 'dark',
    securityLevel: 'strict',
    themeVariables: {
      background: '#1d232a',
      primaryColor: '#7480ff',
      primaryTextColor: '#a6adbb',
      primaryBorderColor: '#2a323c',
      lineColor: '#4b5563',
      secondaryColor: '#2a323c',
      tertiaryColor: '#191e24',
      edgeLabelBackground: '#1d232a',
      fontFamily: 'ui-monospace, monospace',
    },
  })
  mermaidInstance = mermaid
  return mermaid
}

async function renderArtifactMermaid() {
  const kind = selectedArtifactPreview.value?.kind
  if (!['markdown', 'mermaid'].includes(kind)) return
  await nextTick()
  const root = artifactPreviewRef.value
  if (!root) return

  root.querySelectorAll('.language-mermaid').forEach(block => {
    const source = block.textContent || ''
    const container = block.closest('pre') || block
    const target = document.createElement('div')
    target.className = 'artifact-mermaid-block'
    target.textContent = source
    container.replaceWith(target)
  })

  const blocks = root.querySelectorAll('.artifact-mermaid-block')
  if (!blocks.length) return
  try {
    const mermaid = await ensureMermaid()
    await mermaid.run({ nodes: Array.from(blocks) })
  } catch (error) {
    console.warn('artifact mermaid render error:', error)
  }
}

const graphExplorerMermaidSource = computed(() => {
  const payload = graphExplorerResult.value?.output
  const tool = graphExplorerResult.value?.tool
  if (!payload || !tool) return ''

  const esc = (value) => String(value || '').replace(/"/g, '\\"')

  if (tool === 'etr.usage_graph' || tool === 'etr.impact_radius') {
    const nodes = payload.nodes || []
    const edges = payload.edges || []
    if (!nodes.length) return ''
    const ids = new Map(nodes.map((node, index) => [node.id, `N${index + 1}`]))
    const lines = ['graph TD']
    for (const node of nodes) {
      const nodeId = ids.get(node.id)
      const label = `${node.name}${node.path ? `\\n${node.path}` : ''}`
      lines.push(`  ${nodeId}["${esc(label)}"]`)
    }
    for (const edge of edges) {
      const from = ids.get(edge.from)
      const to = ids.get(edge.to)
      if (!from || !to) continue
      lines.push(`  ${from} -->|${esc(edge.kind)}| ${to}`)
    }
    return lines.join('\n')
  }

  if (tool === 'etr.callers') {
    const rows = payload.callers || []
    if (!rows.length) return ''
    return ['graph TD', ...rows.map((row, index) =>
      `  C${index}["${esc(row.caller)}"] --> T${index}["${esc(row.callee)}"]`
    )].join('\n')
  }

  if (tool === 'etr.callees') {
    const rows = payload.callees || []
    if (!rows.length) return ''
    return ['graph TD', ...rows.map((row, index) =>
      `  C${index}["${esc(row.caller)}"] --> T${index}["${esc(row.callee)}"]`
    )].join('\n')
  }

  if (tool === 'etr.doc_links') {
    const rows = payload.docs || []
    if (!rows.length) return ''
    return ['graph TD', ...rows.map((row, index) =>
      `  D${index}["${esc(row.doc)}"] -.-> S${index}["${esc(row.symbol)}"]`
    )].join('\n')
  }

  if (tool === 'etr.dep_path') {
    const steps = payload.steps || []
    if (!steps.length) return ''
    const lines = ['graph TD']
    steps.forEach((step, index) => {
      lines.push(`  P${index}["${esc(step.from_name)}"] -->|${esc(step.edge_kind)}| P${index + 1}["${esc(step.to_name)}"]`)
    })
    return lines.join('\n')
  }

  if (tool === 'etr.context_bundle') {
    const symbols = payload.matched_symbols || []
    const docs = payload.related_docs || []
    if (!symbols.length && !docs.length) return ''
    const lines = ['graph TD']
    symbols.forEach((node, index) => {
      lines.push(`  S${index}["${esc(node.name)}"]`)
    })
    docs.forEach((doc, index) => {
      lines.push(`  D${index}["${esc(doc.name)}"]`)
      if (symbols.length) lines.push(`  D${index} -.-> S0`)
    })
    return lines.join('\n')
  }

  return ''
})

async function renderGraphExplorerMermaid() {
  await nextTick()
  const root = graphExplorerMermaidRef.value
  const source = graphExplorerMermaidSource.value
  if (!root) return
  root.innerHTML = ''
  if (!source) return
  const target = document.createElement('div')
  target.className = 'artifact-mermaid-block'
  target.textContent = source
  root.appendChild(target)
  try {
    const mermaid = await ensureMermaid()
    await mermaid.run({ nodes: [target] })
  } catch (error) {
    console.warn('graph explorer mermaid render error:', error)
  }
}

function toggleRawStep(stepId) {
  const next = new Set(rawStepView.value)
  if (next.has(stepId)) next.delete(stepId)
  else next.add(stepId)
  rawStepView.value = next
}

function toggleStepExpand(stepId) {
  const next = new Set(expandedSteps.value)
  if (next.has(stepId)) next.delete(stepId)
  else next.add(stepId)
  expandedSteps.value = next
}

function stepFullOutput(step) {
  if (step.output_key && selectedWorkflowState.value?.outputs?.[step.output_key]) {
    return selectedWorkflowState.value.outputs[step.output_key]
  }
  return step.output_preview || ''
}

const STALE_SECS = 600 // 10 minutes

function isStaleStep(step) {
  if (!step.last_heartbeat_at) return false
  const age = (Date.now() - new Date(step.last_heartbeat_at).getTime()) / 1000
  return age > STALE_SECS
}

function heartbeatAge(step) {
  if (!step.last_heartbeat_at) return ''
  const secs = Math.floor((Date.now() - new Date(step.last_heartbeat_at).getTime()) / 1000)
  if (secs < 60) return `${secs}s ago`
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m ago`
}

const recentEvents = computed(() => [...(props.events || [])].reverse().slice(0, 50))
const selectedWorkflowState = computed(() => selectedRun.value?.workflow_state || null)
const selectedEvalGateDecision = computed(() => selectedWorkflowState.value?.eval_gate_decision || null)
const selectedRoutingRecommendations = computed(() => selectedWorkflowState.value?.routing_recommendations || [])
const pendingApproval = computed(() => selectedWorkflowState.value?.pending_approval || null)
const selectedRunOutput = computed(() => selectedRun.value?.output_text || '')
const selectedRunKind = computed(() => selectedRun.value?.run?.metadata?.kind || '')
const selectedRetrievalTelemetry = computed(() => selectedRun.value?.retrieval_telemetry || null)
const selectedPersonaPolicyWarning = computed(() => selectedRun.value?.persona_policy_warning || null)
const selectedRunTokenSummary = computed(() => selectedRun.value?.token_summary || null)
const dashboardOwnsSelectedWorkflow = computed(() => selectedRunKind.value.startsWith('workflow-web-'))
const selectedRunArtifacts = computed(() => selectedRun.value?.run?.artifacts || [])
const selectedArtifactIsRenderable = computed(() => {
  const kind = selectedArtifactPreview.value?.kind
  return ['markdown', 'html', 'json', 'mermaid', 'text', 'image'].includes(kind)
})
const selectedArtifactRawUrl = computed(() => selectedArtifactPreview.value?.raw_url || '')
const selectedRunResumeTargetStatus = computed(() => selectedRun.value?.resume_target_status || null)
const selectedRunAlreadyResumed = computed(() => {
  if (!selectedRunArtifacts.value.includes('resume-target.json')) return false
  return selectedRunResumeTargetStatus.value !== 'failed'
})
const selectedRunHasApprovalDecision = computed(() => {
  const decisions = selectedWorkflowState.value?.approval_decisions || {}
  return Object.keys(decisions).length > 0
})
const canResumeSelectedRun = computed(() => {
  const run = selectedRun.value?.run?.metadata
  if (!run || !selectedWorkflowState.value || pendingApproval.value) return false
  if (!dashboardOwnsSelectedWorkflow.value) return false
  if (selectedRunAlreadyResumed.value) return false
  if (!selectedRunHasApprovalDecision.value) return false
  return run.status === 'awaiting-approval' || selectedWorkflowState.value.status === 'running'
})
const externalWorkflowControlNotice = computed(() => {
  const run = selectedRun.value?.run?.metadata
  if (!selectedWorkflowState.value || dashboardOwnsSelectedWorkflow.value) return ''
  if (pendingApproval.value) {
    return 'This workflow is waiting for approval in the client that started it. Review and approve there; the dashboard is read-only for external workflow runs.'
  }
  if (
    selectedRunHasApprovalDecision.value &&
    run &&
    (run.status === 'awaiting-approval' || selectedWorkflowState.value.status === 'running')
  ) {
    return 'Approval has been recorded, but continuation belongs to the client that started this workflow. Continue there; the dashboard is read-only for external workflow runs.'
  }
  return ''
})

function fmtReasonCodes(codes) {
  if (!Array.isArray(codes) || !codes.length) return '—'
  return codes.join(', ')
}

function fmtConfidence(value) {
  if (typeof value !== 'number') return '0%'
  return `${Math.round(value * 100)}%`
}

function fmtPct(value) {
  if (typeof value !== 'number') return '0%'
  return `${(value * 100).toFixed(1)}%`
}

// Sparkline: build SVG polyline path from an array of numbers
function sparklinePath(values, width = 80, height = 24) {
  if (!values || values.length < 2) return ''
  const max = Math.max(...values) || 1
  const step = width / (values.length - 1)
  const pts = values.map((v, i) => `${(i * step).toFixed(1)},${(height - (v / max) * height).toFixed(1)}`)
  return pts.join(' ')
}

// Last N token counts from recent_tasks
const tokenSparkline = computed(() => {
  const tasks = m.value.recent_tasks || []
  return tasks.slice(-12).map(t => t.tokens || 0)
})

// Filtered runs for the persisted runs list
const filteredRuns = computed(() => {
  let list = runs.value
  if (runStatusFilter.value) list = list.filter(r => r.status === runStatusFilter.value)
  if (runFilter.value.trim()) {
    const q = runFilter.value.toLowerCase()
    list = list.filter(r =>
      (r.task || '').toLowerCase().includes(q) ||
      (r.kind || '').toLowerCase().includes(q) ||
      (r.id || '').toLowerCase().includes(q)
    )
  }
  return list
})

async function loadEtrCacheStats() {
  try { etrCacheStats.value = await api.etrCacheStats() } catch {}
}

async function clearEtrCache() {
  await api.etrCacheClear()
  loadEtrCacheStats()
}

async function refreshRuntimeSessions() {
  runtimeSessions.value = await api.getRuntimeSessions(12) || runtimeSessions.value
}

async function refreshDashboardSnapshots() {
  const results = await Promise.allSettled([
    api.getStats(),
    refreshRuns(),
    refreshRuntimeSessions(),
    api.getProviderStatus(),
  ])
  if (results[0].status === 'fulfilled' && results[0].value) {
    metrics.value = results[0].value
  }
  if (results[3].status === 'fulfilled' && results[3].value) {
    providerStatus.value = results[3].value
  }
}

function splitGraphPaths(raw) {
  return (raw || '')
    .split(/\r?\n|,/)
    .map(v => v.trim())
    .filter(Boolean)
}

async function runGraphExplorer(toolOverride = null) {
  graphExplorerBusy.value = true
  graphExplorerStatus.value = ''
  try {
    let tool = toolOverride
    let input = {}
    if (!tool) {
      switch (graphExplorerMode.value) {
        case 'usage':
          tool = 'etr.usage_graph'
          input = { symbol: graphExplorerSymbol.value, exact: true, max_depth: Number(graphExplorerDepth.value || 1) }
          break
        case 'impact':
          tool = 'etr.impact_radius'
          input = { symbol: graphExplorerSymbol.value, exact: true, max_depth: Number(graphExplorerDepth.value || 2) }
          break
        case 'callers':
          tool = 'etr.callers'
          input = { symbol: graphExplorerSymbol.value, exact: true }
          break
        case 'callees':
          tool = 'etr.callees'
          input = { symbol: graphExplorerSymbol.value, exact: true }
          break
        case 'docs':
          tool = 'etr.doc_links'
          input = { symbol: graphExplorerSymbol.value, exact: true }
          break
        case 'path':
          tool = 'etr.dep_path'
          input = { from_symbol: graphExplorerFrom.value, to_symbol: graphExplorerTo.value, exact: true }
          break
        case 'context':
          tool = 'etr.context_bundle'
          input = {
            query: graphExplorerQuery.value,
            max_symbols: 6,
            max_neighbors: Math.max(1, Number(graphExplorerDepth.value || 2)),
          }
          break
        default:
          throw new Error('unknown graph explorer mode')
      }
    } else if (tool === 'etr.graph_refresh_paths') {
      input = { paths: splitGraphPaths(graphExplorerPaths.value) }
    }

    const output = await api.etrCall(tool, input, false)
    graphExplorerResult.value = { tool, input, output }
    graphExplorerStatus.value = `${tool} ok`
    if (tool.startsWith('etr.graph_')) {
      await refreshDashboardSnapshots()
    }
  } catch (error) {
    graphExplorerStatus.value = error?.message || String(error)
  } finally {
    graphExplorerBusy.value = false
  }
}

async function refreshRuns() {
  runs.value = await api.listRuns() || []
  if (expandedRunId.value) {
    if (runs.value.find(r => r.id === expandedRunId.value)) {
      selectedRun.value = await api.getRunDetail(expandedRunId.value)
    } else {
      expandedRunId.value = null
      selectedRunId.value = null
      selectedRun.value = null
      clearRuntimeMessages()
    }
  }
}

async function toggleRun(id) {
  if (expandedRunId.value === id) {
    expandedRunId.value = null
    selectedRunId.value = null
    selectedRun.value = null
    clearRuntimeMessages()
    clearArtifactPreview()
    approvalStatus.value = ''
    resumeStatus.value = ''
  } else {
    expandedRunId.value = id
    await selectRun(id)
  }
}

async function selectRun(id) {
  selectedRunId.value = id
  selectedRun.value = await api.getRunDetail(id)
  await loadRuntimeMessages(id)
  approvalEditContent.value = selectedRun.value?.workflow_state?.pending_approval?.content || ''
  resumeStatus.value = ''
  clearArtifactPreview()
}

function clearRuntimeMessages() {
  runtimeMessages.value = []
  runtimeMessageText.value = ''
  runtimeMessageKind.value = 'note'
  runtimeMessageTo.value = ''
  runtimeMessageStatus.value = ''
  runtimeMessageBusy.value = false
}

async function loadRuntimeMessages(id = selectedRunId.value) {
  if (!id) {
    runtimeMessages.value = []
    return
  }
  try {
    const result = await api.getRuntimeMessages(id)
    runtimeMessages.value = Array.isArray(result?.messages) ? result.messages : []
  } catch {
    runtimeMessages.value = []
  }
}

async function postRuntimeMessage() {
  if (!selectedRunId.value || !runtimeMessageText.value.trim() || runtimeMessageBusy.value) return
  runtimeMessageBusy.value = true
  runtimeMessageStatus.value = 'Saving note...'
  try {
    await api.postRuntimeMessage(selectedRunId.value, {
      from: 'operator',
      to: runtimeMessageTo.value.trim() || null,
      kind: runtimeMessageKind.value,
      body: runtimeMessageText.value.trim(),
    })
    runtimeMessageText.value = ''
    runtimeMessageTo.value = ''
    runtimeMessageStatus.value = ''
    await loadRuntimeMessages(selectedRunId.value)
    selectedRun.value = await api.getRunDetail(selectedRunId.value)
  } catch (e) {
    runtimeMessageStatus.value = `Error: ${e.message}`
  } finally {
    runtimeMessageBusy.value = false
  }
}

function clearArtifactPreview() {
  selectedArtifactPath.value = ''
  selectedArtifactPreview.value = null
  artifactPreviewStatus.value = ''
}

async function previewArtifact(path) {
  if (!selectedRunId.value || !path) return
  selectedArtifactPath.value = path
  artifactPreviewStatus.value = 'Loading artifact preview...'
  try {
    selectedArtifactPreview.value = await api.previewRunArtifact(selectedRunId.value, path)
    artifactPreviewStatus.value = ''
    await renderArtifactMermaid()
  } catch (error) {
    selectedArtifactPreview.value = null
    artifactPreviewStatus.value = error?.message || 'Unable to load artifact preview'
  }
}

async function recordApproval(decision) {
  if (!selectedRunId.value || !pendingApproval.value || !dashboardOwnsSelectedWorkflow.value) return
  approvalStatus.value = 'Saving approval...'
  try {
    await api.approveRunStep(selectedRunId.value, {
      step: pendingApproval.value.step_id,
      decision,
      content: decision === 'edit' ? approvalEditContent.value : undefined,
    })
    const isHostedMcp = m.value.runtime_mode === 'hosted-mcp'
    approvalStatus.value = isHostedMcp
      ? `✓ Approval recorded (${decision}). Your AI assistant will continue automatically — ask it to call agent007_workflow_next.`
      : `Recorded ${decision}. Resume the workflow to continue.`
    await selectRun(selectedRunId.value)
    await refreshRuns()
  } catch (e) {
    approvalStatus.value = `Error: ${e.message}`
  }
}

async function resumeSelectedRun() {
  if (!selectedRunId.value || !canResumeSelectedRun.value) return
  resumeStatus.value = 'Resuming workflow...'
  try {
    const response = await api.resumeRun(selectedRunId.value)
    approvalStatus.value = ''
    await refreshRuns()
    if (response?.session) {
      expandedRunId.value = response.session
      await selectRun(response.session)
    } else {
      await selectRun(selectedRunId.value)
    }
    if (response?.already_resumed) {
      resumeStatus.value = `This workflow was already resumed as ${response.session}.`
    } else if (response?.status === 'awaiting-approval') {
      resumeStatus.value = `Workflow resumed and paused again for approval on step ${response.step}.`
    } else {
      resumeStatus.value = 'Workflow resumed successfully.'
    }
  } catch (e) {
    resumeStatus.value = `Error: ${e.message}`
  }
}

async function cleanupStaleApprovals() {
  cleanupStatus.value = 'Cleaning stale approval runs...'
  try {
    const result = await api.cleanupAwaitingRuns({
      older_than_hours: 24 * 7,
      limit: 1000,
      include_dashboard_owned: false,
      dry_run: false,
    })
    const cleaned = result?.cleaned || 0
    const matched = result?.matched || 0
    cleanupStatus.value = `Closed ${cleaned} stale run(s) out of ${matched} matched.`
    await refreshRuns()
    metrics.value = await api.getStats() || metrics.value
  } catch (e) {
    cleanupStatus.value = `Error: ${e.message}`
  }
}

async function submitTask() {
  const input = taskInput.value.trim()
  if (!input || chatPending.value) return

  chatMessages.value.push({ role: 'user', content: input })
  chatMessages.value.push({ role: 'assistant', content: '', status: 'running', sessionId: null })
  const replyIdx = chatMessages.value.length - 1

  taskInput.value = ''
  chatPending.value = true

  try {
    const response = await api.runTask(input)
    const sessionId = response?.session || null

    let output = ''
    if (sessionId) {
      expandedRunId.value = sessionId
      await selectRun(sessionId)
      output = selectedRun.value?.output_text || ''
    }

    chatMessages.value.splice(replyIdx, 1, {
      role: 'assistant',
      content: output || '(no output)',
      status: 'completed',
      sessionId,
    })
    await refreshRuns()
  } catch (e) {
    chatMessages.value.splice(replyIdx, 1, {
      role: 'assistant',
      content: e.message,
      status: 'error',
      sessionId: null,
    })
  } finally {
    chatPending.value = false
  }
}
</script>

<template>
  <div class="flex flex-col h-full">

    <!-- ── Header bar ─────────────────────────────────────────────────── -->
    <div class="px-5 py-3 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <div class="flex items-center gap-3 min-w-0">
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40 shrink-0">Dashboard</span>
        <span class="text-base-content/20 shrink-0">·</span>
        <span class="badge badge-xs font-mono shrink-0" :class="{
          'badge-warning': m.runtime_mode === 'hosted-mcp',
          'badge-success': m.runtime_mode === 'standalone' || m.runtime_mode === 'local-ollama',
          'badge-info': m.runtime_mode === 'dry-run',
        }">{{ m.runtime_mode || 'hosted-mcp' }}</span>
        <span class="text-[11px] font-mono text-base-content/35 truncate hidden sm:block">{{ m.model_provider || '—' }}</span>
        <span class="text-base-content/20 shrink-0 hidden md:block">·</span>
        <span class="text-[11px] font-mono text-base-content/30 shrink-0 hidden md:block">
          {{ m.skills_count }}sk · {{ m.workflows_count }}wf · {{ m.personas_count }}p · {{ m.memory_keys }}mem
        </span>
      </div>
      <div class="flex items-center gap-3 shrink-0">
        <span class="text-[11px] font-mono text-base-content/30 hidden sm:block">{{ uptime }}</span>
        <span class="w-1.5 h-1.5 rounded-full" :class="connected ? 'bg-success shadow-[0_0_6px_oklch(var(--su)/0.8)]' : 'bg-error'"></span>
        <span class="text-[11px] font-mono" :class="connected ? 'text-success/70' : 'text-error/60'">{{ connected ? 'live' : 'offline' }}</span>
        <div class="w-px h-4 bg-base-300"></div>
        <button class="btn btn-sm btn-primary font-mono text-xs gap-1.5" @click="taskPanelOpen = true">
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/>
          </svg>
          New Task
        </button>
      </div>
    </div>

    <!-- ── Stats bento strip ──────────────────────────────────────────── -->
    <div class="shrink-0 px-4 py-3 border-b border-base-content/8 bg-base-200">
      <div class="grid grid-cols-6 rounded-xl border border-base-content/10 bg-base-300 overflow-hidden">

        <!-- Running -->
        <div class="relative px-4 py-3 overflow-hidden border-r border-base-content/8">
          <div v-if="m.running_tasks > 0" class="absolute inset-0 bg-gradient-to-br from-info/10 to-transparent pointer-events-none"></div>
          <div v-if="m.running_tasks > 0" class="absolute bottom-0 inset-x-0 h-[2px] bg-info/60"></div>
          <div class="text-[9px] font-mono text-base-content/35 uppercase tracking-widest mb-1">Running</div>
          <div class="text-2xl font-bold font-mono tabular-nums leading-none" :class="m.running_tasks > 0 ? 'text-info' : 'text-base-content/25'">
            {{ m.running_tasks }}
          </div>
          <div class="text-[9px] font-mono text-base-content/25 mt-1 flex items-center gap-1">
            <span v-if="m.running_tasks > 0" class="w-1 h-1 rounded-full bg-info animate-pulse shrink-0"></span>
            <span>{{ m.session_requests }} req</span>
          </div>
        </div>

        <!-- Awaiting Approval -->
        <div class="relative px-4 py-3 overflow-hidden border-r border-base-content/8">
          <div v-if="(m.awaiting_approvals || 0) > 0" class="absolute inset-0 bg-gradient-to-br from-warning/12 to-transparent pointer-events-none"></div>
          <div v-if="(m.awaiting_approvals || 0) > 0" class="absolute bottom-0 inset-x-0 h-[2px] bg-warning/70"></div>
          <div class="text-[9px] font-mono text-base-content/35 uppercase tracking-widest mb-1">Approvals</div>
          <div class="text-2xl font-bold font-mono tabular-nums leading-none" :class="(m.awaiting_approvals || 0) > 0 ? 'text-warning' : 'text-base-content/25'">
            {{ m.awaiting_approvals || 0 }}
          </div>
          <div class="text-[9px] font-mono text-base-content/25 mt-1">pending</div>
        </div>

        <!-- Completed -->
        <div class="relative px-4 py-3 overflow-hidden border-r border-base-content/8">
          <div class="text-[9px] font-mono text-base-content/35 uppercase tracking-widest mb-1">Completed</div>
          <div class="text-2xl font-bold font-mono tabular-nums leading-none text-success">{{ m.completed_tasks }}</div>
          <div class="text-[9px] font-mono text-base-content/25 mt-1">{{ ((m.success_rate || 0) * 100).toFixed(0) }}% rate</div>
        </div>

        <!-- Failed -->
        <div class="relative px-4 py-3 overflow-hidden border-r border-base-content/8">
          <div v-if="m.failed_tasks > 0" class="absolute inset-0 bg-gradient-to-br from-error/8 to-transparent pointer-events-none"></div>
          <div v-if="m.failed_tasks > 0" class="absolute bottom-0 inset-x-0 h-[2px] bg-error/60"></div>
          <div class="text-[9px] font-mono text-base-content/35 uppercase tracking-widest mb-1">Failed</div>
          <div class="text-2xl font-bold font-mono tabular-nums leading-none" :class="m.failed_tasks > 0 ? 'text-error' : 'text-base-content/25'">
            {{ m.failed_tasks }}
          </div>
          <div class="text-[9px] font-mono text-base-content/25 mt-1">{{ m.total_retries || 0 }} retries</div>
        </div>

        <!-- Tokens + Cost -->
        <div class="relative px-4 py-3 overflow-hidden border-r border-base-content/8">
          <div class="text-[9px] font-mono text-base-content/35 uppercase tracking-widest mb-1">Tokens</div>
          <div class="text-2xl font-bold font-mono tabular-nums leading-none text-primary">
            {{ fmtTokens(m.total_tokens) }}<span v-if="m.runtime_mode === 'hosted-mcp' && m.total_tokens > 0" class="text-[10px] text-base-content/25 ml-0.5">~</span>
          </div>
          <div class="text-[9px] font-mono text-base-content/25 mt-1">${{ (m.estimated_usd || 0).toFixed(4) }}</div>
        </div>

        <!-- Latency + Reward -->
        <div class="relative px-4 py-3 overflow-hidden">
          <div class="text-[9px] font-mono text-base-content/35 uppercase tracking-widest mb-1">Latency</div>
          <div class="text-2xl font-bold font-mono tabular-nums leading-none text-accent">{{ fmtMs(m.avg_latency_ms || 0) }}</div>
          <div class="text-[9px] font-mono text-base-content/25 mt-1">{{ (m.avg_reward || 0).toFixed(2) }} reward</div>
        </div>

      </div>
    </div>

    <div v-if="m.repo_graph" class="shrink-0 px-4 pb-3 border-b border-base-content/8 bg-base-200">
      <div class="rounded-xl border border-base-content/10 bg-base-300/70 px-4 py-3 flex flex-wrap items-center gap-3">
        <div class="flex items-center gap-2 mr-2">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Repo Graph</span>
          <span class="badge badge-xs font-mono" :class="repoGraphBadgeClass(m.repo_graph)">{{ repoGraphState(m.repo_graph) }}</span>
        </div>
        <span class="text-[10px] font-mono text-base-content/35">
          v{{ m.repo_graph.version || '—' }}
        </span>
        <span class="text-[10px] font-mono text-base-content/30">
          {{ m.repo_graph.counts?.symbols || 0 }} sym
        </span>
        <span class="text-[10px] font-mono text-base-content/30">
          {{ m.repo_graph.counts?.files || 0 }} files
        </span>
        <span class="text-[10px] font-mono text-base-content/30">
          {{ m.repo_graph.counts?.edges || 0 }} edges
        </span>
        <span class="text-[10px] font-mono text-base-content/30">
          stale {{ m.repo_graph.stale_files || 0 }}
        </span>
        <span class="text-[10px] font-mono text-base-content/30">
          missing {{ m.repo_graph.missing_files || 0 }}
        </span>
        <span class="text-[10px] font-mono text-base-content/25 truncate max-w-[32rem]">
          {{ m.repo_graph.graph_path || 'no graph file yet' }}
        </span>
      </div>
    </div>

    <div class="shrink-0 px-4 pb-3 border-b border-base-content/8 bg-base-200">
      <div class="rounded-xl border border-base-content/10 bg-base-300/70 p-4 space-y-3">
        <div class="flex flex-wrap items-center gap-3">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Graph Explorer</span>
          <select v-model="graphExplorerMode" class="select select-xs select-bordered font-mono">
            <option value="usage">usage neighborhood</option>
            <option value="impact">impact radius</option>
            <option value="callers">callers</option>
            <option value="callees">callees</option>
            <option value="docs">doc links</option>
            <option value="path">dependency path</option>
            <option value="context">context bundle</option>
          </select>
          <input
            v-if="['usage','impact','callers','callees','docs'].includes(graphExplorerMode)"
            v-model="graphExplorerSymbol"
            class="input input-xs input-bordered font-mono w-56"
            placeholder="symbol"
          />
          <template v-if="graphExplorerMode === 'path'">
            <input v-model="graphExplorerFrom" class="input input-xs input-bordered font-mono w-48" placeholder="from symbol" />
            <input v-model="graphExplorerTo" class="input input-xs input-bordered font-mono w-48" placeholder="to symbol" />
          </template>
          <input
            v-if="graphExplorerMode === 'context'"
            v-model="graphExplorerQuery"
            class="input input-xs input-bordered font-mono flex-1 min-w-[18rem]"
            placeholder="free-text query"
          />
          <input
            v-if="['usage','impact','context'].includes(graphExplorerMode)"
            v-model="graphExplorerDepth"
            type="number"
            min="1"
            max="6"
            class="input input-xs input-bordered font-mono w-20"
            placeholder="depth"
          />
          <button class="btn btn-xs btn-primary font-mono" :disabled="graphExplorerBusy" @click="runGraphExplorer()">
            {{ graphExplorerBusy ? 'running…' : 'run' }}
          </button>
          <button class="btn btn-xs btn-ghost font-mono" :disabled="graphExplorerBusy" @click="runGraphExplorer('etr.graph_build')">build</button>
          <button class="btn btn-xs btn-ghost font-mono" :disabled="graphExplorerBusy" @click="runGraphExplorer('etr.graph_refresh')">refresh</button>
        </div>

        <div class="flex flex-wrap items-start gap-2">
          <textarea
            v-model="graphExplorerPaths"
            rows="2"
            class="textarea textarea-bordered textarea-xs font-mono w-[32rem] max-w-full"
            placeholder="paths for incremental refresh, one per line or comma-separated"
          />
          <button class="btn btn-xs btn-ghost font-mono mt-1" :disabled="graphExplorerBusy" @click="runGraphExplorer('etr.graph_refresh_paths')">refresh paths</button>
          <span class="text-[10px] font-mono text-base-content/35 mt-1">{{ graphExplorerStatus || 'query repo graph, rebuild, or patch changed paths' }}</span>
        </div>

        <div class="grid grid-cols-1 xl:grid-cols-[minmax(0,1.2fr)_22rem] gap-3 items-start">
          <div class="rounded-xl border border-base-content/8 bg-base-200/70 min-h-[10rem] overflow-hidden">
            <div ref="graphExplorerMermaidRef" class="p-3 max-h-[24rem] overflow-auto"></div>
            <div v-if="!graphExplorerMermaidSource" class="px-3 pb-3 text-[10px] font-mono text-base-content/30">
              no diagram yet
            </div>
          </div>
          <div class="rounded-xl border border-base-content/8 bg-base-200/70 p-3 space-y-2">
            <div class="text-[9px] font-mono uppercase tracking-widest text-base-content/30">Result Summary</div>
            <template v-if="graphExplorerResult?.output">
              <div class="font-mono text-[11px] text-base-content/65">tool: {{ graphExplorerResult.tool }}</div>
              <div class="font-mono text-[11px] text-base-content/65" v-if="graphExplorerResult.output.count != null">count: {{ graphExplorerResult.output.count }}</div>
              <div class="font-mono text-[11px] text-base-content/65" v-if="graphExplorerResult.output.symbol_count != null">symbols: {{ graphExplorerResult.output.symbol_count }}</div>
              <div class="font-mono text-[11px] text-base-content/65" v-if="graphExplorerResult.output.file_count != null">files: {{ graphExplorerResult.output.file_count }}</div>
              <div class="font-mono text-[11px] text-base-content/65" v-if="graphExplorerResult.output.counts">graph: {{ graphExplorerResult.output.counts.symbols || 0 }} sym / {{ graphExplorerResult.output.counts.edges || 0 }} edges</div>
              <pre class="mt-2 text-[10px] font-mono whitespace-pre-wrap break-words max-h-[18rem] overflow-auto text-base-content/60">{{ JSON.stringify(graphExplorerResult.output, null, 2) }}</pre>
            </template>
            <div v-else class="text-[10px] font-mono text-base-content/30">run a graph query to inspect structure and paths</div>
          </div>
        </div>
      </div>
    </div>

    <!-- ── Master-detail body ─────────────────────────────────────────── -->
    <div class="flex-1 flex min-h-0">

      <!-- ── Left panel: live sessions + run list ──────────────────── -->
      <div class="w-72 xl:w-80 flex flex-col bg-base-300/70 border-r border-base-content/12 overflow-hidden shrink-0">

        <!-- Live sessions header -->
        <div class="shrink-0 px-3 py-2 border-b border-base-content/8 flex items-center justify-between">
          <div class="flex items-center gap-2">
            <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Live</span>
            <span class="badge badge-xs badge-ghost font-mono">{{ runtime.counts.active }}</span>
          </div>
          <div class="flex items-center gap-2.5 text-[9px] font-mono text-base-content/25">
            <span>{{ runtime.counts.running }}r · {{ runtime.counts.blocked }}b · {{ runtime.counts.failed }}f</span>
            <button class="text-primary/60 hover:text-primary transition-colors" @click="refreshRuntimeSessions" title="Refresh">↺</button>
          </div>
        </div>

        <!-- Live session cards -->
        <div class="shrink-0 max-h-56 overflow-auto" v-if="runtimeFocusSessions.length">
          <button
            v-for="session in runtimeFocusSessions"
            :key="session.id"
            class="w-full text-left px-3 py-2.5 border-b border-base-content/8 transition-all hover:bg-base-200/60 group border-l-[3px]"
            :class="[
              sessionBorderColor(session.lifecycle),
              expandedRunId === session.id ? 'bg-primary/8' : '',
            ]"
            @click="toggleRun(session.id)"
          >
            <!-- Kind + badge -->
            <div class="flex items-center justify-between gap-2 mb-0.5">
              <span class="font-mono text-[11px] font-semibold text-base-content/80 truncate">{{ session.kind }}</span>
              <span class="badge badge-xs font-mono shrink-0" :class="runtimeBadgeClass(session.lifecycle)">{{ session.lifecycle }}</span>
            </div>
            <!-- Task -->
            <div class="font-mono text-[10px] text-base-content/40 truncate mb-2">{{ session.task }}</div>
            <!-- Workflow progress bar -->
            <div v-if="session.workflow" class="mb-1.5">
              <div class="flex items-center justify-between mb-1">
                <span class="text-[9px] font-mono text-base-content/30 truncate max-w-[10rem]">{{ session.workflow.workflow }}</span>
                <span class="text-[9px] font-mono text-base-content/30 shrink-0 ml-1">{{ session.workflow.completed_steps }}/{{ session.workflow.total_steps }}</span>
              </div>
              <div class="h-1 bg-base-content/10 rounded-full overflow-hidden">
                <div
                  class="h-full rounded-full transition-all duration-700"
                  :class="workflowProgressColor(session.lifecycle)"
                  :style="{ width: `${Math.min(100, ((session.workflow.completed_steps || 0) / (session.workflow.total_steps || 1)) * 100)}%` }"
                ></div>
              </div>
              <div v-if="session.workflow.pending_approval_step" class="text-[9px] font-mono text-warning/70 mt-1">⏸ gate: {{ session.workflow.pending_approval_step }}</div>
            </div>
            <!-- Footer: provider + age + action hint -->
            <div class="flex items-center gap-1.5">
              <span v-if="session.provider" class="text-[9px] font-mono text-base-content/25 truncate">{{ session.provider }}</span>
              <span v-if="session.workflow?.running_steps?.length" class="text-[9px] font-mono text-info/60 truncate">→ {{ session.workflow.running_steps[0] }}</span>
              <span class="text-[9px] font-mono text-base-content/20 ml-auto shrink-0">{{ fmtAgeSeconds(session.age_seconds) }}</span>
            </div>
          </button>
        </div>
        <div v-else class="shrink-0 py-4 text-center text-[10px] font-mono text-base-content/20 border-b border-base-content/8">
          no active sessions
        </div>

        <!-- Runs header + filter -->
        <div class="shrink-0 px-3 pt-2.5 pb-2 border-b border-base-content/8 space-y-1.5">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2">
              <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Runs</span>
              <span class="text-[9px] font-mono text-base-content/20">{{ filteredRuns.length }}/{{ runs.length }}</span>
            </div>
            <div class="flex items-center gap-2 text-[9px] font-mono">
              <button class="text-primary/60 hover:text-primary transition-colors" @click="refreshRuns" title="Refresh">↺</button>
              <button class="text-base-content/30 hover:text-base-content/60 transition-colors" @click="cleanupStaleApprovals" title="Cleanup stale approvals">⌫</button>
            </div>
          </div>
          <input
            v-model="runFilter"
            class="w-full bg-base-200/80 border border-base-content/10 rounded-lg text-[10px] font-mono px-2.5 py-1.5 focus:outline-none focus:border-primary/50 transition-colors placeholder-base-content/20"
            placeholder="search runs…"
          />
          <div class="flex gap-1 flex-wrap">
            <button
              v-for="s in ['', 'running', 'succeeded', 'failed', 'awaiting-approval']"
              :key="s"
              class="px-1.5 py-0.5 text-[9px] font-mono rounded-md transition-colors border"
              :class="runStatusFilter === s
                ? 'bg-primary/15 border-primary/40 text-primary'
                : 'bg-transparent border-base-content/10 text-base-content/30 hover:border-base-content/25 hover:text-base-content/50'"
              @click="runStatusFilter = s"
            >{{ s === '' ? 'all' : s.replace('-', ' ') }}</button>
          </div>
          <div v-if="cleanupStatus" class="text-[9px] font-mono text-base-content/35 leading-relaxed">{{ cleanupStatus }}</div>
        </div>

        <!-- Runs scrollable list -->
        <div class="flex-1 overflow-auto">
          <button
            v-for="run in filteredRuns"
            :key="run.id"
            class="w-full text-left px-3 py-2.5 border-b border-base-content/6 flex items-start gap-2.5 transition-all hover:bg-base-200/50 border-l-[3px]"
            :class="expandedRunId === run.id
              ? 'bg-primary/8 border-l-primary'
              : 'border-l-transparent hover:border-l-base-content/15'"
            @click="toggleRun(run.id)"
          >
            <!-- Status dot -->
            <span class="w-1.5 h-1.5 rounded-full shrink-0 mt-1.5" :class="{
              'bg-info animate-pulse': run.status === 'running',
              'bg-warning animate-pulse': run.status === 'awaiting-approval',
              'bg-success': run.status === 'succeeded',
              'bg-error': run.status === 'failed',
              'bg-base-content/20': !['running','awaiting-approval','succeeded','failed'].includes(run.status),
            }"></span>
            <!-- Content -->
            <div class="min-w-0 flex-1">
              <div class="font-mono text-[11px] text-base-content/65 truncate leading-tight">{{ run.kind }}</div>
              <div class="font-mono text-[10px] text-base-content/35 truncate mt-0.5 leading-tight">{{ run.task }}</div>
            </div>
            <!-- Status badge -->
            <span class="badge badge-xs shrink-0 mt-0.5" :class="{
              'badge-info': run.status === 'running',
              'badge-warning': run.status === 'awaiting-approval',
              'badge-success': run.status === 'succeeded',
              'badge-error': run.status === 'failed',
            }">{{ run.status }}</span>
          </button>
          <div v-if="!filteredRuns.length" class="py-10 text-center text-[10px] font-mono text-base-content/20">
            <template v-if="runFilter || runStatusFilter">no runs match</template>
            <template v-else>no persisted runs yet</template>
          </div>
        </div>

      </div>
      <!-- /left panel -->

      <!-- ── Right panel ───────────────────────────────────────────── -->
      <div class="flex-1 overflow-auto bg-base-100/30">

        <!-- ── Idle view: shown when no run is selected ──────────── -->
        <div v-if="!expandedRunId" class="p-4 space-y-4">

          <!-- Provider readiness -->
          <div class="rounded-xl border border-base-content/12 bg-base-200/50 overflow-hidden">
            <div class="px-4 py-2.5 border-b border-base-content/10 flex items-center justify-between gap-3">
              <div class="flex items-center gap-2 min-w-0">
                <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Provider Readiness</span>
                <span class="badge badge-xs font-mono" :class="providerReadiness.standalone_available ? 'badge-success' : 'badge-warning'">
                  {{ providerReadiness.runtime_mode || 'hosted-mcp' }}
                </span>
                <span v-if="providerReadiness.selected_model" class="text-[10px] font-mono text-base-content/35 truncate hidden md:block">
                  {{ providerReadiness.selected_model }}
                </span>
              </div>
              <div class="text-[10px] font-mono text-base-content/25 truncate max-w-[40rem] hidden lg:block">
                {{ providerReadiness.hints?.[0] || 'Provider status loaded from local config and environment.' }}
              </div>
            </div>
            <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-5 gap-2.5 p-3">
              <div
                v-for="provider in providerCards"
                :key="provider.id"
                class="rounded-xl border p-3 min-h-28 flex flex-col justify-between"
                :class="providerCardClass(provider.status)"
              >
                <div>
                  <div class="flex items-start justify-between gap-2 mb-1.5">
                    <div class="font-mono text-xs font-semibold text-base-content/80 truncate" :title="provider.label">{{ provider.label }}</div>
                    <span class="badge badge-xs font-mono shrink-0" :class="providerBadgeClass(provider.status)">{{ provider.status }}</span>
                  </div>
                  <div class="text-[10px] font-mono text-base-content/35 truncate" :title="provider.model || provider.source">
                    {{ provider.model || provider.source || '—' }}
                  </div>
                </div>
                <div class="mt-3 space-y-1">
                  <div class="flex items-center gap-2 text-[10px] font-mono">
                    <span class="w-1.5 h-1.5 rounded-full shrink-0" :class="provider.available ? 'bg-success' : provider.configured ? 'bg-warning' : 'bg-base-content/20'"></span>
                    <span class="text-base-content/40">{{ provider.selected ? 'selected' : provider.available ? 'available' : provider.configured ? 'configured' : 'not configured' }}</span>
                  </div>
                  <div class="text-[10px] font-mono text-base-content/30 line-clamp-2" :title="provider.hint">{{ provider.hint }}</div>
                </div>
              </div>
              <div v-if="!providerCards.length" class="col-span-full py-6 text-center text-[10px] font-mono text-base-content/20">
                no providers configured
              </div>
            </div>
          </div>

          <!-- ETR cache stats + sparkline row -->
          <div class="grid gap-3" :class="etrCacheStats ? 'grid-cols-3' : 'grid-cols-1'">

            <!-- ETR cache stats -->
            <div v-if="etrCacheStats" class="col-span-2 rounded-xl border border-warning/25 bg-base-200/50 p-4 flex items-center gap-5">
              <div class="flex items-center gap-2 shrink-0">
                <span class="text-warning text-base">⚡</span>
                <div>
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">ETR Step Cache</div>
                  <div class="text-xs font-mono text-base-content/70 mt-0.5">
                    {{ etrCacheStats.entries ?? 0 }} entries · {{ etrCacheStats.size_bytes ? (etrCacheStats.size_bytes / 1024).toFixed(1) + ' KB' : '0 KB' }}
                  </div>
                </div>
              </div>
              <div class="flex gap-5 ml-auto">
                <div class="text-center">
                  <div class="text-[9px] font-mono text-base-content/30">Hits</div>
                  <div class="text-base font-bold font-mono text-success tabular-nums">{{ etrCacheStats.hits ?? '—' }}</div>
                </div>
                <div class="text-center">
                  <div class="text-[9px] font-mono text-base-content/30">Misses</div>
                  <div class="text-base font-bold font-mono text-warning tabular-nums">{{ etrCacheStats.misses ?? '—' }}</div>
                </div>
                <div v-if="etrCacheStats.hits !== undefined && etrCacheStats.misses !== undefined && (etrCacheStats.hits + etrCacheStats.misses) > 0" class="text-center">
                  <div class="text-[9px] font-mono text-base-content/30">Hit Rate</div>
                  <div class="text-base font-bold font-mono text-success tabular-nums">{{ ((etrCacheStats.hits / (etrCacheStats.hits + etrCacheStats.misses)) * 100).toFixed(0) }}%</div>
                </div>
              </div>
              <button class="btn btn-xs btn-warning btn-outline font-mono ml-4" @click="clearEtrCache">clear</button>
            </div>

            <!-- Token sparkline + KPI mini cards -->
            <div class="rounded-xl border border-base-content/10 bg-base-200/50 p-4 flex flex-col justify-between gap-3" :class="etrCacheStats ? '' : 'col-span-1'">
              <div v-if="tokenSparkline.length >= 4" class="flex items-center gap-3">
                <span class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest shrink-0">Tokens</span>
                <svg :width="120" :height="22" class="opacity-50 shrink-0">
                  <polyline
                    :points="sparklinePath(tokenSparkline, 120, 22)"
                    fill="none"
                    stroke="oklch(var(--p))"
                    stroke-width="1.5"
                    stroke-linejoin="round"
                    stroke-linecap="round"
                  />
                </svg>
                <span class="text-[9px] font-mono text-base-content/25">{{ fmtTokens(tokenSparkline[tokenSparkline.length - 1]) }} last</span>
              </div>
              <div class="grid grid-cols-3 gap-2">
                <div>
                  <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest">Success</div>
                  <div class="text-sm font-bold font-mono text-success tabular-nums">{{ ((m.success_rate || 0) * 100).toFixed(0) }}%</div>
                </div>
                <div>
                  <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest">Cost/run</div>
                  <div class="text-sm font-bold font-mono text-warning tabular-nums">${{ (m.avg_cost_usd || 0).toFixed(3) }}</div>
                </div>
                <div>
                  <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest">Reward</div>
                  <div class="text-sm font-bold font-mono text-accent tabular-nums">{{ (m.avg_reward || 0).toFixed(2) }}</div>
                </div>
              </div>
            </div>
          </div>

          <!-- Runtime sessions full view -->
          <div class="rounded-xl border border-base-content/12 bg-base-200/50 overflow-hidden">
            <div class="px-4 py-2.5 border-b border-base-content/10 flex items-center justify-between">
              <div class="flex items-center gap-2">
                <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Runtime Sessions</span>
                <span class="badge badge-xs badge-ghost font-mono">{{ runtime.counts.active }} active</span>
              </div>
              <div class="flex items-center gap-3 text-[10px] font-mono text-base-content/25">
                <span>{{ runtime.counts.running }} running · {{ runtime.counts.blocked }} blocked · {{ runtime.counts.failed }} failed</span>
                <button class="text-primary/60 hover:text-primary transition-colors" @click="refreshRuntimeSessions">↺ refresh</button>
              </div>
            </div>
            <div v-if="runtimeFocusSessions.length" class="grid grid-cols-1 xl:grid-cols-2 gap-2 p-3">
              <button
                v-for="session in runtimeFocusSessions"
                :key="session.id"
                class="text-left rounded-xl border p-3 transition-all hover:border-primary/40 hover:bg-base-300/20"
                :class="runtimeLifecycleClass(session.lifecycle)"
                @click="toggleRun(session.id)"
              >
                <div class="flex items-start justify-between gap-3 mb-2">
                  <div class="min-w-0">
                    <div class="flex items-center gap-2 mb-0.5">
                      <span class="font-mono text-xs font-semibold text-base-content/80 truncate max-w-[16rem]">{{ session.kind }}</span>
                      <span class="badge badge-xs font-mono" :class="runtimeBadgeClass(session.lifecycle)">{{ session.lifecycle }}</span>
                    </div>
                    <div class="font-mono text-xs text-base-content/45 truncate">{{ session.task }}</div>
                  </div>
                  <div class="text-right shrink-0">
                    <div class="text-[10px] font-mono text-base-content/35">{{ fmtAgeSeconds(session.age_seconds) }}</div>
                    <div class="text-[10px] font-mono text-base-content/20">{{ session.mode }}</div>
                  </div>
                </div>
                <!-- Progress bar for workflow sessions -->
                <div v-if="session.workflow" class="mb-2">
                  <div class="flex items-center justify-between mb-1">
                    <span class="text-[9px] font-mono text-base-content/30">{{ session.workflow.workflow }}</span>
                    <span class="text-[9px] font-mono text-base-content/30">{{ session.workflow.completed_steps }}/{{ session.workflow.total_steps }} steps</span>
                  </div>
                  <div class="h-1 bg-base-content/10 rounded-full overflow-hidden">
                    <div
                      class="h-full rounded-full transition-all duration-700"
                      :class="workflowProgressColor(session.lifecycle)"
                      :style="{ width: `${Math.min(100, ((session.workflow.completed_steps || 0) / (session.workflow.total_steps || 1)) * 100)}%` }"
                    ></div>
                  </div>
                </div>
                <div class="flex items-center gap-2 flex-wrap">
                  <span v-if="session.provider" class="badge badge-xs badge-ghost font-mono">{{ session.provider }}</span>
                  <span v-if="session.workflow?.running_steps?.length" class="text-[10px] font-mono text-info/70">→ {{ session.workflow.running_steps.join(', ') }}</span>
                  <span v-if="session.workflow?.ready_steps?.length" class="text-[10px] font-mono text-primary/70">ready: {{ session.workflow.ready_steps.join(', ') }}</span>
                  <span v-if="session.workflow?.pending_approval_step" class="text-[10px] font-mono text-warning/80">⏸ {{ session.workflow.pending_approval_step }}</span>
                </div>
                <div class="mt-2 flex items-center justify-between gap-3">
                  <span class="text-[10px] font-mono text-base-content/30 truncate">{{ session.action_hint }}</span>
                  <span class="text-[10px] font-mono text-base-content/20 shrink-0">{{ session.id.slice(0, 8) }}</span>
                </div>
                <div v-if="session.workflow?.last_error || session.output_preview" class="mt-1 text-[10px] font-mono text-base-content/30 truncate">
                  {{ session.workflow?.last_error || session.output_preview }}
                </div>
              </button>
            </div>
            <div v-else class="p-8 text-center text-base-content/20 text-sm font-mono">no runtime sessions yet</div>
          </div>

          <!-- Recent Tasks table -->
          <div class="rounded-xl border border-base-content/12 bg-base-200/50 flex flex-col" style="max-height: 28vh">
            <div class="px-4 py-2.5 border-b border-base-content/10 flex items-center justify-between shrink-0">
              <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Recent Tasks</span>
              <span class="text-[10px] font-mono text-base-content/25">{{ m.recent_tasks?.length || 0 }} this session</span>
            </div>
            <div class="overflow-auto flex-1">
              <table class="table table-sm w-full" v-if="m.recent_tasks?.length">
                <thead class="sticky top-0 bg-base-200 z-10">
                  <tr class="text-[10px] text-base-content/35 uppercase tracking-wider">
                    <th class="w-[38%] font-medium">Task</th>
                    <th class="w-[12%] font-medium">Mode</th>
                    <th class="w-[14%] font-medium">Model</th>
                    <th class="w-[10%] font-medium">Status</th>
                    <th class="w-[9%] font-medium">Tokens</th>
                    <th class="w-[17%] font-medium">Time</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="(t, i) in [...(m.recent_tasks || [])].reverse()" :key="i" class="hover:bg-base-300/30 transition-colors">
                    <td class="max-w-[0] truncate font-mono text-xs text-base-content/75" :title="t.task">{{ t.task }}</td>
                    <td>
                      <span class="badge badge-xs font-mono" :class="{
                        'badge-warning': t.agent === 'hosted-mcp',
                        'badge-success': t.agent === 'standalone',
                        'badge-info': t.agent === 'dry-run',
                        'badge-ghost': !['hosted-mcp','standalone','dry-run'].includes(t.agent),
                      }">{{ t.agent || '—' }}</span>
                    </td>
                    <td class="text-xs font-mono text-base-content/45 truncate max-w-[0]" :title="t.model">{{ t.model || '—' }}</td>
                    <td>
                      <span class="badge badge-xs" :class="{
                        'badge-info': t.status === 'running',
                        'badge-success': t.status === 'completed' || t.status === 'succeeded',
                        'badge-error': t.status === 'failed',
                        'badge-warning': t.status === 'awaiting-approval',
                      }">{{ t.status }}</span>
                    </td>
                    <td class="text-xs font-mono text-base-content/45">
                      <span v-if="t.tokens > 0">{{ fmtTokens(t.tokens) }}<span v-if="t.agent === 'hosted-mcp'" class="text-base-content/25">~</span></span>
                      <span v-else class="text-base-content/20">—</span>
                    </td>
                    <td class="text-[10px] text-base-content/35 font-mono">{{ fmtLocalTime(t.started_at) }}</td>
                  </tr>
                </tbody>
              </table>
              <div v-else class="py-10 text-center text-base-content/20 text-sm font-mono">no tasks yet this session</div>
            </div>
          </div>

        </div>
        <!-- /idle view -->

        <!-- ── Run detail view ──────────────────────────────────────── -->
        <div v-else-if="selectedRun?.run" class="p-4 space-y-4">

          <!-- Detail header / back -->
          <div class="flex items-center gap-3">
            <button
              class="btn btn-ghost btn-xs font-mono text-[10px] gap-1 text-base-content/40 hover:text-base-content/70 px-2"
              @click="toggleRun(expandedRunId)"
            >← back</button>
            <div class="flex-1 min-w-0 flex items-center gap-2">
              <span class="font-mono text-xs text-base-content/55 truncate">{{ selectedRun.run.metadata.kind }}</span>
              <span class="badge badge-sm font-mono" :class="{
                'badge-info': selectedRun.run.metadata.status === 'running',
                'badge-warning': selectedRun.run.metadata.status === 'awaiting-approval',
                'badge-success': selectedRun.run.metadata.status === 'succeeded',
                'badge-error': selectedRun.run.metadata.status === 'failed',
              }">{{ selectedRun.run.metadata.status }}</span>
            </div>
            <span class="text-[10px] font-mono text-base-content/25 shrink-0">{{ fmtLocalTime(selectedRun.run.metadata.started_at) }}</span>
          </div>

          <!-- Metadata grid -->
          <div class="grid grid-cols-4 gap-2.5">
            <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
              <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Run ID</div>
              <div class="font-mono text-xs text-base-content/60 truncate" :title="selectedRun.run.metadata.id">{{ selectedRun.run.metadata.id }}</div>
            </div>
            <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
              <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Kind</div>
              <div class="font-mono text-xs text-base-content/60 truncate">{{ selectedRun.run.metadata.kind }}</div>
            </div>
            <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
              <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Status</div>
              <span class="badge badge-sm font-mono" :class="{
                'badge-info': selectedRun.run.metadata.status === 'running',
                'badge-warning': selectedRun.run.metadata.status === 'awaiting-approval',
                'badge-success': selectedRun.run.metadata.status === 'succeeded',
                'badge-error': selectedRun.run.metadata.status === 'failed',
              }">{{ selectedRun.run.metadata.status }}</span>
            </div>
            <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
              <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Started</div>
              <div class="font-mono text-xs text-base-content/60">{{ fmtLocalTime(selectedRun.run.metadata.started_at) }}</div>
            </div>
          </div>

          <!-- Task -->
          <div>
            <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Task</div>
            <div class="md-step-output bg-base-200/60 rounded-xl p-4 text-xs leading-relaxed border border-base-content/6"
              v-html="renderMarkdown(selectedRun.run.metadata.task)"
            />
          </div>

          <!-- Output -->
          <div v-if="selectedRunOutput">
            <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Output</div>
            <div class="md-step-output bg-base-200/60 rounded-xl p-4 text-xs leading-relaxed max-h-64 overflow-auto border border-base-content/6"
              v-html="renderMarkdown(selectedRunOutput)"
            />
          </div>

          <!-- Session notes -->
          <div class="rounded-xl border border-base-content/10 bg-base-200/40 p-4">
            <div class="flex items-center justify-between gap-3 mb-3">
              <div>
                <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">Session Notes</div>
                <div class="font-mono text-xs text-base-content/40 mt-1">Operator and agent messages for handoff, review, and continuation context.</div>
              </div>
              <span class="badge badge-sm badge-ghost font-mono">{{ runtimeMessages.length }} note(s)</span>
            </div>

            <div v-if="runtimeMessages.length" class="space-y-2 max-h-48 overflow-auto pr-1 mb-3">
              <div
                v-for="message in runtimeMessages"
                :key="message.id"
                class="rounded-xl border border-base-content/8 bg-base-100/60 p-3"
              >
                <div class="flex items-center justify-between gap-2 mb-1">
                  <div class="flex items-center gap-2">
                    <span class="font-mono text-xs text-base-content/70">{{ message.from || message.author || 'operator' }}</span>
                    <span v-if="message.to" class="font-mono text-[10px] text-base-content/30">→ {{ message.to }}</span>
                    <span class="badge badge-xs badge-ghost font-mono">{{ message.kind || 'note' }}</span>
                  </div>
                  <span class="text-[10px] text-base-content/25 font-mono">{{ fmtLocalTime(message.created_at) }}</span>
                </div>
                <div class="font-mono text-xs text-base-content/55 whitespace-pre-wrap leading-relaxed">{{ message.body }}</div>
              </div>
            </div>
            <div v-else class="rounded-xl border border-dashed border-base-content/10 bg-base-100/30 p-3 mb-3 font-mono text-xs text-base-content/30">
              No notes yet. Add a handoff, decision, or next-action note.
            </div>

            <div class="grid grid-cols-1 md:grid-cols-12 gap-2 mb-2">
              <select v-model="runtimeMessageKind" class="select select-bordered select-sm md:col-span-3 font-mono text-xs">
                <option value="note">note</option>
                <option value="request">request</option>
                <option value="handoff">handoff</option>
                <option value="progress">progress</option>
                <option value="warning">warning</option>
                <option value="result">result</option>
              </select>
              <input
                v-model="runtimeMessageTo"
                class="input input-bordered input-sm md:col-span-9 font-mono text-xs"
                placeholder="optional target agent/session e.g. Coder"
              />
            </div>
            <div class="flex flex-col lg:flex-row gap-2">
              <textarea
                v-model="runtimeMessageText"
                class="textarea textarea-bordered textarea-sm flex-1 font-mono min-h-[74px]"
                placeholder="Add a session note for future agents..."
                @keydown.meta.enter.prevent="postRuntimeMessage"
                @keydown.ctrl.enter.prevent="postRuntimeMessage"
              ></textarea>
              <div class="flex lg:flex-col gap-2 lg:w-36">
                <button
                  class="btn btn-sm btn-primary font-mono text-xs"
                  :class="{ 'loading': runtimeMessageBusy }"
                  :disabled="!runtimeMessageText.trim() || runtimeMessageBusy"
                  @click="postRuntimeMessage"
                >add note</button>
                <button class="btn btn-sm btn-ghost font-mono text-xs" @click="loadRuntimeMessages()">refresh</button>
              </div>
            </div>
            <div v-if="runtimeMessageStatus" class="font-mono text-[10px] mt-2" :class="runtimeMessageStatus.startsWith('Error') ? 'text-error' : 'text-base-content/40'">{{ runtimeMessageStatus }}</div>
          </div>

          <!-- Artifacts -->
          <div v-if="selectedRunArtifacts.length" class="rounded-xl border border-base-content/10 bg-base-200/40 p-4">
            <div class="flex items-center justify-between gap-3 mb-3">
              <div>
                <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">Artifacts</div>
                <div class="font-mono text-xs text-base-content/40 mt-1">Preview reports, diagrams, mocks, and images without leaving the dashboard.</div>
              </div>
              <span class="badge badge-sm badge-ghost font-mono">{{ selectedRunArtifacts.length }} file(s)</span>
            </div>

            <div class="grid grid-cols-12 gap-3">
              <div class="col-span-4 space-y-1 max-h-56 overflow-auto pr-1">
                <button
                  v-for="artifact in selectedRunArtifacts"
                  :key="artifact"
                  class="btn btn-xs h-auto min-h-0 w-full justify-start normal-case font-mono text-left py-2 px-2"
                  :class="selectedArtifactPath === artifact ? 'btn-primary' : 'btn-ghost'"
                  :title="artifact"
                  @click="previewArtifact(artifact)"
                >
                  <span class="truncate">{{ artifact }}</span>
                </button>
              </div>

              <div class="col-span-8 rounded-xl border border-base-content/8 bg-base-100/50 min-h-56 overflow-hidden">
                <div v-if="artifactPreviewStatus" class="p-4 font-mono text-xs text-warning">{{ artifactPreviewStatus }}</div>
                <div v-else-if="!selectedArtifactPreview" class="h-full min-h-56 flex items-center justify-center text-center p-6">
                  <div>
                    <div class="text-2xl mb-2 opacity-20">▣</div>
                    <div class="font-mono text-xs text-base-content/30">Select an artifact to preview.</div>
                  </div>
                </div>
                <div v-else>
                  <div class="flex items-center justify-between gap-3 border-b border-base-content/8 px-3 py-2">
                    <div class="min-w-0">
                      <div class="font-mono text-xs text-base-content/75 truncate" :title="selectedArtifactPreview.path">{{ selectedArtifactPreview.path }}</div>
                      <div class="font-mono text-[10px] text-base-content/30">{{ selectedArtifactPreview.kind }} · {{ selectedArtifactPreview.mime }} · {{ fmtBytes(selectedArtifactPreview.size_bytes) }}</div>
                    </div>
                    <a class="btn btn-xs btn-ghost" :href="selectedArtifactRawUrl" target="_blank" rel="noopener noreferrer">raw</a>
                  </div>
                  <div v-if="selectedArtifactPreview.truncated" class="p-4 font-mono text-xs text-warning">Artifact exceeds inline preview limit. Open raw instead.</div>
                  <div
                    v-else-if="selectedArtifactPreview.kind === 'markdown'"
                    ref="artifactPreviewRef"
                    class="md-step-output p-4 text-xs leading-relaxed max-h-96 overflow-auto"
                    v-html="renderMarkdown(selectedArtifactPreview.content || '')"
                  />
                  <iframe
                    v-else-if="selectedArtifactPreview.kind === 'html'"
                    class="w-full h-96 bg-white"
                    sandbox
                    :srcdoc="selectedArtifactPreview.content || ''"
                  />
                  <img
                    v-else-if="selectedArtifactPreview.kind === 'image'"
                    class="max-h-96 max-w-full mx-auto p-3 object-contain"
                    :src="selectedArtifactRawUrl"
                    :alt="selectedArtifactPreview.path"
                  />
                  <div v-else-if="selectedArtifactPreview.kind === 'mermaid'" ref="artifactPreviewRef" class="p-4 max-h-96 overflow-auto">
                    <div class="artifact-mermaid-block">{{ selectedArtifactPreview.content || '' }}</div>
                  </div>
                  <pre v-else-if="selectedArtifactIsRenderable" class="p-4 text-xs whitespace-pre-wrap max-h-96 overflow-auto"><code>{{ selectedArtifactPreview.content || '' }}</code></pre>
                  <div v-else class="p-4 font-mono text-xs text-base-content/40">Binary artifact. Open raw to inspect.</div>
                </div>
              </div>
            </div>
          </div>

          <!-- Retrieval Telemetry -->
          <div v-if="selectedRetrievalTelemetry">
            <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Retrieval Telemetry</div>
            <div class="grid grid-cols-4 gap-2.5 mb-2.5">
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Indexed Docs</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.indexed_docs || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Hit Rate</div>
                <div class="font-mono text-xs text-base-content/70">{{ fmtPct(selectedRetrievalTelemetry.retrieval_hit_rate || 0) }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Queries / Hits</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.retrieval_queries || 0 }} / {{ selectedRetrievalTelemetry.retrieval_hits || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Context Chars</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.rag_context_chars || 0 }}</div>
              </div>
            </div>
            <div class="grid grid-cols-3 xl:grid-cols-6 gap-2.5">
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Graph Hits</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.graph_hits || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Graph Files</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.graph_files || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Graph Chars</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.graph_context_chars || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Vector Hits</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.vector_hits || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Fallback Hits</div>
                <div class="font-mono text-xs text-base-content/70">{{ selectedRetrievalTelemetry.fallback_hits || 0 }}</div>
              </div>
              <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Mock Embedding</div>
                <div class="font-mono text-xs" :class="selectedRetrievalTelemetry.mock_embedding ? 'text-warning' : 'text-success'">
                  {{ selectedRetrievalTelemetry.mock_embedding ? 'yes' : 'no' }}
                </div>
              </div>
            </div>
          </div>

          <!-- Token Summary -->
          <div v-if="selectedRunTokenSummary" class="rounded-xl border border-base-content/8 bg-base-200/50 p-3">
            <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Token Summary</div>
            <div class="font-mono text-xs text-base-content/65">
              {{ selectedRunTokenSummary.tokens || 0 }} tokens · {{ selectedRunTokenSummary.requests || 0 }} request(s)
            </div>
          </div>

          <!-- Persona Policy Warning -->
          <div v-if="selectedPersonaPolicyWarning" class="rounded-xl border border-warning/40 bg-warning/8 p-4">
            <div class="text-xs font-bold uppercase tracking-wider text-warning mb-2">Persona Tool Policy</div>
            <div class="font-mono text-xs text-base-content/65 whitespace-pre-wrap">
              {{ selectedPersonaPolicyWarning.message || 'Tool policy warning recorded.' }}
            </div>
            <div class="font-mono text-[11px] text-base-content/45 mt-2">
              persona={{ selectedPersonaPolicyWarning.active_persona || 'unknown' }} · tool={{ selectedPersonaPolicyWarning.requested_tool || 'unknown' }} · strict={{ selectedPersonaPolicyWarning.strict_mode ? 'on' : 'off' }}
            </div>
          </div>

          <!-- Workflow State -->
          <template v-if="selectedWorkflowState">
            <div>
              <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-3">Workflow State</div>
              <div class="grid grid-cols-4 gap-2.5 mb-4">
                <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Workflow</div>
                  <div class="font-mono text-xs text-base-content/70">{{ selectedWorkflowState.workflow }}</div>
                </div>
                <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1.5">
                    Progress · {{ selectedWorkflowState.steps_completed }}/{{ selectedWorkflowState.steps_total }}
                  </div>
                  <div class="h-1.5 bg-base-content/10 rounded-full overflow-hidden">
                    <div
                      class="h-full bg-primary rounded-full transition-all duration-700"
                      :style="{ width: `${Math.min(100, ((selectedWorkflowState.steps_completed || 0) / (selectedWorkflowState.steps_total || 1)) * 100)}%` }"
                    ></div>
                  </div>
                </div>
                <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Budget Used</div>
                  <div class="font-mono text-xs text-base-content/70">{{ fmtTokens(selectedWorkflowState.budget_used?.tokens || 0) }} tokens</div>
                </div>
                <div class="bg-base-200/70 rounded-xl p-3 border border-base-content/8">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Artifacts</div>
                  <div class="font-mono text-xs text-base-content/70">{{ selectedRun.run.artifacts?.length || 0 }} file(s)</div>
                  <div class="text-[9px] font-mono text-base-content/25 mt-0.5 truncate" :title="selectedRun.run.artifacts?.join(', ')">
                    {{ selectedRun.run.artifacts?.join(', ') || '—' }}
                  </div>
                </div>
              </div>

              <!-- Eval Gate -->
              <div
                v-if="selectedEvalGateDecision"
                class="rounded-xl p-4 mb-4 border"
                :class="{
                  'border-success/40 bg-success/8': selectedEvalGateDecision.decision === 'pass',
                  'border-warning/40 bg-warning/8': selectedEvalGateDecision.decision === 'warn',
                  'border-error/40 bg-error/8': selectedEvalGateDecision.decision === 'block',
                }"
              >
                <div class="flex items-center justify-between gap-3 mb-3">
                  <div>
                    <div class="text-xs font-bold uppercase tracking-wider" :class="{
                      'text-success': selectedEvalGateDecision.decision === 'pass',
                      'text-warning': selectedEvalGateDecision.decision === 'warn',
                      'text-error': selectedEvalGateDecision.decision === 'block',
                    }">Eval Gate</div>
                    <div class="font-mono text-xs mt-1 text-base-content/50">{{ selectedEvalGateDecision.workflow }} · {{ selectedEvalGateDecision.mode }}</div>
                  </div>
                  <span class="badge badge-sm" :class="{
                    'badge-success': selectedEvalGateDecision.decision === 'pass',
                    'badge-warning': selectedEvalGateDecision.decision === 'warn',
                    'badge-error': selectedEvalGateDecision.decision === 'block',
                  }">{{ selectedEvalGateDecision.decision }}</span>
                </div>
                <div class="grid grid-cols-3 gap-2 mb-3">
                  <div class="bg-base-300/30 rounded-lg p-2">
                    <div class="text-[9px] text-base-content/30 uppercase">Baseline</div>
                    <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.baseline_sample_size }}/{{ selectedEvalGateDecision.min_baseline_runs }}</div>
                  </div>
                  <div class="bg-base-300/30 rounded-lg p-2">
                    <div class="text-[9px] text-base-content/30 uppercase">Window</div>
                    <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.baseline_window }} runs</div>
                  </div>
                  <div class="bg-base-300/30 rounded-lg p-2">
                    <div class="text-[9px] text-base-content/30 uppercase">Reasons</div>
                    <div class="font-mono text-xs mt-1 truncate" :title="fmtReasonCodes(selectedEvalGateDecision.reason_codes)">{{ fmtReasonCodes(selectedEvalGateDecision.reason_codes) }}</div>
                  </div>
                </div>
                <div class="font-mono text-xs whitespace-pre-wrap text-base-content/60">{{ selectedEvalGateDecision.message }}</div>
              </div>

              <!-- Routing Recommendations -->
              <div v-if="selectedRoutingRecommendations.length" class="rounded-xl border border-info/30 bg-info/8 p-4 mb-4">
                <div class="flex items-center justify-between mb-3">
                  <div>
                    <div class="text-xs font-bold uppercase tracking-wider text-info">Adaptive Shadow</div>
                    <div class="font-mono text-xs mt-0.5 text-base-content/45">Advisory only — execution not changed.</div>
                  </div>
                  <span class="badge badge-sm badge-info">shadow-only</span>
                </div>
                <div class="space-y-3">
                  <div v-for="rec in selectedRoutingRecommendations" :key="rec.step_id" class="bg-base-300/25 rounded-xl p-3">
                    <div class="flex items-center justify-between gap-2 mb-2">
                      <div>
                        <div class="text-[9px] text-base-content/30 uppercase">Router Step</div>
                        <div class="font-mono text-xs mt-0.5">{{ rec.step_id }}</div>
                      </div>
                      <span class="badge badge-xs" :class="rec.fallback_used ? 'badge-warning' : 'badge-success'">{{ rec.fallback_used ? 'fallback' : 'recommended' }}</span>
                    </div>
                    <div class="grid grid-cols-3 gap-2 mb-2">
                      <div class="bg-base-200/50 rounded-lg p-2">
                        <div class="text-[9px] text-base-content/25">Current</div>
                        <div class="font-mono text-xs mt-0.5">{{ rec.current_route }}</div>
                      </div>
                      <div class="bg-base-200/50 rounded-lg p-2">
                        <div class="text-[9px] text-base-content/25">Recommended</div>
                        <div class="font-mono text-xs mt-0.5">{{ rec.recommended_route }}</div>
                      </div>
                      <div class="bg-base-200/50 rounded-lg p-2">
                        <div class="text-[9px] text-base-content/25">Confidence</div>
                        <div class="font-mono text-xs mt-0.5">{{ fmtConfidence(rec.confidence) }} · {{ rec.sample_size }}s</div>
                      </div>
                    </div>
                    <div class="font-mono text-xs whitespace-pre-wrap text-base-content/50">{{ rec.reason }}</div>
                  </div>
                </div>
              </div>

              <!-- Approval Pending -->
              <div v-if="pendingApproval" class="rounded-xl border border-warning/40 bg-warning/8 p-4 mb-4">
                <div class="flex items-center justify-between gap-3 mb-3">
                  <div>
                    <div class="text-xs font-bold uppercase tracking-wider text-warning">Approval Pending</div>
                    <div class="font-mono text-xs mt-0.5 text-base-content/50">{{ pendingApproval.step_id }} · {{ pendingApproval.agent }}</div>
                  </div>
                  <span class="badge badge-sm badge-warning">awaiting</span>
                </div>
                <div class="font-mono text-xs whitespace-pre-wrap text-base-content/60 bg-base-300/30 rounded-xl p-3 mb-3">{{ pendingApproval.content_preview }}</div>
                <template v-if="dashboardOwnsSelectedWorkflow">
                  <textarea
                    v-model="approvalEditContent"
                    class="textarea textarea-bordered textarea-sm w-full font-mono text-xs mb-3"
                    rows="5"
                    placeholder="Edited approval content"
                  />
                  <div class="flex items-center gap-2 flex-wrap">
                    <button class="btn btn-sm btn-success" @click="recordApproval('approve')">Approve</button>
                    <button class="btn btn-sm btn-warning" @click="recordApproval('edit')">Approve Edit</button>
                    <button class="btn btn-sm btn-error" @click="recordApproval('deny')">Deny</button>
                    <span v-if="approvalStatus" class="text-xs text-base-content/50 flex-1">{{ approvalStatus }}</span>
                  </div>
                </template>
                <div v-else class="text-xs text-base-content/50">{{ externalWorkflowControlNotice }}</div>
              </div>

              <!-- External Control Notice -->
              <div v-else-if="externalWorkflowControlNotice" class="rounded-xl border border-info/40 bg-info/8 p-4 mb-4">
                <div class="text-xs font-bold uppercase tracking-wider text-info mb-1.5">External Control</div>
                <div class="text-xs text-base-content/50">{{ externalWorkflowControlNotice }}</div>
              </div>

              <!-- Resume Ready -->
              <div v-else-if="canResumeSelectedRun" class="rounded-xl border border-info/40 bg-info/8 p-4 mb-4">
                <div class="flex items-start justify-between gap-3">
                  <div class="min-w-0">
                    <div class="text-xs font-bold uppercase tracking-wider text-info mb-1">Resume Ready</div>
                    <div v-if="m.runtime_mode === 'hosted-mcp'" class="text-xs text-base-content/50">
                      Approval recorded. Ask your AI assistant to call <code class="font-mono bg-base-200 px-1 py-0.5 rounded text-[10px]">agent007_workflow_next</code>.
                    </div>
                    <div v-else class="text-xs text-base-content/50">Approval recorded. Resume to continue execution.</div>
                  </div>
                  <button v-if="m.runtime_mode !== 'hosted-mcp'" class="btn btn-sm btn-info shrink-0" @click="resumeSelectedRun">Resume</button>
                </div>
                <div v-if="resumeStatus" class="text-xs text-base-content/50 mt-2">{{ resumeStatus }}</div>
              </div>

              <!-- Workflow Steps -->
              <div>
                <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Steps</div>
                <div class="space-y-2">
                  <div
                    v-for="step in selectedWorkflowState.steps || []"
                    :key="step.id"
                    class="rounded-xl border p-3 transition-colors"
                    :class="{
                      'border-base-content/10': step.status === 'pending',
                      'border-error/60 bg-error/6': step.status === 'running' && isStaleStep(step),
                      'border-info/40 bg-info/5': step.status === 'running' && !isStaleStep(step),
                      'border-success/40 bg-success/5': step.status === 'completed',
                      'border-error/40 bg-error/5': step.status === 'failed',
                      'border-warning/40 bg-warning/5': step.status === 'awaiting-approval' || step.status === 'skipped',
                    }"
                  >
                    <div class="flex items-center justify-between gap-2">
                      <span class="font-mono text-xs font-medium text-base-content/80">{{ step.id }}</span>
                      <div class="flex items-center gap-1.5">
                        <span v-if="step.status === 'running' && isStaleStep(step)" class="badge badge-xs badge-error">stale</span>
                        <span v-if="isSlowStep(step)" class="badge badge-xs badge-warning">slow</span>
                        <span v-if="stepDuration(step)" class="text-[10px] font-mono text-base-content/35">⏱ {{ stepDuration(step) }}</span>
                        <span class="badge badge-xs" :class="{
                          'badge-ghost': step.status === 'pending',
                          'badge-info': step.status === 'running' && !isStaleStep(step),
                          'badge-warning': step.status === 'awaiting-approval' || step.status === 'skipped',
                          'badge-success': step.status === 'completed',
                          'badge-error': step.status === 'failed' || (step.status === 'running' && isStaleStep(step)),
                        }">{{ step.status }}</span>
                      </div>
                    </div>
                    <div class="text-[10px] text-base-content/30 mt-1">{{ step.agent }} · {{ step.attempts }} attempt(s)</div>
                    <!-- Liveness for running steps -->
                    <div v-if="step.status === 'running'" class="mt-2 flex items-center gap-2 flex-wrap">
                      <template v-if="step.last_heartbeat_hint">
                        <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-mono"
                          :class="isStaleStep(step) ? 'bg-error/15 text-error/80' : 'bg-info/15 text-info/80'">
                          <span class="w-1.5 h-1.5 rounded-full animate-pulse"
                            :class="isStaleStep(step) ? 'bg-error' : 'bg-info'"></span>
                          {{ step.last_heartbeat_hint }}
                        </span>
                        <span class="text-[10px] font-mono text-base-content/25">{{ heartbeatAge(step) }}</span>
                      </template>
                      <template v-else>
                        <span class="text-[10px] font-mono text-base-content/25 italic">no heartbeat yet</span>
                      </template>
                    </div>
                    <!-- Step output (expandable) -->
                    <div v-if="step.output_preview" class="mt-2">
                      <div v-if="!expandedSteps.has(step.id)"
                        class="md-step-output text-xs bg-base-300/20 rounded-lg p-2 max-h-20 overflow-hidden"
                        v-html="renderMarkdown(step.output_preview)"
                      />
                      <div v-else-if="rawStepView.has(step.id)"
                        class="font-mono text-xs whitespace-pre-wrap text-base-content/55 bg-base-300/20 rounded-lg p-3 max-h-[500px] overflow-auto leading-relaxed"
                      >{{ stepFullOutput(step) }}</div>
                      <div v-else
                        class="md-step-output text-xs bg-base-300/20 rounded-lg p-3 max-h-[500px] overflow-auto"
                        v-html="renderMarkdown(stepFullOutput(step))"
                      />
                      <div class="flex items-center gap-3 mt-1">
                        <button
                          class="text-[10px] font-mono text-primary/60 hover:text-primary transition-colors"
                          @click="toggleStepExpand(step.id)"
                        >{{ expandedSteps.has(step.id) ? '▴ Collapse' : '▾ Expand' }}</button>
                        <button v-if="expandedSteps.has(step.id)"
                          class="text-[10px] font-mono text-base-content/25 hover:text-base-content/60 transition-colors"
                          @click="toggleRawStep(step.id)"
                        >{{ rawStepView.has(step.id) ? '◈ rendered' : '⌂ raw' }}</button>
                      </div>
                    </div>
                    <div v-if="step.error" class="text-xs text-error mt-2">{{ step.error }}</div>
                  </div>
                </div>
              </div>

            </div>
          </template>

          <!-- Recent Trace -->
          <div v-if="selectedRun.run.entries?.length">
            <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Recent Trace</div>
            <div class="space-y-2">
              <div
                v-for="entry in selectedRun.run.entries.slice(-10).reverse()"
                :key="`${entry.timestamp}-${entry.kind}`"
                class="bg-base-200/60 rounded-xl p-3 border border-base-content/6"
              >
                <div class="flex items-center justify-between gap-2 mb-1">
                  <span class="font-mono text-xs text-base-content/55">{{ entry.kind }}</span>
                  <span class="text-[10px] text-base-content/25 font-mono">{{ fmtLocalTime(entry.timestamp) }}</span>
                </div>
                <div class="font-mono text-[10px] text-base-content/40 whitespace-pre-wrap leading-relaxed">{{ JSON.stringify(entry.payload, null, 2) }}</div>
              </div>
            </div>
          </div>

        </div>
        <!-- /run detail -->

        <!-- Loading state -->
        <div v-else-if="expandedRunId" class="p-10 text-center text-base-content/25 font-mono text-sm">
          <div class="inline-flex items-center gap-2">
            <span class="w-1.5 h-1.5 rounded-full bg-primary animate-pulse"></span>
            loading run details…
          </div>
        </div>

      </div>
      <!-- /right panel -->

    </div>
    <!-- /master-detail body -->

    <!-- ── Task submission slide-over ─────────────────────────────────── -->

    <!-- Backdrop -->
    <Transition
      enter-active-class="transition-opacity duration-200"
      leave-active-class="transition-opacity duration-150"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div
        v-if="taskPanelOpen"
        class="fixed inset-0 bg-base-300/30 backdrop-blur-[1px] z-40"
        @click="taskPanelOpen = false"
      ></div>
    </Transition>

    <!-- Panel -->
    <Transition
      enter-active-class="transition-transform duration-300 ease-out"
      leave-active-class="transition-transform duration-200 ease-in"
      enter-from-class="translate-x-full"
      enter-to-class="translate-x-0"
      leave-from-class="translate-x-0"
      leave-to-class="translate-x-full"
    >
      <div
        v-if="taskPanelOpen"
        class="fixed top-0 right-0 h-full w-[42rem] max-w-[92vw] bg-base-100 border-l border-base-300 shadow-2xl z-50 flex flex-col"
      >
        <!-- Chat header -->
        <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
          <div class="flex items-center gap-3">
            <div class="w-7 h-7 rounded-xl bg-primary/15 flex items-center justify-center shrink-0">
              <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
              </svg>
            </div>
            <div>
              <div class="text-sm font-semibold text-base-content/80 leading-none mb-1">agent007</div>
              <div class="flex items-center gap-1.5">
                <span class="w-1.5 h-1.5 rounded-full shrink-0" :class="connected ? 'bg-success shadow-[0_0_4px_oklch(var(--su)/0.8)]' : 'bg-base-content/20'"></span>
                <span class="badge badge-xs font-mono" :class="{
                  'badge-warning': m.runtime_mode === 'hosted-mcp',
                  'badge-success': m.runtime_mode === 'standalone' || m.runtime_mode === 'local-ollama',
                  'badge-info': m.runtime_mode === 'dry-run',
                }">{{ m.runtime_mode || 'hosted-mcp' }}</span>
                <span class="text-[10px] font-mono text-base-content/30">{{ m.model_provider || '—' }}</span>
              </div>
            </div>
          </div>
          <div class="flex items-center gap-1">
            <button
              v-if="chatMessages.length"
              class="btn btn-ghost btn-xs font-mono text-[10px] text-base-content/30 hover:text-base-content/60"
              @click="chatMessages = []"
              title="Clear conversation"
            >clear</button>
            <button
              class="btn btn-ghost btn-sm btn-square text-base-content/30 hover:text-base-content"
              @click="taskPanelOpen = false"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
              </svg>
            </button>
          </div>
        </div>

        <!-- Chat messages area -->
        <div class="flex-1 overflow-auto flex flex-col" ref="chatScrollRef">

          <!-- Empty state -->
          <div v-if="!chatMessages.length" class="flex-1 flex flex-col items-center justify-center p-8 gap-6">
            <div class="text-center">
              <div class="w-12 h-12 rounded-2xl bg-primary/10 flex items-center justify-center mx-auto mb-3">
                <svg class="w-6 h-6 text-primary/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                </svg>
              </div>
              <div class="text-sm font-medium text-base-content/50 mb-1">What should agent007 do?</div>
              <div class="text-xs text-base-content/25 font-mono">Run skills, workflows, or any task</div>
            </div>

            <div v-if="m.recent_tasks?.length" class="w-full space-y-2">
              <div class="text-[10px] font-mono text-base-content/20 uppercase tracking-widest text-center mb-3">recent tasks</div>
              <button
                v-for="(t, i) in [...(m.recent_tasks || [])].reverse().slice(0, 4)"
                :key="i"
                class="w-full text-left px-4 py-3 rounded-xl border border-base-content/8 hover:border-primary/30 hover:bg-primary/5 transition-all group"
                @click="taskInput = t.task"
              >
                <div class="flex items-start gap-3">
                  <span class="w-1.5 h-1.5 rounded-full shrink-0 mt-1.5" :class="{
                    'bg-success': t.status === 'completed',
                    'bg-error': t.status === 'failed',
                    'bg-info animate-pulse': t.status === 'running',
                    'bg-base-content/15': !['completed','failed','running'].includes(t.status),
                  }"></span>
                  <span class="font-mono text-xs text-base-content/40 group-hover:text-base-content/65 transition-colors leading-relaxed line-clamp-2">{{ t.task }}</span>
                </div>
              </button>
            </div>
          </div>

          <!-- Conversation messages -->
          <div v-else class="flex flex-col gap-4 p-5">
            <template v-for="(msg, i) in chatMessages" :key="i">

              <!-- User message -->
              <div v-if="msg.role === 'user'" class="flex justify-end">
                <div class="max-w-[82%]">
                  <div class="bg-primary/15 border border-primary/20 rounded-2xl rounded-tr-sm px-4 py-3">
                    <p class="font-mono text-xs text-base-content/85 whitespace-pre-wrap leading-relaxed">{{ msg.content }}</p>
                  </div>
                </div>
              </div>

              <!-- Assistant message -->
              <div v-else class="flex items-start gap-3">
                <div class="w-6 h-6 rounded-lg bg-primary/15 flex items-center justify-center shrink-0 mt-0.5">
                  <svg class="w-3 h-3 text-primary/70" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                  </svg>
                </div>
                <div class="flex-1 min-w-0">
                  <div v-if="msg.status === 'running'" class="flex items-center gap-2 py-2">
                    <span class="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 0ms"></span>
                    <span class="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 150ms"></span>
                    <span class="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 300ms"></span>
                    <span class="text-xs font-mono text-base-content/30 ml-1">running…</span>
                  </div>
                  <template v-else>
                    <div
                      class="rounded-2xl rounded-tl-sm px-4 py-3 border"
                      :class="msg.status === 'error'
                        ? 'bg-error/8 border-error/20'
                        : 'bg-base-200 border-base-content/8'"
                    >
                      <p
                        class="font-mono text-xs whitespace-pre-wrap leading-relaxed"
                        :class="msg.status === 'error' ? 'text-error/80' : 'text-base-content/70'"
                      >{{ msg.content }}</p>
                    </div>
                    <div v-if="msg.sessionId" class="flex items-center gap-2 mt-1.5 px-1">
                      <span class="badge badge-xs" :class="msg.status === 'completed' ? 'badge-success' : 'badge-error'">
                        {{ msg.status }}
                      </span>
                      <span class="text-[10px] font-mono text-base-content/20">{{ msg.sessionId }}</span>
                    </div>
                  </template>
                </div>
              </div>

            </template>
          </div>
        </div>

        <!-- Input area -->
        <div class="shrink-0 border-t border-base-300 bg-base-200/60 p-4">

          <!-- Slash command menu -->
          <div
            v-if="showSlashMenu && filteredSlashCommands.length"
            class="mb-2 rounded-xl border border-base-300 bg-base-100 shadow-lg overflow-hidden"
          >
            <div class="px-3 py-1.5 border-b border-base-content/8 flex items-center justify-between">
              <span class="text-[10px] font-mono text-base-content/25 uppercase tracking-widest">Commands · ↑↓ navigate · ↵ select · Esc close</span>
              <span class="text-[10px] font-mono text-base-content/20">{{ filteredSlashCommands.length }} match{{ filteredSlashCommands.length !== 1 ? 'es' : '' }}</span>
            </div>
            <div ref="slashMenuRef" class="max-h-64 overflow-auto">
              <button
                v-for="(cmd, i) in filteredSlashCommands"
                :key="cmd.trigger"
                data-slash-item
                class="w-full text-left px-3 py-2.5 flex items-start gap-3 transition-colors"
                :class="i === slashMenuIndex ? 'bg-primary/10' : 'hover:bg-base-200'"
                @mouseenter="slashMenuIndex = i"
                @click="selectSlashCommand(cmd)"
              >
                <div
                  class="w-5 h-5 rounded flex items-center justify-center shrink-0 mt-0.5"
                  :class="cmd.type === 'workflow' ? 'bg-secondary/15' : 'bg-primary/15'"
                >
                  <svg v-if="cmd.type === 'workflow'" class="w-3 h-3 text-secondary/70" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M4 6h16M4 10h16M4 14h16M4 18h7"/>
                  </svg>
                  <svg v-else class="w-3 h-3 text-primary/70" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                  </svg>
                </div>
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="font-mono text-xs font-medium text-base-content/85">{{ cmd.trigger }}</span>
                    <span class="badge badge-xs" :class="cmd.type === 'workflow' ? 'badge-secondary' : 'badge-primary'" style="opacity:0.7">{{ cmd.type }}</span>
                    <span v-if="cmd.source === 'project'" class="badge badge-xs badge-ghost" style="opacity:0.6">proj</span>
                  </div>
                  <div v-if="cmd.description && cmd.name !== cmd.trigger" class="text-[11px] text-base-content/35 truncate mt-0.5">{{ cmd.description }}</div>
                </div>
              </button>
            </div>
          </div>

          <div class="flex items-end gap-2">
            <textarea
              v-model="taskInput"
              class="flex-1 rounded-xl border border-base-300 bg-base-100 px-4 py-3 font-mono text-sm resize-none focus:outline-none focus:border-primary/50 transition-colors leading-relaxed placeholder:text-base-content/20 max-h-36 min-h-[44px]"
              :placeholder="showSlashMenu ? '' : 'Ask agent007 something… or type / for commands'"
              rows="1"
              @input="onChatInput"
              @keydown="onChatKeydown"
              :disabled="chatPending"
            ></textarea>
            <button
              class="btn btn-primary btn-square shrink-0 self-end"
              :class="{ 'loading': chatPending }"
              @click="submitTask"
              :disabled="!taskInput.trim() || chatPending"
            >
              <svg v-if="!chatPending" class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.269 20.876L5.999 12zm0 0h7.5"/>
              </svg>
            </button>
          </div>
          <div class="flex items-center justify-between mt-2 px-1">
            <span class="text-[10px] font-mono text-base-content/15">↵ send · Shift↵ newline · / for commands</span>
            <span class="text-[10px] font-mono text-base-content/15" v-if="chatMessages.length">{{ chatMessages.filter(m => m.role === 'user').length }} task(s)</span>
          </div>
        </div>
      </div>
    </Transition>

  </div>
</template>

<style scoped>
/* ── Markdown step output — :deep() targets v-html injected content ── */
.md-step-output :deep(h1),
.md-step-output :deep(h2),
.md-step-output :deep(h3),
.md-step-output :deep(h4) {
  font-family: var(--font-mono, monospace);
  font-weight: 700;
  margin: 1.1em 0 0.3em;
  padding-bottom: 0.2em;
  border-bottom: 1px solid oklch(var(--b3));
}
.md-step-output :deep(h1) { font-size: 1.1rem; color: oklch(var(--p)); }
.md-step-output :deep(h2) { font-size: 0.95rem; color: oklch(var(--p) / 0.85); }
.md-step-output :deep(h3) { font-size: 0.85rem; color: oklch(var(--s)); border-bottom-color: oklch(var(--b3) / 0.5); }
.md-step-output :deep(h4) { font-size: 0.8rem; color: oklch(var(--bc) / 0.7); border-bottom: none; font-style: italic; }

.md-step-output :deep(p) {
  font-size: 0.8rem; line-height: 1.7;
  color: oklch(var(--bc) / 0.8); margin: 0.4em 0;
}
.md-step-output :deep(hr) { border: none; border-top: 1px solid oklch(var(--b3)); margin: 1em 0; }

.md-step-output :deep(code) {
  font-family: var(--font-mono, monospace); font-size: 0.75rem;
  background: oklch(var(--b3) / 0.8); color: oklch(var(--s));
  padding: 0.1em 0.35em; border-radius: 3px; border: 1px solid oklch(var(--b3));
}
.md-step-output :deep(pre) {
  border-radius: 6px; border: 1px solid oklch(var(--b3));
  overflow: hidden; margin: 0.7em 0; background: oklch(var(--b3) / 0.35);
}
.md-step-output :deep(pre code) {
  display: block; background: none; border: none;
  font-size: 0.75rem; line-height: 1.55; color: oklch(var(--bc) / 0.85);
  padding: 10px 14px; overflow-x: auto;
}

.md-step-output :deep(ul) { margin: 0.35em 0 0.35em 1.25em; list-style: disc; }
.md-step-output :deep(ol) { margin: 0.35em 0 0.35em 1.25em; list-style: decimal; }
.md-step-output :deep(li) { font-size: 0.8rem; line-height: 1.6; color: oklch(var(--bc) / 0.78); }

.md-step-output :deep(blockquote) {
  border-left: 2px solid oklch(var(--p) / 0.4);
  margin: 0.6em 0; padding: 3px 12px;
  color: oklch(var(--bc) / 0.55); font-style: italic;
}

.md-step-output :deep(a)       { color: oklch(var(--p)); text-decoration: underline; text-underline-offset: 2px; }
.md-step-output :deep(a:hover) { color: oklch(var(--s)); }

.md-step-output :deep(table) {
  width: 100%; border-collapse: collapse; font-size: 0.75rem;
  font-family: var(--font-mono, monospace); margin: 0.7em 0;
  border-radius: 5px; overflow: hidden; border: 1px solid oklch(var(--b3));
}
.md-step-output :deep(th) {
  background: oklch(var(--b3) / 0.7); color: oklch(var(--bc) / 0.55);
  font-size: 0.65rem; text-transform: uppercase; letter-spacing: 0.06em;
  padding: 5px 10px; text-align: left; border-bottom: 1px solid oklch(var(--b3));
}
.md-step-output :deep(td) {
  padding: 5px 10px; border-bottom: 1px solid oklch(var(--b3) / 0.5);
  color: oklch(var(--bc) / 0.78);
}
.md-step-output :deep(tr:last-child td) { border-bottom: none; }
.md-step-output :deep(tr:hover td)      { background: oklch(var(--b3) / 0.25); }

.md-step-output :deep(strong) { color: oklch(var(--bc)); font-weight: 700; }
.md-step-output :deep(em)     { color: oklch(var(--bc) / 0.75); font-style: italic; }
.md-step-output :deep(del)    { color: oklch(var(--bc) / 0.35); text-decoration: line-through; }

.md-step-output :deep(.artifact-mermaid-block),
.artifact-mermaid-block {
  margin: 0.75em 0;
  padding: 16px;
  border-radius: 8px;
  border: 1px solid oklch(var(--b3));
  background: oklch(var(--b3) / 0.25);
  display: flex;
  justify-content: center;
  overflow-x: auto;
}
.md-step-output :deep(.artifact-mermaid-block svg),
.artifact-mermaid-block :deep(svg) { max-width: 100%; height: auto; }
</style>
