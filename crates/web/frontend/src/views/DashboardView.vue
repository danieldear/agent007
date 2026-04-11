<script setup>
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
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
let refreshTimer = null

onMounted(async () => {
  health.value = await api.health()
  metrics.value = await api.getStats() || null
  await refreshRuns()
  refreshTimer = setInterval(async () => {
    try {
      metrics.value = await api.getStats() || metrics.value
      await refreshRuns()
      if (selectedRunId.value) {
        selectedRun.value = await api.getRunDetail(selectedRunId.value)
      }
    } catch {
      // Keep the last successful snapshot when background refresh fails.
    }
  }, 5000)
})

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})

watch(() => props.stats, (v) => {
  if (v) metrics.value = v
})

const m = computed(() => metrics.value || {
  active_agents: 0, running_tasks: 0, completed_tasks: 0, failed_tasks: 0,
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

const recentEvents = computed(() => [...(props.events || [])].reverse().slice(0, 50))
const selectedWorkflowState = computed(() => selectedRun.value?.workflow_state || null)
const selectedEvalGateDecision = computed(() => selectedWorkflowState.value?.eval_gate_decision || null)
const pendingApproval = computed(() => selectedWorkflowState.value?.pending_approval || null)
const selectedRunArtifacts = computed(() => selectedRun.value?.run?.artifacts || [])
const selectedRunAlreadyResumed = computed(() => selectedRunArtifacts.value.includes('resume-target.json'))
const selectedRunHasApprovalDecision = computed(() => {
  const decisions = selectedWorkflowState.value?.approval_decisions || {}
  return Object.keys(decisions).length > 0
})
const canResumeSelectedRun = computed(() => {
  const run = selectedRun.value?.run?.metadata
  if (!run || !selectedWorkflowState.value || pendingApproval.value) return false
  if (selectedRunAlreadyResumed.value) return false
  if (!selectedRunHasApprovalDecision.value) return false
  return run.status === 'awaiting-approval' || selectedWorkflowState.value.status === 'running'
})

function fmtReasonCodes(codes) {
  if (!Array.isArray(codes) || !codes.length) return '—'
  return codes.join(', ')
}

async function refreshRuns() {
  runs.value = await api.listRuns() || []
  if (!selectedRunId.value && runs.value.length) {
    await selectRun(runs.value[0].id)
  } else if (selectedRunId.value && !runs.value.find((run) => run.id === selectedRunId.value)) {
    selectedRunId.value = null
    selectedRun.value = null
  }
}

async function selectRun(id) {
  selectedRunId.value = id
  selectedRun.value = await api.getRunDetail(id)
  approvalEditContent.value = selectedRun.value?.workflow_state?.pending_approval?.content || ''
  resumeStatus.value = ''
}

async function recordApproval(decision) {
  if (!selectedRunId.value || !pendingApproval.value) return
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

async function submitTask() {
  if (!taskInput.value.trim()) return
  taskStatus.value = 'Submitting...'
  try {
    const response = await api.runTask(taskInput.value.trim())
    taskStatus.value = response?.session ? `Submitted as ${response.session}` : 'Submitted'
    taskInput.value = ''
    setTimeout(async () => {
      await refreshRuns()
      if (response?.session) {
        await selectRun(response.session)
      }
    }, 800)
  } catch (e) {
    taskStatus.value = `Error: ${e.message}`
  }
  setTimeout(() => { taskStatus.value = '' }, 4000)
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Dashboard</span>
      <div class="flex items-center gap-2">
        <span class="w-1.5 h-1.5 rounded-full" :class="connected ? 'bg-success shadow-[0_0_4px_theme(colors.success)]' : 'bg-error'"></span>
        <span class="text-[11px] font-mono" :class="connected ? 'text-success/60' : 'text-error/60'">{{ connected ? 'live' : 'offline' }}</span>
      </div>
    </div>

    <div class="flex-1 overflow-auto p-4 space-y-4">

      <!-- Stats Row 1: Runtime -->
      <div class="grid grid-cols-4 gap-3">
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-primary/60 relative overflow-hidden">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Active Agents</div>
          <div class="text-3xl font-bold font-mono text-primary tabular-nums">{{ m.active_agents }}</div>
          <div v-if="m.active_agents > 0" class="absolute top-3 right-3 w-1.5 h-1.5 rounded-full bg-primary animate-pulse" />
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-info/60 relative overflow-hidden">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Running Tasks</div>
          <div class="text-3xl font-bold font-mono text-info tabular-nums">{{ m.running_tasks }}</div>
          <div v-if="m.running_tasks > 0" class="absolute top-3 right-3 w-1.5 h-1.5 rounded-full bg-info animate-pulse" />
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-success/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Completed</div>
          <div class="text-3xl font-bold font-mono text-success tabular-nums">{{ m.completed_tasks }}</div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-error/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Failed</div>
          <div class="text-3xl font-bold font-mono text-error tabular-nums">{{ m.failed_tasks }}</div>
        </div>
      </div>

      <!-- Stats Row 2: Tokens, Cost, Reward, Uptime -->
      <div class="grid grid-cols-4 gap-3">
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-secondary/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Total Tokens</div>
          <div class="flex items-baseline gap-2">
            <div class="text-3xl font-bold font-mono text-secondary tabular-nums">{{ fmtTokens(m.total_tokens) }}</div>
            <span v-if="m.runtime_mode === 'hosted-mcp' && m.total_tokens > 0" class="text-[10px] font-mono text-base-content/30" title="Estimated from prompt length (chars ÷ 4)">est</span>
          </div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1.5" v-if="m.runtime_mode === 'hosted-mcp'">
            {{ m.session_requests }}req ·
            <span :title="'Set AGENT007_HOST_MODEL env var to specify your model'">{{ m.model_provider || '?' }}</span>
          </div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1.5" v-else>{{ m.session_requests }} requests</div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-warning/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Est. Cost</div>
          <div class="flex items-baseline gap-2">
            <div class="text-3xl font-bold font-mono text-warning tabular-nums">${{ (m.estimated_usd || 0).toFixed(4) }}</div>
            <span v-if="m.runtime_mode === 'hosted-mcp' && m.estimated_usd > 0" class="text-[10px] font-mono text-base-content/30" title="Rough estimate at $0.002/1k tokens">~</span>
          </div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1.5" v-if="m.runtime_mode === 'hosted-mcp'">
            <span title="Set AGENT007_HOST_MODEL env var for accurate tracking">set HOST_MODEL for accuracy</span>
          </div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-accent/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Avg Reward</div>
          <div class="text-3xl font-bold font-mono text-accent tabular-nums">{{ (m.avg_reward || 0).toFixed(3) }}</div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1.5">{{ m.feedback_count }}fb · {{ m.prompt_improvements }}imp</div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-base-content/20">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Uptime</div>
          <div class="text-3xl font-bold font-mono text-base-content tabular-nums">{{ uptime }}</div>
        </div>
      </div>

      <!-- Stats Row 3: Scorecard KPIs -->
      <div class="grid grid-cols-4 gap-3">
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-success/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Success Rate</div>
          <div class="text-3xl font-bold font-mono text-success tabular-nums">{{ ((m.success_rate || 0) * 100).toFixed(1) }}%</div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1.5">{{ m.scorecard_run_count || 0 }} scorecards</div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-info/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Avg Latency</div>
          <div class="text-3xl font-bold font-mono text-info tabular-nums">{{ fmtMs(m.avg_latency_ms || 0) }}</div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-warning/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Avg Cost / Run</div>
          <div class="text-3xl font-bold font-mono text-warning tabular-nums">${{ (m.avg_cost_usd || 0).toFixed(4) }}</div>
        </div>
        <div class="bg-base-200 rounded-lg p-4 border-l-2 border-secondary/60">
          <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2">Avg Retries</div>
          <div class="text-3xl font-bold font-mono text-secondary tabular-nums">{{ (m.avg_retries_per_run || 0).toFixed(2) }}</div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1.5">{{ m.total_retries || 0 }} total retries</div>
        </div>
      </div>

      <!-- System Status -->
      <div class="bg-base-200 rounded-lg p-4">
        <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-3 flex items-center gap-2">
          <span class="w-3 h-px bg-base-content/20"></span>
          System Status
          <span class="flex-1 h-px bg-base-content/10"></span>
        </div>
        <div class="flex flex-wrap gap-x-8 gap-y-2 text-sm">
          <div class="flex items-center gap-2">
            <span class="text-[11px] font-mono text-base-content/40">exec</span>
            <span class="badge badge-sm font-mono" :class="{
              'badge-warning': m.runtime_mode === 'hosted-mcp',
              'badge-success': m.runtime_mode === 'standalone' || m.runtime_mode === 'local-ollama',
              'badge-info': m.runtime_mode === 'dry-run',
            }">
              {{ m.runtime_mode || 'hosted-mcp' }}
            </span>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[11px] font-mono text-base-content/40">model</span>
            <span class="font-mono text-xs text-base-content/70">{{ m.model_provider || '—' }}</span>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[11px] font-mono text-base-content/40">ws</span>
            <span class="badge badge-sm font-mono" :class="connected ? 'badge-success' : 'badge-error'">{{ connected ? 'live' : 'off' }}</span>
          </div>
        </div>
        <div class="my-3 h-px bg-base-300/60"></div>
        <div class="flex flex-wrap gap-x-6 gap-y-2">
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] font-mono text-base-content/40">skills</span>
            <span class="text-[11px] font-mono font-bold text-primary/80">{{ m.skills_count }}</span>
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] font-mono text-base-content/40">workflows</span>
            <span class="text-[11px] font-mono font-bold text-secondary/80">{{ m.workflows_count }}</span>
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] font-mono text-base-content/40">personas</span>
            <span class="text-[11px] font-mono font-bold text-accent/80">{{ m.personas_count }}</span>
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] font-mono text-base-content/40">mem-keys</span>
            <span class="text-[11px] font-mono font-bold text-info/80">{{ m.memory_keys }}</span>
          </div>
        </div>
      </div>

      <!-- Recent Tasks -->
      <div class="bg-base-200 rounded-lg flex flex-col" style="max-height: 36vh">
        <div class="px-4 py-2.5 border-b border-base-300 flex justify-between items-center">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Recent Tasks</span>
          <span class="text-[10px] font-mono text-base-content/30">{{ m.recent_tasks?.length || 0 }} entries</span>
        </div>
        <div class="overflow-auto flex-1">
          <table class="table table-xs w-full" v-if="m.recent_tasks?.length">
            <thead class="sticky top-0 bg-base-200 z-10">
              <tr class="text-xs text-base-content/50">
                <th class="w-[35%]">Task</th>
                <th class="w-[15%]">Mode</th>
                <th class="w-[15%]">Model</th>
                <th class="w-[10%]">Status</th>
                <th class="w-[10%]">Tokens</th>
                <th class="w-[15%]">Time</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(t, i) in [...(m.recent_tasks || [])].reverse()" :key="i" class="hover:bg-base-300/30">
                <td class="max-w-[0] truncate font-mono text-xs" :title="t.task">{{ t.task }}</td>
                <td class="text-xs">
                  <span class="badge badge-xs" :class="{
                    'badge-warning': t.agent === 'hosted-mcp',
                    'badge-success': t.agent === 'standalone',
                    'badge-info': t.agent === 'dry-run',
                    'badge-ghost': !['hosted-mcp','standalone','dry-run'].includes(t.agent),
                  }">{{ t.agent || '—' }}</span>
                </td>
                <td class="text-xs font-mono text-base-content/70" :title="t.model">
                  {{ t.model || '—' }}
                </td>
                <td>
                  <span class="badge badge-xs"
                    :class="{
                      'badge-info': t.status === 'running',
                      'badge-success': t.status === 'completed',
                      'badge-error': t.status === 'failed',
                      'badge-warning': t.status === 'awaiting-approval',
                    }"
                  >{{ t.status }}</span>
                </td>
                <td class="text-xs font-mono">
                  <span v-if="t.tokens > 0">
                    {{ fmtTokens(t.tokens) }}
                    <span v-if="t.agent === 'hosted-mcp'" class="text-base-content/30" title="Estimated from prompt length">~</span>
                  </span>
                  <span v-else class="text-base-content/30">—</span>
                </td>
                <td class="text-xs text-base-content/60 font-mono">
                  {{ t.started_at }}<span v-if="t.finished_at && t.finished_at !== t.started_at"> → {{ t.finished_at }}</span>
                </td>
              </tr>
            </tbody>
          </table>
          <div v-else class="p-6 text-center text-base-content/40 text-sm">No tasks yet this session</div>
        </div>
      </div>

      <!-- Persisted Runs -->
      <div class="bg-base-200 rounded-lg overflow-hidden">
        <div class="px-4 py-2.5 border-b border-base-300 flex justify-between items-center">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Persisted Runs</span>
          <div class="flex items-center gap-2">
            <span class="text-[10px] font-mono text-base-content/30">{{ runs.length }} runs</span>
            <button class="btn btn-ghost btn-xs text-[11px] font-mono" @click="refreshRuns">↺ refresh</button>
          </div>
        </div>
        <div class="grid grid-cols-[320px_1fr] min-h-[16rem]">
          <div class="border-r border-base-300 overflow-auto max-h-[24rem]">
            <button
              v-for="run in runs"
              :key="run.id"
              class="w-full text-left px-4 py-3 border-b border-base-300/40 hover:bg-base-300/30"
              :class="{ 'bg-base-300/50': selectedRunId === run.id }"
              @click="selectRun(run.id)"
            >
              <div class="flex items-center justify-between gap-2">
                <span class="font-mono text-[11px] truncate">{{ run.kind }}</span>
                <span
                  class="badge badge-xs"
                  :class="{
                    'badge-info': run.status === 'running',
                    'badge-warning': run.status === 'awaiting-approval',
                    'badge-success': run.status === 'succeeded',
                    'badge-error': run.status === 'failed',
                  }"
                >{{ run.status }}</span>
              </div>
              <div class="text-xs text-base-content/70 mt-1 truncate">{{ run.task }}</div>
              <div class="text-[11px] text-base-content/40 mt-1">
                {{ run.mode }} · {{ run.provider || 'hosted-mcp' }}
              </div>
            </button>
            <div v-if="!runs.length" class="p-6 text-center text-base-content/40 text-sm">No persisted runs yet</div>
          </div>

          <div class="p-4 overflow-auto max-h-[24rem]">
            <template v-if="selectedRun?.run">
              <div class="flex flex-wrap gap-x-6 gap-y-2 text-sm">
                <div><span class="text-base-content/50">ID:</span> <span class="font-mono text-xs">{{ selectedRun.run.metadata.id }}</span></div>
                <div><span class="text-base-content/50">Kind:</span> <span class="font-mono text-xs">{{ selectedRun.run.metadata.kind }}</span></div>
                <div><span class="text-base-content/50">Status:</span> <span class="font-mono text-xs">{{ selectedRun.run.metadata.status }}</span></div>
                <div><span class="text-base-content/50">Started:</span> <span class="font-mono text-xs">{{ selectedRun.run.metadata.started_at }}</span></div>
              </div>

              <div class="mt-4">
                <div class="text-xs font-bold uppercase tracking-wider text-base-content/60 mb-2">Task</div>
                <div class="bg-base-300/40 rounded p-3 font-mono text-xs whitespace-pre-wrap">{{ selectedRun.run.metadata.task }}</div>
              </div>

              <div class="mt-4" v-if="selectedWorkflowState">
                <div class="text-xs font-bold uppercase tracking-wider text-base-content/60 mb-2">Workflow State</div>
                <div class="grid grid-cols-4 gap-3 mb-3 text-sm">
                  <div class="bg-base-300/30 rounded p-3">
                    <div class="text-[11px] text-base-content/50 uppercase">Workflow</div>
                    <div class="font-mono text-xs mt-1">{{ selectedWorkflowState.workflow }}</div>
                  </div>
                  <div class="bg-base-300/30 rounded p-3">
                    <div class="text-[11px] text-base-content/50 uppercase">Progress</div>
                    <div class="font-mono text-xs mt-1">{{ selectedWorkflowState.steps_completed }}/{{ selectedWorkflowState.steps_total }}</div>
                  </div>
                  <div class="bg-base-300/30 rounded p-3">
                    <div class="text-[11px] text-base-content/50 uppercase">Budget</div>
                    <div class="font-mono text-xs mt-1">{{ selectedWorkflowState.budget_used?.tokens || 0 }} tokens</div>
                  </div>
                  <div class="bg-base-300/30 rounded p-3">
                    <div class="text-[11px] text-base-content/50 uppercase">Artifacts</div>
                    <div class="font-mono text-xs mt-1">{{ selectedRun.run.artifacts?.join(', ') || '—' }}</div>
                  </div>
                </div>
                <div
                  v-if="selectedEvalGateDecision"
                  class="rounded p-4 mb-3 border"
                  :class="{
                    'border-success/40 bg-success/10': selectedEvalGateDecision.decision === 'pass',
                    'border-warning/40 bg-warning/10': selectedEvalGateDecision.decision === 'warn',
                    'border-error/40 bg-error/10': selectedEvalGateDecision.decision === 'block',
                  }"
                >
                  <div class="flex items-center justify-between gap-3">
                    <div>
                      <div class="text-xs font-bold uppercase tracking-wider"
                        :class="{
                          'text-success': selectedEvalGateDecision.decision === 'pass',
                          'text-warning': selectedEvalGateDecision.decision === 'warn',
                          'text-error': selectedEvalGateDecision.decision === 'block',
                        }"
                      >Eval Gate</div>
                      <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.workflow }} · {{ selectedEvalGateDecision.mode }}</div>
                    </div>
                    <span
                      class="badge badge-sm"
                      :class="{
                        'badge-success': selectedEvalGateDecision.decision === 'pass',
                        'badge-warning': selectedEvalGateDecision.decision === 'warn',
                        'badge-error': selectedEvalGateDecision.decision === 'block',
                      }"
                    >{{ selectedEvalGateDecision.decision }}</span>
                  </div>
                  <div class="grid grid-cols-3 gap-3 mt-3 text-sm">
                    <div class="bg-base-300/30 rounded p-3">
                      <div class="text-[11px] text-base-content/50 uppercase">Baseline</div>
                      <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.baseline_sample_size }}/{{ selectedEvalGateDecision.min_baseline_runs }}</div>
                    </div>
                    <div class="bg-base-300/30 rounded p-3">
                      <div class="text-[11px] text-base-content/50 uppercase">Window</div>
                      <div class="font-mono text-xs mt-1">{{ selectedEvalGateDecision.baseline_window }} runs</div>
                    </div>
                    <div class="bg-base-300/30 rounded p-3">
                      <div class="text-[11px] text-base-content/50 uppercase">Reason Codes</div>
                      <div class="font-mono text-xs mt-1">{{ fmtReasonCodes(selectedEvalGateDecision.reason_codes) }}</div>
                    </div>
                  </div>
                  <div class="font-mono text-xs whitespace-pre-wrap mt-3">{{ selectedEvalGateDecision.message }}</div>
                </div>
                <div v-if="pendingApproval" class="rounded border border-warning/40 bg-warning/10 p-4 mb-3">
                  <div class="flex items-center justify-between gap-3">
                    <div>
                      <div class="text-xs font-bold uppercase tracking-wider text-warning">Approval Pending</div>
                      <div class="font-mono text-xs mt-1">{{ pendingApproval.step_id }} · {{ pendingApproval.agent }}</div>
                    </div>
                    <span class="badge badge-sm badge-warning">awaiting approval</span>
                  </div>
                  <div class="font-mono text-xs whitespace-pre-wrap mt-3">{{ pendingApproval.content_preview }}</div>
                  <textarea
                    v-model="approvalEditContent"
                    class="textarea textarea-bordered w-full mt-3 font-mono text-xs"
                    rows="6"
                    placeholder="Edited approval content"
                  />
                  <div class="flex items-center gap-2 mt-3">
                    <button class="btn btn-xs btn-success" @click="recordApproval('approve')">Approve</button>
                    <button class="btn btn-xs btn-warning" @click="recordApproval('edit')">Approve Edit</button>
                    <button class="btn btn-xs btn-error" @click="recordApproval('deny')">Deny</button>
                    <span v-if="approvalStatus" class="text-xs text-base-content/70">{{ approvalStatus }}</span>
                  </div>
                </div>
                <div v-else-if="canResumeSelectedRun" class="rounded border border-info/40 bg-info/10 p-4 mb-3">
                  <div class="flex items-center justify-between gap-3">
                    <div>
                      <div class="text-xs font-bold uppercase tracking-wider text-info">Resume Ready</div>
                      <div v-if="m.runtime_mode === 'hosted-mcp'" class="text-xs text-base-content/70 mt-1">
                        Approval recorded. Your AI assistant will continue automatically — ask it to call <code class="font-mono bg-base-200 px-1 rounded">agent007_workflow_next</code>.
                      </div>
                      <div v-else class="text-xs text-base-content/70 mt-1">
                        Approval has been recorded for this paused workflow. Resume it to continue execution.
                      </div>
                    </div>
                    <button v-if="m.runtime_mode !== 'hosted-mcp'" class="btn btn-xs btn-info" @click="resumeSelectedRun">Resume Workflow</button>
                  </div>
                  <div v-if="resumeStatus" class="text-xs text-base-content/70 mt-3">{{ resumeStatus }}</div>
                </div>
                <div class="space-y-2">
                  <div
                    v-for="step in selectedWorkflowState.steps || []"
                    :key="step.id"
                    class="rounded border border-base-300/60 p-3"
                  >
                    <div class="flex items-center justify-between gap-2">
                      <span class="font-mono text-xs">{{ step.id }}</span>
                      <span
                        class="badge badge-xs"
                        :class="{
                          'badge-ghost': step.status === 'pending',
                          'badge-info': step.status === 'running',
                          'badge-warning': step.status === 'awaiting-approval' || step.status === 'skipped',
                          'badge-success': step.status === 'completed',
                          'badge-error': step.status === 'failed',
                        }"
                      >{{ step.status }}</span>
                    </div>
                    <div class="text-[11px] text-base-content/50 mt-1">{{ step.agent }} · attempts {{ step.attempts }}</div>
                    <div v-if="step.output_preview" class="font-mono text-xs mt-2 whitespace-pre-wrap">{{ step.output_preview }}</div>
                    <div v-if="step.error" class="text-xs text-error mt-2">{{ step.error }}</div>
                  </div>
                </div>
              </div>

              <div class="mt-4" v-if="selectedRun.run.entries?.length">
                <div class="text-xs font-bold uppercase tracking-wider text-base-content/60 mb-2">Recent Trace</div>
                <div class="space-y-2">
                  <div
                    v-for="entry in selectedRun.run.entries.slice(-10).reverse()"
                    :key="`${entry.timestamp}-${entry.kind}`"
                    class="bg-base-300/30 rounded p-3"
                  >
                    <div class="flex items-center justify-between gap-2">
                      <span class="font-mono text-xs">{{ entry.kind }}</span>
                      <span class="text-[11px] text-base-content/40">{{ entry.timestamp }}</span>
                    </div>
                    <div class="font-mono text-[11px] text-base-content/70 mt-2 whitespace-pre-wrap">{{ JSON.stringify(entry.payload, null, 2) }}</div>
                  </div>
                </div>
              </div>
            </template>
            <div v-else class="h-full flex items-center justify-center text-base-content/40 text-sm">
              Select a run to inspect its persisted state
            </div>
          </div>
        </div>
      </div>

      <!-- Event Log -->
      <div class="bg-base-200 rounded-lg flex flex-col" style="max-height: 35vh">
        <div class="px-4 py-2.5 border-b border-base-300 flex justify-between items-center">
          <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">Event Log</span>
          <span class="text-[10px] font-mono text-base-content/30">{{ events?.length || 0 }} events</span>
        </div>
        <div class="overflow-auto flex-1 font-mono text-xs">
          <div
            v-for="(evt, i) in recentEvents"
            :key="i"
            class="px-4 py-1.5 border-b border-base-300/30 flex gap-3 hover:bg-base-300/30"
          >
            <span class="text-base-content/40 shrink-0 w-20">{{ evt._ts?.slice(11, 19) || '' }}</span>
            <span
              class="badge badge-xs shrink-0"
              :class="{
                'badge-info': evt.type === 'AgentEvent',
                'badge-success': evt.type === 'LearningEvent',
                'badge-warning': evt.type === 'StatusUpdate',
              }"
            >{{ evt.type || 'event' }}</span>
            <span class="truncate text-base-content/70">{{ JSON.stringify(evt.payload || evt).slice(0, 120) }}</span>
          </div>
          <div v-if="!recentEvents.length" class="p-8 text-center text-base-content/40">
            Waiting for events...
          </div>
        </div>
      </div>
    </div>

    <!-- Task input bar -->
    <div class="px-4 py-3 border-t border-base-300 bg-base-200 flex items-center gap-3">
      <span class="text-primary font-bold font-mono text-sm shrink-0 select-none">›_</span>
      <input
        v-model="taskInput"
        class="input input-sm flex-1 font-mono text-sm bg-base-300/50 border-base-300 focus:border-primary/50 placeholder:text-base-content/25"
        placeholder="describe a task for agent007…"
        @keydown.enter="submitTask"
      />
      <button class="btn btn-sm btn-primary font-mono text-xs px-4" @click="submitTask">run</button>
      <span v-if="taskStatus" class="text-[11px] font-mono text-base-content/50 shrink-0">{{ taskStatus }}</span>
    </div>
  </div>
</template>
