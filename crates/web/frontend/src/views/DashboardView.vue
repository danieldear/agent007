<script setup>
import { ref, onMounted, onUnmounted, computed, watch, nextTick } from 'vue'
import { useApi } from '../composables/useApi.js'

const props = defineProps({ events: Array, connected: Boolean, stats: Object })
const emit  = defineEmits(['open-run'])
const { api } = useApi()

// ── State ───────────────────────────────────────────────────────────────
const runs           = ref([])
const runtimeSessions = ref(null)
const metrics        = ref(null)
const runFilter      = ref('')
const runStatusFilter = ref('')

// ── Task panel ──────────────────────────────────────────────────────────
const taskPanelOpen       = ref(false)
const taskInput           = ref('')
const chatMessages        = ref([])
const chatPending         = ref(false)
const slashCommands       = ref([])
const slashCommandsLoaded = ref(false)
const showSlashMenu       = ref(false)
const slashFilter         = ref('')
const slashMenuIndex      = ref(0)
const slashMenuRef        = ref(null)
const taskInputRef        = ref(null)

// ── Timers ──────────────────────────────────────────────────────────────
let refreshTimer = null

onMounted(async () => {
  await refresh()
  refreshTimer = setInterval(refresh, 5000)
})

onUnmounted(() => { if (refreshTimer) clearInterval(refreshTimer) })

watch(() => props.stats, v => { if (v) metrics.value = v })
watch(() => props.events?.length, refresh)

async function refresh() {
  try {
    const [runList, sessions, statsData] = await Promise.allSettled([
      api.listRuns(),
      api.getRuntimeSessions(12),
      api.getStats(),
    ])
    if (runList.status === 'fulfilled')   runs.value = runList.value || []
    if (sessions.status === 'fulfilled')  runtimeSessions.value = sessions.value
    if (statsData.status === 'fulfilled' && statsData.value) metrics.value = statsData.value
  } catch {}
}

// ── Computed ────────────────────────────────────────────────────────────
const m = computed(() => metrics.value || props.stats || {
  running_tasks: 0, awaiting_approvals: 0, completed_tasks: 0, failed_tasks: 0,
})

const activeSessions = computed(() => {
  const s = runtimeSessions.value?.sessions || []
  return s.filter(r => ['running', 'ready', 'blocked', 'attention'].includes(r.lifecycle))
})

const approvalSessions = computed(() =>
  (runtimeSessions.value?.sessions || []).filter(s => s.lifecycle === 'awaiting-approval')
)

