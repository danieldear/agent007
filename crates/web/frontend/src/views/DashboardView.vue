<script setup>
import { ref, onMounted, onUnmounted, computed, watch, nextTick } from 'vue'
import { useApi } from '../composables/useApi.js'

const props = defineProps({ events: Array, connected: Boolean, stats: Object })
const { api } = useApi()

const health = ref(null)
const metrics = ref(null)
const taskInput = ref('')
const taskStatus = ref('')
const runs = ref([])
const selectedRun = ref(null)
const selectedRunId = ref(null)
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
  health.value = await api.health()
  metrics.value = await api.getStats() || null
  await refreshRuns()
  loadEtrCacheStats()
  refreshTimer = setInterval(async () => {
    try {
      metrics.value = await api.getStats() || metrics.value
      await refreshRuns()
    } catch {
      // Keep the last successful snapshot when background refresh fails.
    }
  }, 5000)
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

const m = computed(() => metrics.value || {
  active_agents: 0, running_tasks: 0, awaiting_approvals: 0, completed_tasks: 0, failed_tasks: 0,
  total_tokens: 0, estimated_usd: 0, avg_reward: 0, session_requests: 0,
  scorecard_run_count: 0, success_rate: 0, avg_cost_usd: 0, avg_latency_ms: 0, total_retries: 0, avg_retries_per_run: 0,
  feedback_count: 0, prompt_improvements: 0,
  skills_count: 0, workflows_count: 0, personas_count: 0, memory_keys: 0,
  started_at: null, local_execution_available: false, runtime_mode: 'hosted-mcp', model_provider: 'unknown',
  recent_tasks: [], recent_scorecards: [],
})