const filteredRuns = computed(() => {
  let list = [...runs.value]
  if (runStatusFilter.value) list = list.filter(r => r.status === runStatusFilter.value)
  if (runFilter.value) {
    const q = runFilter.value.toLowerCase()
    list = list.filter(r => r.task?.toLowerCase().includes(q))
  }
  return list
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

// ── Slash commands ──────────────────────────────────────────────────────
async function loadSlashCommands() {
  if (slashCommandsLoaded.value) return
  slashCommandsLoaded.value = true
  try {
    const [skills, workflows] = await Promise.all([api.listSkills(), api.listWorkflows()])
    const cmds = []
    for (const s of skills || []) {
      if (s.trigger) cmds.push({ type: 'skill', trigger: s.trigger, name: s.name || s.trigger, description: s.description || '' })
    }
    for (const w of workflows || []) {
      cmds.push({ type: 'workflow', trigger: `/workflow:${w.name}`, name: w.name, description: 'Run workflow' })
    }
    slashCommands.value = cmds.sort((a, b) => a.trigger.localeCompare(b.trigger))
  } catch { slashCommandsLoaded.value = false }
}

watch(slashMenuIndex, idx => {
  nextTick(() => {
    slashMenuRef.value?.querySelectorAll('button[data-slash-item]')?.[idx]?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  })
})

function onChatInput(e) {
  const ta = e.target
  ta.style.height = 'auto'
  ta.style.height = Math.min(ta.scrollHeight, 144) + 'px'
  const match = taskInput.value.match(/^\/(\S*)$/)
  if (match !== null) {
    slashFilter.value = match[1]
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

async function submitTask() {
  const input = taskInput.value.trim()
  if (!input || chatPending.value) return
  chatMessages.value.push({ role: 'user', content: input })
  chatMessages.value.push({ role: 'assistant', content: '', status: 'running', sessionId: null })
  const replyIdx = chatMessages.value.length - 1
  taskInput.value = ''
  if (taskInputRef.value) { taskInputRef.value.style.height = 'auto' }
  chatPending.value = true
  try {
    const response = await api.runTask(input)
    const sessionId = response?.session || null
    chatMessages.value.splice(replyIdx, 1, {
      role: 'assistant', content: response?.output || '(task submitted)', status: 'completed', sessionId,
    })
    await refresh()
    if (sessionId) {
      taskPanelOpen.value = false
      emit('open-run', sessionId)
    }
  } catch (e) {
    chatMessages.value.splice(replyIdx, 1, { role: 'assistant', content: e.message, status: 'error', sessionId: null })
  } finally {
    chatPending.value = false
  }
}

// ── Helpers ─────────────────────────────────────────────────────────────
function statusDot(status) {
  return {
    running: 'bg-info animate-pulse',
    completed: 'bg-success',
    failed: 'bg-error',
    'awaiting-approval': 'bg-warning animate-pulse',
    blocked: 'bg-warning',
  }[status] || 'bg-base-content/20'
}

function statusLabel(status) {
  return {
    running: 'text-info',
    completed: 'text-success',
    failed: 'text-error',
    'awaiting-approval': 'text-warning',
    blocked: 'text-warning',
  }[status] || 'text-base-content/30'
}

function fmtAge(seconds) {
  if (!seconds) return '—'
  const s = Number(seconds)
  if (s < 60)    return `${s}s ago`
  if (s < 3600)  return `${Math.floor(s / 60)}m ago`
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`
  return `${Math.floor(s / 86400)}d ago`
}
</script>

<template>
  <div class="flex flex-col h-full">

    <!-- Header -->
    <div class="px-5 py-3 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <div class="flex items-center gap-3">
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Tasks</span>
        <span v-if="(m.running_tasks || 0) > 0" class="flex items-center gap-1.5 text-[11px] font-mono text-info">
          <span class="w-1.5 h-1.5 rounded-full bg-info animate-pulse"></span>
          {{ m.running_tasks }} running
        </span>
        <span v-if="(m.awaiting_approvals || 0) > 0" class="flex items-center gap-1.5 text-[11px] font-mono text-warning">
          <span class="w-1.5 h-1.5 rounded-full bg-warning"></span>
          {{ m.awaiting_approvals }} need approval
        </span>
        <span
          v-if="!m.running_tasks && !m.awaiting_approvals"
          class="text-[11px] font-mono text-base-content/25"
        >idle</span>
      </div>
      <button class="btn btn-sm btn-primary font-mono text-xs gap-1.5" @click="taskPanelOpen = true">
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/>
        </svg>
        New Task
      </button>
    </div>

    <!-- Body -->
    <div class="flex-1 overflow-auto p-4 space-y-5">

      <!-- Approval alerts -->
      <div v-if="approvalSessions.length" class="space-y-2">
        <div
          v-for="s in approvalSessions"
          :key="s.id"
          class="flex items-center gap-3 px-4 py-3 bg-warning/8 border border-warning/30 rounded-lg cursor-pointer hover:bg-warning/12 transition-colors"
          @click="emit('open-run', s.id)"
        >
          <span class="w-2 h-2 rounded-full bg-warning shrink-0 animate-pulse"></span>
          <div class="flex-1 min-w-0">
            <div class="text-[11px] font-mono font-semibold text-warning">Approval required</div>
            <div class="text-[11px] font-mono text-base-content/50 truncate mt-0.5">{{ s.task }}</div>
          </div>
          <span class="text-[10px] font-mono text-warning/60 shrink-0">Review →</span>
        </div>
      </div>

      <!-- Active runs -->
      <div v-if="activeSessions.length">
        <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest mb-2">Active</div>
        <div class="space-y-1.5">
          <div
            v-for="s in activeSessions"
            :key="s.id"
            class="flex items-center gap-3 px-4 py-3 bg-base-200 border border-base-300 rounded-lg cursor-pointer hover:border-primary/40 hover:bg-primary/4 transition-colors group"
            @click="emit('open-run', s.id)"
          >
            <span :class="['w-2 h-2 rounded-full shrink-0', statusDot(s.status)]"></span>
            <div class="flex-1 min-w-0">
              <div class="text-[12px] font-mono text-base-content/80 truncate">{{ s.task }}</div>
              <div class="flex items-center gap-2 mt-0.5">
                <span class="text-[10px] font-mono text-base-content/30">{{ s.kind }}</span>
                <template v-if="s.workflow">
                  <span class="text-base-content/15">·</span>
                  <span class="text-[10px] font-mono text-info/50">
                    {{ s.workflow.completed_steps }}/{{ s.workflow.total_steps }} steps
                  </span>
                  <span v-if="s.workflow.running_steps?.length" class="text-[10px] font-mono text-info/40">
                    · {{ s.workflow.running_steps[0] }}<span v-if="s.workflow.running_steps.length > 1">…</span>
                  </span>
                </template>
              </div>
            </div>
            <div class="flex items-center gap-3 shrink-0">
              <span class="text-[10px] font-mono text-base-content/25">{{ fmtAge(s.age_seconds) }}</span>
              <span class="text-[10px] font-mono text-primary/40 opacity-0 group-hover:opacity-100 transition-opacity">View →</span>
            </div>
          </div>
        </div>
      </div>

      <!-- History -->
      <div>
        <!-- Filter bar -->
        <div class="flex items-center gap-2 mb-3">
          <div class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest shrink-0">History</div>
          <div class="flex gap-0.5 bg-base-300/50 rounded p-0.5">
            <button
              v-for="[val, lbl] in [['','all'],['running','running'],['completed','done'],['failed','failed'],['awaiting-approval','approval']]"
              :key="val"
              class="btn btn-xs font-mono rounded px-2.5 text-[10px]"
              :class="runStatusFilter === val ? 'btn-primary shadow-sm' : 'btn-ghost text-base-content/40'"
              @click="runStatusFilter = val"
            >{{ lbl }}</button>
          </div>
          <input
            v-model="runFilter"
            class="flex-1 max-w-52 bg-base-300/50 border border-base-300/80 rounded text-[11px] font-mono px-2.5 py-1 focus:outline-none focus:border-primary/40 placeholder-base-content/20"
            placeholder="filter…"
          />
          <span class="text-[10px] font-mono text-base-content/20 ml-auto shrink-0">{{ filteredRuns.length }}</span>
        </div>

        <!-- Empty -->
        <div v-if="!filteredRuns.length" class="text-center py-16">
          <div class="text-4xl mb-3 text-base-content/8">▶</div>
          <div class="text-sm font-mono text-base-content/30">No runs yet</div>
          <div class="text-[11px] font-mono text-base-content/20 mt-1">
            Click <span class="text-primary">+ New Task</span> to get started
          </div>
        </div>

        <!-- Run rows -->
        <div v-else class="space-y-0.5">
          <div
            v-for="run in filteredRuns"
            :key="run.id"
            class="flex items-center gap-3 px-4 py-2.5 rounded-lg cursor-pointer hover:bg-base-200 transition-colors group border border-transparent hover:border-base-300/60"
            @click="emit('open-run', run.id)"
          >
            <span :class="['w-1.5 h-1.5 rounded-full shrink-0', statusDot(run.status)]"></span>
            <div class="flex-1 min-w-0">
              <div class="text-[12px] font-mono text-base-content/70 truncate">{{ run.task }}</div>
            </div>
            <div class="flex items-center gap-3 shrink-0">
              <span class="text-[10px] font-mono text-base-content/25 hidden sm:block">{{ run.kind }}</span>
              <span class="text-[10px] font-mono text-base-content/25">{{ fmtAge(run.age_seconds) }}</span>
              <span :class="['text-[10px] font-mono', statusLabel(run.status)]">{{ run.status }}</span>
              <span class="text-[10px] font-mono text-primary/30 opacity-0 group-hover:opacity-100 transition-opacity">→</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- ── Task slide-over ────────────────────────────────────────────── -->
    <Teleport to="body">
      <div v-if="taskPanelOpen" class="fixed inset-0 z-50 flex">
        <!-- Backdrop -->
        <div class="absolute inset-0 bg-base-300/60 backdrop-blur-sm" @click="taskPanelOpen = false" />

        <!-- Panel -->
        <div class="relative ml-auto w-full max-w-lg h-full bg-base-100 border-l border-base-300 flex flex-col shadow-2xl">

          <!-- Chat header -->
          <div class="flex items-center justify-between px-5 py-3.5 bg-base-200 border-b border-base-300 shrink-0">
            <div>
              <div class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">New Task</div>
              <div class="text-[10px] font-mono text-base-content/30 mt-0.5">Type a task or <span class="text-primary">/skill</span> to run</div>
            </div>
            <button class="btn btn-ghost btn-xs font-mono text-base-content/40 px-1" @click="taskPanelOpen = false">✕</button>
          </div>

          <!-- Messages -->
          <div class="flex-1 overflow-auto p-4 space-y-3">
            <!-- Empty state -->
            <div v-if="!chatMessages.length" class="pt-8 space-y-2">
              <div class="text-[10px] font-mono text-base-content/25 uppercase tracking-widest px-1 mb-3">Suggested</div>
              <button
                v-for="prompt in ['Run all tests and fix failures', 'Review the last PR for security issues', 'Refactor the auth module', 'Write a changelog for recent commits']"
                :key="prompt"
                class="w-full text-left px-3 py-2.5 rounded border border-base-300/60 bg-base-200/60 hover:border-primary/30 hover:bg-primary/5 transition-colors"
                @click="taskInput = prompt"
              >
                <span class="text-[12px] font-mono text-base-content/60">{{ prompt }}</span>
              </button>
            </div>

            <!-- Conversation -->
            <template v-else>
              <div
                v-for="(msg, i) in chatMessages"
                :key="i"
                class="flex"
                :class="msg.role === 'user' ? 'justify-end' : 'justify-start'"
              >
                <!-- User bubble -->
                <div
                  v-if="msg.role === 'user'"
                  class="max-w-[85%] px-4 py-2.5 rounded-2xl rounded-tr-sm bg-primary/15 border border-primary/25"
                >
                  <p class="text-[12px] font-mono text-base-content/80 whitespace-pre-wrap">{{ msg.content }}</p>
                </div>

                <!-- Assistant bubble -->
                <div v-else class="max-w-[85%] flex gap-2.5">
                  <span class="text-sm mt-0.5 text-base-content/20 shrink-0">▶</span>
                  <div>
                    <div
                      v-if="msg.status === 'running'"
                      class="flex items-center gap-2 text-[11px] font-mono text-base-content/40"
                    >
                      <span class="w-1.5 h-1.5 rounded-full bg-info animate-pulse"></span>
                      Running…
                    </div>
                    <p
                      v-else
                      class="text-[12px] font-mono whitespace-pre-wrap"
                      :class="msg.status === 'error' ? 'text-error/70' : 'text-base-content/70'"
                    >{{ msg.content }}</p>
                    <button
                      v-if="msg.sessionId"
                      class="mt-1.5 text-[10px] font-mono text-primary/60 hover:text-primary transition-colors"
                      @click="taskPanelOpen = false; emit('open-run', msg.sessionId)"
                    >View run →</button>
                  </div>
                </div>
              </div>
            </template>
          </div>

          <!-- Input area -->
          <div class="px-4 pb-4 pt-2 border-t border-base-300 shrink-0">
            <div class="relative">
              <!-- Slash command menu -->
              <div
                v-if="showSlashMenu && filteredSlashCommands.length"
                ref="slashMenuRef"
                class="absolute bottom-full mb-1 left-0 right-0 bg-base-100 border border-base-300 rounded-lg shadow-xl max-h-56 overflow-y-auto z-10"
              >
                <button
                  v-for="(cmd, i) in filteredSlashCommands"
                  :key="cmd.trigger"
                  data-slash-item
                  class="w-full text-left px-3 py-2 flex items-start gap-2.5 hover:bg-base-200 transition-colors"
                  :class="i === slashMenuIndex ? 'bg-primary/8' : ''"
                  @click="selectSlashCommand(cmd)"
                >
                  <span class="text-[10px] font-mono text-primary/60 mt-0.5 shrink-0">{{ cmd.type === 'skill' ? '⚡' : '⬡' }}</span>
                  <div class="min-w-0">
                    <div class="text-[11px] font-mono text-base-content/80">{{ cmd.trigger }}</div>
                    <div class="text-[10px] font-mono text-base-content/35 truncate">{{ cmd.description }}</div>
                  </div>
                </button>
              </div>

              <div class="flex items-end gap-2 bg-base-200 border border-base-300 rounded-xl px-3 py-2 focus-within:border-primary/40 transition-colors">
                <textarea
                  ref="taskInputRef"
                  v-model="taskInput"
                  rows="1"
                  class="flex-1 bg-transparent text-[13px] font-mono text-base-content/80 resize-none focus:outline-none placeholder-base-content/25 leading-relaxed"
                  placeholder="Describe a task or /skill…"
                  :disabled="chatPending"
                  @input="onChatInput"
                  @keydown="onChatKeydown"
                />
                <button
                  class="btn btn-sm btn-primary font-mono text-xs px-3 shrink-0 self-end"
                  :disabled="!taskInput.trim() || chatPending"
                  :class="chatPending ? 'loading' : ''"
                  @click="submitTask"
                >{{ chatPending ? '' : '▶' }}</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Teleport>

  </div>
</template>