const uptime = computed(() => {
  if (!m.value.started_at) return '—'
  const diff = Math.floor((Date.now() - new Date(m.value.started_at).getTime()) / 1000)
  if (diff < 60) return `${diff}s`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ${diff % 60}s`
  return `${Math.floor(diff / 3600)}h ${Math.floor((diff % 3600) / 60)}m`
})

function fmtTokens(n) {
  if (!n) return '0'
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
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

async function refreshRuns() {
  runs.value = await api.listRuns() || []
  // Never auto-select — only refresh detail for the currently expanded run
  if (expandedRunId.value) {
    if (runs.value.find(r => r.id === expandedRunId.value)) {
      selectedRun.value = await api.getRunDetail(expandedRunId.value)
    } else {
      expandedRunId.value = null
      selectedRunId.value = null
      selectedRun.value = null
    }
  }
}

async function toggleRun(id) {
  if (expandedRunId.value === id) {
    expandedRunId.value = null
    selectedRunId.value = null
    selectedRun.value = null
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
  approvalEditContent.value = selectedRun.value?.workflow_state?.pending_approval?.content || ''
  resumeStatus.value = ''
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

  // Add user bubble immediately
  chatMessages.value.push({ role: 'user', content: input })
  // Add pending assistant bubble
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
        <span class="w-1.5 h-1.5 rounded-full" :class="connected ? 'bg-success shadow-[0_0_4px_theme(colors.success)]' : 'bg-error'"></span>
        <span class="text-[11px] font-mono" :class="connected ? 'text-success/60' : 'text-error/60'">{{ connected ? 'live' : 'offline' }}</span>
        <div class="w-px h-4 bg-base-300"></div>
        <button
          class="btn btn-sm btn-primary font-mono text-xs gap-1.5"
          @click="taskPanelOpen = true"
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/>
          </svg>
          New Task
        </button>
      </div>
    </div>

    <!-- ── Scrollable body ────────────────────────────────────────────── -->
    <div class="flex-1 overflow-auto p-4 space-y-4">

      <!-- Primary Stats: 4 hero cards -->
      <div class="grid grid-cols-4 gap-3">
        <div class="bg-base-200 rounded-xl p-5 border border-base-300 relative overflow-hidden">
          <div class="absolute inset-0 bg-gradient-to-br from-primary/8 to-transparent pointer-events-none"></div>
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1">Active Agents</div>
          <div class="text-4xl font-bold font-mono text-primary tabular-nums">{{ m.active_agents }}</div>
          <div v-if="m.active_agents > 0" class="absolute top-4 right-4 flex items-center gap-1.5">
            <span class="w-1.5 h-1.5 rounded-full bg-primary animate-pulse"></span>
            <span class="text-[10px] font-mono text-primary/50">active</span>
          </div>
          <div v-else class="text-[10px] font-mono text-base-content/25 mt-1">idle</div>
        </div>
        <div class="bg-base-200 rounded-xl p-5 border border-base-300 relative overflow-hidden">
          <div class="absolute inset-0 bg-gradient-to-br from-info/8 to-transparent pointer-events-none"></div>
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1">Running</div>
          <div class="text-4xl font-bold font-mono text-info tabular-nums">{{ m.running_tasks }}</div>
          <div v-if="m.running_tasks > 0" class="absolute top-4 right-4">
            <span class="w-1.5 h-1.5 rounded-full bg-info animate-pulse inline-block"></span>
          </div>
          <div class="text-[10px] font-mono text-base-content/25 mt-1">
            <template v-if="(m.awaiting_approvals || 0) > 0">
              {{ m.awaiting_approvals }} awaiting approval
            </template>
            <template v-else>
              {{ m.session_requests }} requests
            </template>
          </div>
        </div>
        <div class="bg-base-200 rounded-xl p-5 border border-base-300 relative overflow-hidden">
          <div class="absolute inset-0 bg-gradient-to-br from-success/8 to-transparent pointer-events-none"></div>
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1">Completed</div>
          <div class="text-4xl font-bold font-mono text-success tabular-nums">{{ m.completed_tasks }}</div>
          <div class="text-[10px] font-mono text-success/40 mt-1" v-if="m.success_rate">
            {{ ((m.success_rate || 0) * 100).toFixed(1) }}% success rate
          </div>
          <div class="text-[10px] font-mono text-base-content/25 mt-1" v-else>—</div>
        </div>
        <div class="bg-base-200 rounded-xl p-5 border border-base-300 relative overflow-hidden">
          <div class="absolute inset-0 bg-gradient-to-br from-error/8 to-transparent pointer-events-none"></div>
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1">Failed</div>
          <div class="text-4xl font-bold font-mono text-error tabular-nums">{{ m.failed_tasks }}</div>
          <div class="text-[10px] font-mono text-base-content/25 mt-1" v-if="m.total_retries">{{ m.total_retries }} retries</div>
          <div class="text-[10px] font-mono text-base-content/25 mt-1" v-else>—</div>
        </div>
      </div>

      <!-- Secondary Stats: compact 6-column KPI row -->
      <div class="grid grid-cols-6 gap-2">
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Tokens</div>
          <div class="text-base font-bold font-mono text-secondary tabular-nums mt-0.5">
            {{ fmtTokens(m.total_tokens) }}<span v-if="m.runtime_mode === 'hosted-mcp' && m.total_tokens > 0" class="text-[9px] text-base-content/25 ml-0.5" title="Estimated from prompt length">~</span>
          </div>
          <div class="text-[9px] text-base-content/25 font-mono mt-0.5">{{ m.session_requests }}req</div>
        </div>
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Est. Cost</div>
          <div class="text-base font-bold font-mono text-warning tabular-nums mt-0.5">${{ (m.estimated_usd || 0).toFixed(4) }}</div>
          <div class="text-[9px] text-base-content/25 font-mono mt-0.5">${{ (m.avg_cost_usd || 0).toFixed(4) }}/run</div>
        </div>
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Success</div>
          <div class="text-base font-bold font-mono text-success tabular-nums mt-0.5">{{ ((m.success_rate || 0) * 100).toFixed(1) }}%</div>
          <div class="text-[9px] text-base-content/25 font-mono mt-0.5">{{ m.scorecard_run_count || 0 }} scorecards</div>
        </div>
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Avg Latency</div>
          <div class="text-base font-bold font-mono text-info tabular-nums mt-0.5">{{ fmtMs(m.avg_latency_ms || 0) }}</div>
          <div class="text-[9px] text-base-content/25 font-mono mt-0.5">{{ (m.avg_retries_per_run || 0).toFixed(2) }} retries/run</div>
        </div>
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Avg Reward</div>
          <div class="text-base font-bold font-mono text-accent tabular-nums mt-0.5">{{ (m.avg_reward || 0).toFixed(3) }}</div>
          <div class="text-[9px] text-base-content/25 font-mono mt-0.5">{{ m.feedback_count }}fb · {{ m.prompt_improvements }}imp</div>
        </div>
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Uptime</div>
          <div class="text-base font-bold font-mono text-base-content tabular-nums mt-0.5">{{ uptime }}</div>
          <div class="text-[9px] text-base-content/25 font-mono mt-0.5 truncate">{{ m.model_provider || '—' }}</div>
        </div>
      </div>

      <!-- ETR Cache Stats row -->
      <div class="grid grid-cols-4 gap-2" v-if="etrCacheStats">
        <div class="bg-base-200 rounded-lg p-3 border border-warning/30 col-span-3 flex items-center gap-4">
          <span class="text-warning text-sm">⚡</span>
          <div>
            <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">ETR Step Cache</div>
            <div class="text-xs font-mono text-base-content mt-0.5">
              {{ etrCacheStats.entries ?? 0 }} entries
              <span class="text-base-content/30 mx-1">·</span>
              {{ etrCacheStats.size_bytes ? (etrCacheStats.size_bytes / 1024).toFixed(1) + ' KB' : '0 KB' }}
            </div>
          </div>
          <div class="ml-auto flex gap-3 text-center">
            <div>
              <div class="text-[9px] font-mono text-base-content/30">Hits</div>
              <div class="text-sm font-bold font-mono text-success">{{ etrCacheStats.hits ?? '—' }}</div>
            </div>
            <div>
              <div class="text-[9px] font-mono text-base-content/30">Misses</div>
              <div class="text-sm font-bold font-mono text-warning">{{ etrCacheStats.misses ?? '—' }}</div>
            </div>
            <div v-if="etrCacheStats.hits !== undefined && etrCacheStats.misses !== undefined && (etrCacheStats.hits + etrCacheStats.misses) > 0">
              <div class="text-[9px] font-mono text-base-content/30">Hit Rate</div>
              <div class="text-sm font-bold font-mono text-success">{{ ((etrCacheStats.hits / (etrCacheStats.hits + etrCacheStats.misses)) * 100).toFixed(0) }}%</div>
            </div>
          </div>
        </div>
        <div class="bg-base-200 rounded-lg p-3 border border-base-300/50 flex flex-col justify-center items-center gap-2">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">Cache Actions</div>
          <button class="btn btn-xs btn-warning btn-outline font-mono" @click="clearEtrCache">Clear Cache</button>
        </div>
      </div>

      <!-- Sparkline token trend (visible only when we have enough data) -->
      <div v-if="tokenSparkline.length >= 4" class="bg-base-200 rounded-lg px-4 py-2 border border-base-300/50 flex items-center gap-3">
        <span class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest shrink-0">Token Trend</span>
        <svg :width="160" :height="24" class="shrink-0 opacity-60">
          <polyline
            :points="sparklinePath(tokenSparkline, 160, 24)"
            fill="none"
            stroke="oklch(var(--s))"
            stroke-width="1.5"
            stroke-linejoin="round"
            stroke-linecap="round"
          />
        </svg>
        <span class="text-[9px] font-mono text-base-content/30">last {{ tokenSparkline.length }} tasks · latest {{ fmtTokens(tokenSparkline[tokenSparkline.length - 1]) }} tok</span>
      </div>

      <!-- Recent Tasks (session) -->
      <div class="bg-base-200 rounded-xl border border-base-300 flex flex-col" style="max-height: 26vh">
        <div class="px-4 py-2.5 border-b border-base-300 flex justify-between items-center shrink-0">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Recent Tasks</span>
          <span class="text-[10px] font-mono text-base-content/30">{{ m.recent_tasks?.length || 0 }} this session</span>
        </div>
        <div class="overflow-auto flex-1">
          <table class="table table-sm w-full" v-if="m.recent_tasks?.length">
            <thead class="sticky top-0 bg-base-200 z-10">
              <tr class="text-[10px] text-base-content/35 uppercase tracking-wider">
                <th class="w-[38%] font-medium">Task</th>
                <th class="w-[12%] font-medium">Mode</th>
                <th class="w-[15%] font-medium">Model</th>
                <th class="w-[10%] font-medium">Status</th>
                <th class="w-[9%] font-medium">Tokens</th>
                <th class="w-[16%] font-medium">Time</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(t, i) in [...(m.recent_tasks || [])].reverse()" :key="i" class="hover:bg-base-300/30 transition-colors">
                <td class="max-w-[0] truncate font-mono text-xs text-base-content/80" :title="t.task">{{ t.task }}</td>
                <td>
                  <span class="badge badge-xs font-mono" :class="{
                    'badge-warning': t.agent === 'hosted-mcp',
                    'badge-success': t.agent === 'standalone',
                    'badge-info': t.agent === 'dry-run',
                    'badge-ghost': !['hosted-mcp','standalone','dry-run'].includes(t.agent),
                  }">{{ t.agent || '—' }}</span>
                </td>
                <td class="text-xs font-mono text-base-content/50 truncate max-w-[0]" :title="t.model">{{ t.model || '—' }}</td>
                <td>
                  <span class="badge badge-xs" :class="{
                    'badge-info': t.status === 'running',
                    'badge-success': t.status === 'completed',
                    'badge-error': t.status === 'failed',
                    'badge-warning': t.status === 'awaiting-approval',
                  }">{{ t.status }}</span>
                </td>
                <td class="text-xs font-mono text-base-content/50">
                  <span v-if="t.tokens > 0">{{ fmtTokens(t.tokens) }}<span v-if="t.agent === 'hosted-mcp'" class="text-base-content/25">~</span></span>
                  <span v-else class="text-base-content/25">—</span>
                </td>
                <td class="text-[11px] text-base-content/40 font-mono">{{ fmtLocalTime(t.started_at) }}</td>
              </tr>
            </tbody>
          </table>
          <div v-else class="p-8 text-center text-base-content/30 text-sm font-mono">No tasks yet this session</div>
        </div>
      </div>

      <!-- Persisted Runs: accordion list -->
      <div class="bg-base-200 rounded-xl border border-base-300 overflow-hidden">
        <div class="px-4 py-2.5 border-b border-base-300 flex justify-between items-center">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Persisted Runs</span>
          <div class="flex items-center gap-3">
            <span class="text-[10px] font-mono text-base-content/30">{{ filteredRuns.length }}/{{ runs.length }} runs</span>
            <button class="btn btn-ghost btn-xs text-[10px] font-mono" @click="cleanupStaleApprovals">
              cleanup stale approvals
            </button>
            <button class="btn btn-ghost btn-xs text-[10px] font-mono" @click="refreshRuns">↺ refresh</button>
          </div>
        </div>
        <!-- Filter bar -->
        <div class="px-4 py-2 border-b border-base-300/40 flex items-center gap-2 flex-wrap">
          <input
            v-model="runFilter"
            class="input input-xs input-bordered font-mono text-[10px] w-40"
            placeholder="search runs…"
          />
          <button
            v-for="s in ['', 'running', 'completed', 'failed', 'awaiting_approval']"
            :key="s"
            class="btn btn-xs font-mono text-[10px]"
            :class="runStatusFilter === s ? 'btn-primary' : 'btn-ghost'"
            @click="runStatusFilter = s"
          >{{ s === '' ? 'all' : s.replace('_', ' ') }}</button>
        </div>
        <div v-if="cleanupStatus" class="px-4 py-2 text-[10px] font-mono text-base-content/45 border-b border-base-300/40">
          {{ cleanupStatus }}
        </div>

        <div v-if="filteredRuns.length" class="divide-y divide-base-300/40">
          <div v-for="run in filteredRuns" :key="run.id">
            <!-- Run row: click to expand/collapse -->
            <button
              class="w-full text-left px-4 py-3 hover:bg-base-300/20 transition-colors flex items-center gap-4 group"
              :class="expandedRunId === run.id ? 'bg-base-300/30' : ''"
              @click="toggleRun(run.id)"
            >
              <!-- Expand chevron -->
              <svg
                class="w-3.5 h-3.5 text-base-content/25 shrink-0 transition-transform duration-200"
                :class="expandedRunId === run.id ? 'rotate-90 text-primary/50' : 'group-hover:text-base-content/40'"
                fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"
              >
                <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7"/>
              </svg>
              <!-- Status dot -->
              <span class="w-2 h-2 rounded-full shrink-0" :class="{
                'bg-info animate-pulse': run.status === 'running',
                'bg-warning animate-pulse': run.status === 'awaiting-approval',
                'bg-success': run.status === 'succeeded',
                'bg-error': run.status === 'failed',
                'bg-base-content/20': !['running','awaiting-approval','succeeded','failed'].includes(run.status),
              }"></span>
              <!-- Kind -->
              <span class="font-mono text-xs text-base-content/60 shrink-0 w-44 truncate" :title="run.kind">{{ run.kind }}</span>
              <!-- Task -->
              <span class="font-mono text-xs text-base-content/50 flex-1 truncate" :title="run.task">{{ run.task }}</span>
              <!-- Mode / provider (hidden on small screens) -->
              <span class="text-[10px] font-mono text-base-content/25 shrink-0 hidden lg:block">{{ run.mode }} · {{ run.provider || 'hosted-mcp' }}</span>
              <!-- Status badge -->
              <span class="badge badge-xs shrink-0" :class="{
                'badge-info': run.status === 'running',
                'badge-warning': run.status === 'awaiting-approval',
                'badge-success': run.status === 'succeeded',
                'badge-error': run.status === 'failed',
              }">{{ run.status }}</span>
            </button>

            <!-- Accordion detail panel -->
            <div
              v-if="expandedRunId === run.id && selectedRun?.run"
              class="border-t border-base-300/40 bg-base-100/50 px-6 py-5 space-y-5"
            >
              <!-- Metadata grid -->
              <div class="grid grid-cols-4 gap-3">
                <div class="bg-base-200 rounded-lg p-3">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Run ID</div>
                  <div class="font-mono text-xs text-base-content/60 truncate" :title="selectedRun.run.metadata.id">{{ selectedRun.run.metadata.id }}</div>
                </div>
                <div class="bg-base-200 rounded-lg p-3">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Kind</div>
                  <div class="font-mono text-xs text-base-content/60 truncate">{{ selectedRun.run.metadata.kind }}</div>
                </div>
                <div class="bg-base-200 rounded-lg p-3">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Status</div>
                  <span class="badge badge-sm font-mono" :class="{
                    'badge-info': selectedRun.run.metadata.status === 'running',
                    'badge-warning': selectedRun.run.metadata.status === 'awaiting-approval',
                    'badge-success': selectedRun.run.metadata.status === 'succeeded',
                    'badge-error': selectedRun.run.metadata.status === 'failed',
                  }">{{ selectedRun.run.metadata.status }}</span>
                </div>
                <div class="bg-base-200 rounded-lg p-3">
                  <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Started</div>
                  <div class="font-mono text-xs text-base-content/60">{{ fmtLocalTime(selectedRun.run.metadata.started_at) }}</div>
                </div>
              </div>

              <!-- Task -->
              <div>
                <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Task</div>
                <div class="bg-base-200 rounded-lg p-4 font-mono text-xs whitespace-pre-wrap text-base-content/75 leading-relaxed">{{ selectedRun.run.metadata.task }}</div>
              </div>

              <!-- Output -->
              <div v-if="selectedRunOutput">
                <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Output</div>
                <div class="bg-base-200 rounded-lg p-4 font-mono text-xs whitespace-pre-wrap text-base-content/75 leading-relaxed max-h-48 overflow-auto">{{ selectedRunOutput }}</div>
              </div>

              <!-- Retrieval Telemetry -->
              <div v-if="selectedRetrievalTelemetry">
                <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Retrieval Telemetry</div>
                <div class="grid grid-cols-4 gap-3">
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Indexed Docs</div>
                    <div class="font-mono text-xs text-base-content/75">{{ selectedRetrievalTelemetry.indexed_docs || 0 }}</div>
                  </div>
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Hit Rate</div>
                    <div class="font-mono text-xs text-base-content/75">{{ fmtPct(selectedRetrievalTelemetry.retrieval_hit_rate || 0) }}</div>
                  </div>
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Queries/Hits</div>
                    <div class="font-mono text-xs text-base-content/75">{{ selectedRetrievalTelemetry.retrieval_queries || 0 }} / {{ selectedRetrievalTelemetry.retrieval_hits || 0 }}</div>
                  </div>
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Context Chars</div>
                    <div class="font-mono text-xs text-base-content/75">{{ selectedRetrievalTelemetry.rag_context_chars || 0 }}</div>
                  </div>
                </div>
                <div class="grid grid-cols-3 gap-3 mt-3">
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Vector Hits</div>
                    <div class="font-mono text-xs text-base-content/75">{{ selectedRetrievalTelemetry.vector_hits || 0 }}</div>
                  </div>
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Fallback Hits</div>
                    <div class="font-mono text-xs text-base-content/75">{{ selectedRetrievalTelemetry.fallback_hits || 0 }}</div>
                  </div>
                  <div class="bg-base-200 rounded-lg p-3">
                    <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Mock Embedding</div>
                    <div class="font-mono text-xs" :class="selectedRetrievalTelemetry.mock_embedding ? 'text-warning' : 'text-success'">
                      {{ selectedRetrievalTelemetry.mock_embedding ? 'yes' : 'no' }}
                    </div>
                  </div>
                </div>
              </div>

              <!-- Token Summary -->
              <div v-if="selectedRunTokenSummary" class="rounded-lg border border-base-300/60 bg-base-200 p-3">
                <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Token Summary</div>
                <div class="font-mono text-xs text-base-content/70">
                  {{ selectedRunTokenSummary.tokens || 0 }} tokens · {{ selectedRunTokenSummary.requests || 0 }} request(s)
                </div>
              </div>

              <!-- Persona Policy Warning -->
              <div v-if="selectedPersonaPolicyWarning" class="rounded-lg border border-warning/40 bg-warning/10 p-4">
                <div class="text-xs font-bold uppercase tracking-wider text-warning mb-2">Persona Tool Policy</div>
                <div class="font-mono text-xs text-base-content/70 whitespace-pre-wrap">
                  {{ selectedPersonaPolicyWarning.message || 'Tool policy warning recorded.' }}
                </div>
                <div class="font-mono text-[11px] text-base-content/50 mt-2">
                  persona={{ selectedPersonaPolicyWarning.active_persona || 'unknown' }} · tool={{ selectedPersonaPolicyWarning.requested_tool || 'unknown' }} · strict={{ selectedPersonaPolicyWarning.strict_mode ? 'on' : 'off' }}
                </div>
              </div>

              <!-- Workflow State -->
              <template v-if="selectedWorkflowState">
                <div>
                  <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-3">Workflow State</div>
                  <div class="grid grid-cols-4 gap-3 mb-4">
                    <div class="bg-base-200 rounded-lg p-3">
                      <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Workflow</div>
                      <div class="font-mono text-xs text-base-content/75">{{ selectedWorkflowState.workflow }}</div>
                    </div>
                    <div class="bg-base-200 rounded-lg p-3">
                      <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1.5">Progress · {{ selectedWorkflowState.steps_completed }}/{{ selectedWorkflowState.steps_total }}</div>
                      <progress class="progress progress-primary w-full h-1.5" :value="selectedWorkflowState.steps_completed" :max="selectedWorkflowState.steps_total || 1"></progress>
                    </div>
                    <div class="bg-base-200 rounded-lg p-3">
                      <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Budget Used</div>
                      <div class="font-mono text-xs text-base-content/75">{{ fmtTokens(selectedWorkflowState.budget_used?.tokens || 0) }} tokens</div>
                    </div>
                    <div class="bg-base-200 rounded-lg p-3">
                      <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-1">Artifacts</div>
                      <div class="font-mono text-xs text-base-content/75">{{ selectedRun.run.artifacts?.length || 0 }} file(s)</div>
                      <div class="text-[9px] font-mono text-base-content/30 mt-0.5 truncate" :title="selectedRun.run.artifacts?.join(', ')">{{ selectedRun.run.artifacts?.join(', ') || '—' }}</div>
                    </div>
                  </div>

                  <!-- Eval Gate -->
                  <div
                    v-if="selectedEvalGateDecision"
                    class="rounded-lg p-4 mb-4 border"
                    :class="{
                      'border-success/40 bg-success/10': selectedEvalGateDecision.decision === 'pass',
                      'border-warning/40 bg-warning/10': selectedEvalGateDecision.decision === 'warn',
                      'border-error/40 bg-error/10': selectedEvalGateDecision.decision === 'block',
                    }"
                  >
                    <div class="flex items-center justify-between gap-3 mb-3">
                      <div>
                        <div class="text-xs font-bold uppercase tracking-wider" :class="{
                          'text-success': selectedEvalGateDecision.decision === 'pass',
                          'text-warning': selectedEvalGateDecision.decision === 'warn',
                          'text-error': selectedEvalGateDecision.decision === 'block',
                        }">Eval Gate</div>
                        <div class="font-mono text-xs mt-1 text-base-content/55">{{ selectedEvalGateDecision.workflow }} · {{ selectedEvalGateDecision.mode }}</div>
                      </div>
                      <span class="badge badge-sm" :class="{
                        'badge-success': selectedEvalGateDecision.decision === 'pass',
                        'badge-warning': selectedEvalGateDecision.decision === 'warn',
                        'badge-error': selectedEvalGateDecision.decision === 'block',
                      }">{{ selectedEvalGateDecision.decision }}</span>
                    </div>
                    <div class="grid grid-cols-3 gap-2 mb-3">
                      <div class="bg-base-300/30 rounded p-2">
                        <div class="text-[9px] text-base-content/35 uppercase">Baseline</div>
                        <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.baseline_sample_size }}/{{ selectedEvalGateDecision.min_baseline_runs }}</div>
                      </div>
                      <div class="bg-base-300/30 rounded p-2">
                        <div class="text-[9px] text-base-content/35 uppercase">Window</div>
                        <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.baseline_window }} runs</div>
                      </div>
                      <div class="bg-base-300/30 rounded p-2">
                        <div class="text-[9px] text-base-content/35 uppercase">Reasons</div>
                        <div class="font-mono text-xs mt-1 truncate" :title="fmtReasonCodes(selectedEvalGateDecision.reason_codes)">{{ fmtReasonCodes(selectedEvalGateDecision.reason_codes) }}</div>
                      </div>
                    </div>
                    <div class="font-mono text-xs whitespace-pre-wrap text-base-content/65">{{ selectedEvalGateDecision.message }}</div>
                  </div>

                  <!-- Routing Recommendations -->
                  <div v-if="selectedRoutingRecommendations.length" class="rounded-lg border border-info/30 bg-info/10 p-4 mb-4">
                    <div class="flex items-center justify-between mb-3">
                      <div>
                        <div class="text-xs font-bold uppercase tracking-wider text-info">Adaptive Shadow</div>
                        <div class="font-mono text-xs mt-0.5 text-base-content/50">Advisory only — execution not changed.</div>
                      </div>
                      <span class="badge badge-sm badge-info">shadow-only</span>
                    </div>
                    <div class="space-y-3">
                      <div v-for="rec in selectedRoutingRecommendations" :key="rec.step_id" class="bg-base-300/25 rounded-lg p-3">
                        <div class="flex items-center justify-between gap-2 mb-2">
                          <div>
                            <div class="text-[9px] text-base-content/35 uppercase">Router Step</div>
                            <div class="font-mono text-xs mt-0.5">{{ rec.step_id }}</div>
                          </div>
                          <span class="badge badge-xs" :class="rec.fallback_used ? 'badge-warning' : 'badge-success'">{{ rec.fallback_used ? 'fallback' : 'recommended' }}</span>
                        </div>
                        <div class="grid grid-cols-3 gap-2 mb-2">
                          <div class="bg-base-200/50 rounded p-2">
                            <div class="text-[9px] text-base-content/30">Current</div>
                            <div class="font-mono text-xs mt-0.5">{{ rec.current_route }}</div>
                          </div>
                          <div class="bg-base-200/50 rounded p-2">
                            <div class="text-[9px] text-base-content/30">Recommended</div>
                            <div class="font-mono text-xs mt-0.5">{{ rec.recommended_route }}</div>
                          </div>
                          <div class="bg-base-200/50 rounded p-2">
                            <div class="text-[9px] text-base-content/30">Confidence</div>
                            <div class="font-mono text-xs mt-0.5">{{ fmtConfidence(rec.confidence) }} · {{ rec.sample_size }}s</div>
                          </div>
                        </div>
                        <div class="font-mono text-xs whitespace-pre-wrap text-base-content/55">{{ rec.reason }}</div>
                      </div>
                    </div>
                  </div>

                  <!-- Approval Pending -->
                  <div v-if="pendingApproval" class="rounded-lg border border-warning/40 bg-warning/10 p-4 mb-4">
                    <div class="flex items-center justify-between gap-3 mb-3">
                      <div>
                        <div class="text-xs font-bold uppercase tracking-wider text-warning">Approval Pending</div>
                        <div class="font-mono text-xs mt-0.5 text-base-content/55">{{ pendingApproval.step_id }} · {{ pendingApproval.agent }}</div>
                      </div>
                      <span class="badge badge-sm badge-warning">awaiting</span>
                    </div>
                    <div class="font-mono text-xs whitespace-pre-wrap text-base-content/65 bg-base-300/30 rounded-lg p-3 mb-3">{{ pendingApproval.content_preview }}</div>
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
                        <span v-if="approvalStatus" class="text-xs text-base-content/55 flex-1">{{ approvalStatus }}</span>
                      </div>
                    </template>
                    <div v-else class="text-xs text-base-content/55">{{ externalWorkflowControlNotice }}</div>
                  </div>

                  <!-- External Control Notice -->
                  <div v-else-if="externalWorkflowControlNotice" class="rounded-lg border border-info/40 bg-info/10 p-4 mb-4">
                    <div class="text-xs font-bold uppercase tracking-wider text-info mb-1.5">External Control</div>
                    <div class="text-xs text-base-content/55">{{ externalWorkflowControlNotice }}</div>
                  </div>

                  <!-- Resume Ready -->
                  <div v-else-if="canResumeSelectedRun" class="rounded-lg border border-info/40 bg-info/10 p-4 mb-4">
                    <div class="flex items-start justify-between gap-3">
                      <div class="min-w-0">
                        <div class="text-xs font-bold uppercase tracking-wider text-info mb-1">Resume Ready</div>
                        <div v-if="m.runtime_mode === 'hosted-mcp'" class="text-xs text-base-content/55">
                          Approval recorded. Ask your AI assistant to call <code class="font-mono bg-base-200 px-1 py-0.5 rounded text-[10px]">agent007_workflow_next</code>.
                        </div>
                        <div v-else class="text-xs text-base-content/55">Approval recorded. Resume to continue execution.</div>
                      </div>
                      <button v-if="m.runtime_mode !== 'hosted-mcp'" class="btn btn-sm btn-info shrink-0" @click="resumeSelectedRun">Resume</button>
                    </div>
                    <div v-if="resumeStatus" class="text-xs text-base-content/55 mt-2">{{ resumeStatus }}</div>
                  </div>

                  <!-- Workflow Steps -->
                  <div>
                    <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35 mb-2">Steps</div>
                    <div class="space-y-2">
                      <div
                        v-for="step in selectedWorkflowState.steps || []"
                        :key="step.id"
                        class="rounded-lg border p-3 transition-colors"
                        :class="{
                          'border-base-300/60': step.status === 'pending',
                          'border-error/60 bg-error/8': step.status === 'running' && isStaleStep(step),
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
                            <span v-if="stepDuration(step)" class="text-[10px] font-mono text-base-content/40">⏱ {{ stepDuration(step) }}</span>
                            <span class="badge badge-xs" :class="{
                              'badge-ghost': step.status === 'pending',
                              'badge-info': step.status === 'running' && !isStaleStep(step),
                              'badge-warning': step.status === 'awaiting-approval' || step.status === 'skipped',
                              'badge-success': step.status === 'completed',
                              'badge-error': step.status === 'failed' || (step.status === 'running' && isStaleStep(step)),
                            }">{{ step.status }}</span>
                          </div>
                        </div>
                        <div class="text-[10px] text-base-content/35 mt-1">{{ step.agent }} · {{ step.attempts }} attempt(s)</div>
                        <!-- Liveness row for running steps -->
                        <div v-if="step.status === 'running'" class="mt-2 flex items-center gap-2 flex-wrap">
                          <template v-if="step.last_heartbeat_hint">
                            <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-mono"
                              :class="isStaleStep(step) ? 'bg-error/15 text-error/80' : 'bg-info/15 text-info/80'">
                              <span class="w-1.5 h-1.5 rounded-full animate-pulse"
                                :class="isStaleStep(step) ? 'bg-error' : 'bg-info'"></span>
                              {{ step.last_heartbeat_hint }}
                            </span>
                            <span class="text-[10px] font-mono text-base-content/30">{{ heartbeatAge(step) }}</span>
                          </template>
                          <template v-else>
                            <span class="text-[10px] font-mono text-base-content/30 italic">no heartbeat yet</span>
                          </template>
                        </div>
                        <!-- Step output (expandable) -->
                        <div v-if="step.output_preview" class="mt-2">
                          <div
                            class="font-mono text-xs whitespace-pre-wrap text-base-content/60 bg-base-300/20 rounded p-2 transition-all"
                            :class="expandedSteps.has(step.id) ? 'max-h-[500px] overflow-auto' : 'max-h-20 overflow-hidden'"
                          >{{ expandedSteps.has(step.id) ? stepFullOutput(step) : step.output_preview }}</div>
                          <button
                            class="mt-1 text-[10px] font-mono text-primary/60 hover:text-primary transition-colors"
                            @click="toggleStepExpand(step.id)"
                          >{{ expandedSteps.has(step.id) ? '▴ Collapse' : '▾ Expand' }}</button>
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
                    class="bg-base-200 rounded-lg p-3"
                  >
                    <div class="flex items-center justify-between gap-2 mb-1">
                      <span class="font-mono text-xs text-base-content/60">{{ entry.kind }}</span>
                      <span class="text-[10px] text-base-content/30 font-mono">{{ fmtLocalTime(entry.timestamp) }}</span>
                    </div>
                    <div class="font-mono text-[10px] text-base-content/45 whitespace-pre-wrap leading-relaxed">{{ JSON.stringify(entry.payload, null, 2) }}</div>
                  </div>
                </div>
              </div>

            </div>
            <!-- /accordion detail -->
          </div>
        </div>
        <div v-else class="p-10 text-center text-base-content/30 text-sm font-mono">No persisted runs yet</div>
      </div>

    </div>
    <!-- /scrollable body -->

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
            <div class="w-7 h-7 rounded-lg bg-primary/15 flex items-center justify-center shrink-0">
              <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
              </svg>
            </div>
            <div>
              <div class="text-sm font-semibold text-base-content/80 leading-none mb-1">agent007</div>
              <div class="flex items-center gap-1.5">
                <span class="w-1.5 h-1.5 rounded-full shrink-0" :class="connected ? 'bg-success' : 'bg-base-content/20'"></span>
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
              class="btn btn-ghost btn-xs font-mono text-[10px] text-base-content/35 hover:text-base-content/60"
              @click="chatMessages = []"
              title="Clear conversation"
            >clear</button>
            <button
              class="btn btn-ghost btn-sm btn-square text-base-content/35 hover:text-base-content"
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

          <!-- Empty state: show suggested prompts -->
          <div v-if="!chatMessages.length" class="flex-1 flex flex-col items-center justify-center p-8 gap-6">
            <div class="text-center">
              <div class="w-12 h-12 rounded-2xl bg-primary/10 flex items-center justify-center mx-auto mb-3">
                <svg class="w-6 h-6 text-primary/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                </svg>
              </div>
              <div class="text-sm font-medium text-base-content/50 mb-1">What should agent007 do?</div>
              <div class="text-xs text-base-content/30 font-mono">Run skills, workflows, or any task</div>
            </div>

            <!-- Suggested prompts from recent tasks -->
            <div v-if="m.recent_tasks?.length" class="w-full space-y-2">
              <div class="text-[10px] font-mono text-base-content/25 uppercase tracking-widest text-center mb-3">recent tasks</div>
              <button
                v-for="(t, i) in [...(m.recent_tasks || [])].reverse().slice(0, 4)"
                :key="i"
                class="w-full text-left px-4 py-3 rounded-xl border border-base-300/50 hover:border-primary/30 hover:bg-primary/5 transition-all group"
                @click="taskInput = t.task"
              >
                <div class="flex items-start gap-3">
                  <span class="w-1.5 h-1.5 rounded-full shrink-0 mt-1.5" :class="{
                    'bg-success': t.status === 'completed',
                    'bg-error': t.status === 'failed',
                    'bg-info animate-pulse': t.status === 'running',
                    'bg-base-content/15': !['completed','failed','running'].includes(t.status),
                  }"></span>
                  <span class="font-mono text-xs text-base-content/45 group-hover:text-base-content/70 transition-colors leading-relaxed line-clamp-2">{{ t.task }}</span>
                </div>
              </button>
            </div>
          </div>

          <!-- Conversation messages -->
          <div v-else class="flex flex-col gap-4 p-5">
            <template v-for="(msg, i) in chatMessages" :key="i">

              <!-- User message: right-aligned bubble -->
              <div v-if="msg.role === 'user'" class="flex justify-end">
                <div class="max-w-[82%]">
                  <div class="bg-primary/15 border border-primary/20 rounded-2xl rounded-tr-sm px-4 py-3">
                    <p class="font-mono text-xs text-base-content/85 whitespace-pre-wrap leading-relaxed">{{ msg.content }}</p>
                  </div>
                </div>
              </div>

              <!-- Assistant message: left-aligned -->
              <div v-else class="flex items-start gap-3">
                <!-- Avatar -->
                <div class="w-6 h-6 rounded-lg bg-primary/15 flex items-center justify-center shrink-0 mt-0.5">
                  <svg class="w-3 h-3 text-primary/70" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                  </svg>
                </div>

                <div class="flex-1 min-w-0">
                  <!-- Thinking indicator -->
                  <div v-if="msg.status === 'running'" class="flex items-center gap-2 py-2">
                    <span class="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 0ms"></span>
                    <span class="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 150ms"></span>
                    <span class="w-1.5 h-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 300ms"></span>
                    <span class="text-xs font-mono text-base-content/35 ml-1">running…</span>
                  </div>

                  <!-- Response content -->
                  <template v-else>
                    <div
                      class="rounded-2xl rounded-tl-sm px-4 py-3 border"
                      :class="msg.status === 'error'
                        ? 'bg-error/10 border-error/25'
                        : 'bg-base-200 border-base-300/50'"
                    >
                      <p
                        class="font-mono text-xs whitespace-pre-wrap leading-relaxed"
                        :class="msg.status === 'error' ? 'text-error/80' : 'text-base-content/75'"
                      >{{ msg.content }}</p>
                    </div>

                    <!-- Session metadata pill -->
                    <div v-if="msg.sessionId" class="flex items-center gap-2 mt-1.5 px-1">
                      <span class="badge badge-xs" :class="msg.status === 'completed' ? 'badge-success' : 'badge-error'">
                        {{ msg.status }}
                      </span>
                      <span class="text-[10px] font-mono text-base-content/25">{{ msg.sessionId }}</span>
                    </div>
                  </template>
                </div>
              </div>

            </template>
          </div>
        </div>

        <!-- Input area: always pinned at bottom -->
        <div class="shrink-0 border-t border-base-300 bg-base-200/60 p-4">

          <!-- Slash command menu (floats above input) -->
          <div
            v-if="showSlashMenu && filteredSlashCommands.length"
            class="mb-2 rounded-xl border border-base-300 bg-base-100 shadow-lg overflow-hidden"
          >
            <div class="px-3 py-1.5 border-b border-base-300/50 flex items-center justify-between">
              <span class="text-[10px] font-mono text-base-content/30 uppercase tracking-widest">Commands · ↑↓ navigate · ↵ select · Esc close</span>
              <span class="text-[10px] font-mono text-base-content/25">{{ filteredSlashCommands.length }} match{{ filteredSlashCommands.length !== 1 ? 'es' : '' }}</span>
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
                <!-- Type icon -->
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
                <!-- Content -->
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="font-mono text-xs font-medium text-base-content/85">{{ cmd.trigger }}</span>
                    <span class="badge badge-xs" :class="cmd.type === 'workflow' ? 'badge-secondary' : 'badge-primary'" style="opacity:0.7">{{ cmd.type }}</span>
                    <span v-if="cmd.source === 'project'" class="badge badge-xs badge-ghost" style="opacity:0.6">proj</span>
                  </div>
                  <div v-if="cmd.description && cmd.name !== cmd.trigger" class="text-[11px] text-base-content/40 truncate mt-0.5">{{ cmd.description }}</div>
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
            <span class="text-[10px] font-mono text-base-content/20">↵ send · Shift↵ newline · / for commands</span>
            <span class="text-[10px] font-mono text-base-content/20" v-if="chatMessages.length">{{ chatMessages.filter(m => m.role === 'user').length }} task(s)</span>
          </div>
        </div>
      </div>
    </Transition>

  </div>
</template>
