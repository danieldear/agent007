<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useApi } from '../composables/useApi.js'
import { marked } from 'marked'
import DOMPurify from 'dompurify'

marked.setOptions({ gfm: true, breaks: true })

const props = defineProps({ runId: String, events: Array })
const emit  = defineEmits(['go-back'])
const { api } = useApi()

// ── State ────────────────────────────────────────────────────────────────
const run             = ref(null)
const loading         = ref(true)
const activeTab       = ref('output')  // output | steps | artifacts | messages
const runtimeMessages = ref([])
const msgText         = ref('')
const msgKind         = ref('note')
const msgBusy         = ref(false)
const msgStatus       = ref('')
const approvalStatus  = ref('')
const approvalEdit    = ref('')
const resumeStatus    = ref('')
const artifactPath    = ref('')
const artifactPreview = ref(null)
const artifactStatus  = ref('')

let pollTimer = null

onMounted(async () => {
  await loadRun()
  await loadMessages()
  pollTimer = setInterval(async () => {
    if (['running', 'awaiting-approval'].includes(runMeta.value?.status)) {
      await loadRun()
    }
  }, 2000)
})

onUnmounted(() => { if (pollTimer) clearInterval(pollTimer) })

watch(() => props.events?.length, loadRun)

// Reset all per-run state when navigating to a different run while already on this view
watch(() => props.runId, async (newId, oldId) => {
  if (!newId || newId === oldId) return
  run.value = null
  loading.value = true
  runtimeMessages.value = []
  approvalStatus.value = ''
  approvalEdit.value = ''
  resumeStatus.value = ''
  artifactPath.value = ''
  artifactPreview.value = null
  artifactStatus.value = ''
  activeTab.value = 'output'
  await loadRun()
  await loadMessages()
})

async function loadRun() {
  try {
    run.value = await api.getRunDetail(props.runId)
    loading.value = false
    if (!approvalEdit.value && pendingApproval.value?.content) {
      approvalEdit.value = pendingApproval.value.content
    }
  } catch { loading.value = false }
}

async function loadMessages() {
  try {
    const r = await api.getRuntimeMessages(props.runId)
    runtimeMessages.value = Array.isArray(r?.messages) ? r.messages : []
  } catch {}
}

async function sendMessage() {
  if (!msgText.value.trim() || msgBusy.value) return
  msgBusy.value = true
  msgStatus.value = ''
  try {
    await api.postRuntimeMessage(props.runId, {
      from: 'operator', to: null, kind: msgKind.value, body: msgText.value.trim(),
    })
    msgText.value = ''
    await loadMessages()
    await loadRun()
  } catch (e) { msgStatus.value = e.message }
  finally { msgBusy.value = false }
}

async function recordApproval(decision) {
  if (!pendingApproval.value || !isDashboardOwned.value) return
  approvalStatus.value = 'Saving…'
  try {
    await api.approveRunStep(props.runId, {
      step: pendingApproval.value.step_id,
      decision,
      content: decision === 'edit' ? approvalEdit.value : undefined,
    })
    approvalStatus.value = `✓ ${decision} recorded`
    await loadRun()
  } catch (e) { approvalStatus.value = `Error: ${e.message}` }
}

async function resumeRun() {
  resumeStatus.value = 'Resuming…'
  try {
    await api.resumeRun(props.runId)
    resumeStatus.value = 'Resumed'
    await loadRun()
  } catch (e) { resumeStatus.value = `Error: ${e.message}` }
}

async function previewArtifact(path) {
  artifactPath.value = path
  artifactStatus.value = 'Loading…'
  artifactPreview.value = null
  try {
    artifactPreview.value = await api.previewRunArtifact(props.runId, path)
    artifactStatus.value = ''
  } catch (e) { artifactStatus.value = e.message }
}

// ── Computed ─────────────────────────────────────────────────────────────
const runMeta          = computed(() => run.value?.run?.metadata || null)
const outputText       = computed(() => run.value?.output_text || '')
const artifacts        = computed(() => run.value?.run?.artifacts || [])
const workflowState    = computed(() => run.value?.workflow_state || null)
const workflowSteps    = computed(() => workflowState.value?.steps || [])
const pendingApproval  = computed(() => workflowState.value?.pending_approval || null)
const tokenSummary     = computed(() => run.value?.token_summary || null)
const isDashboardOwned = computed(() => (runMeta.value?.kind || '').startsWith('workflow-web-'))

const canResume = computed(() => {
  if (!workflowState.value || !isDashboardOwned.value) return false
  if (pendingApproval.value) return false
  const decisions = Object.keys(workflowState.value?.approval_decisions || {})
  return decisions.length > 0 && ['awaiting-approval'].includes(runMeta.value?.status)
})

const renderedOutput = computed(() => {
  if (!outputText.value) return ''
  return DOMPurify.sanitize(marked.parse(outputText.value), { ADD_TAGS: ['pre','code'], ADD_ATTR: ['class'] })
})

// ── Helpers ──────────────────────────────────────────────────────────────
function statusDot(status) {
  return {
    running: 'bg-info animate-pulse',
    completed: 'bg-success',
    succeeded: 'bg-success',
    failed: 'bg-error',
    'awaiting-approval': 'bg-warning animate-pulse',
    blocked: 'bg-warning',
  }[status] || 'bg-base-content/20'
}

function statusLabel(status) {
  return {
    running: 'text-info',
    completed: 'text-success',
    succeeded: 'text-success',
    failed: 'text-error',
    'awaiting-approval': 'text-warning',
    blocked: 'text-warning',
  }[status] || 'text-base-content/40'
}

function stepStatusDot(status) {
  return {
    running: 'bg-info animate-pulse',
    completed: 'bg-success',
    failed: 'bg-error',
    pending: 'bg-base-content/20',
    skipped: 'bg-base-content/15',
  }[status] || 'bg-base-content/15'
}

function fmtTokens(n) {
  if (n == null) return '—'
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

function fmtDuration(seconds) {
  if (!seconds) return '—'
  const s = Number(seconds)
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`
}

function fmtDate(iso) {
  if (!iso) return '—'
  return new Date(iso).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}
</script>

<template>
  <div class="flex flex-col h-full">

    <!-- Header -->
    <div class="px-5 py-3 border-b border-base-300 bg-base-200 flex items-center gap-3 shrink-0">
      <button
        class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-2 gap-1"
        @click="emit('go-back')"
      >
        ← Tasks
      </button>
      <div class="w-px h-4 bg-base-300"></div>
      <template v-if="runMeta">
        <span :class="['w-2 h-2 rounded-full shrink-0', statusDot(runMeta.status)]"></span>
        <span class="text-[12px] font-mono text-base-content/70 truncate flex-1 min-w-0">{{ runMeta.task || runId }}</span>
        <span :class="['text-[11px] font-mono shrink-0', statusLabel(runMeta.status)]">{{ runMeta.status }}</span>
      </template>
      <span v-else-if="loading" class="text-[11px] font-mono text-base-content/30">Loading…</span>
      <span v-else class="text-[11px] font-mono text-base-content/30 font-mono truncate">{{ runId }}</span>
    </div>

    <!-- Body: metadata sidebar + content -->
    <div class="flex-1 overflow-hidden flex">

      <!-- Left: metadata -->
      <aside class="w-52 shrink-0 border-r border-base-300/60 bg-base-200/50 overflow-y-auto p-4 space-y-4">

        <!-- Status + timing -->
        <div class="space-y-2">
          <div v-if="runMeta?.started_at">
            <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest mb-0.5">Started</div>
            <div class="text-[11px] font-mono text-base-content/60">{{ fmtDate(runMeta.started_at) }}</div>
          </div>
          <div v-if="runMeta?.finished_at">
            <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest mb-0.5">Finished</div>
            <div class="text-[11px] font-mono text-base-content/60">{{ fmtDate(runMeta.finished_at) }}</div>
          </div>
          <div v-if="runMeta?.duration_seconds">
            <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest mb-0.5">Duration</div>
            <div class="text-[11px] font-mono text-base-content/60">{{ fmtDuration(runMeta.duration_seconds) }}</div>
          </div>
        </div>

        <!-- Agent / model -->
        <div class="space-y-2" v-if="runMeta">
          <div v-if="runMeta.kind">
            <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest mb-0.5">Kind</div>
            <div class="text-[11px] font-mono text-base-content/60">{{ runMeta.kind }}</div>
          </div>
          <div v-if="runMeta.provider">
            <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest mb-0.5">Provider</div>
            <div class="text-[11px] font-mono text-base-content/60">{{ runMeta.provider }}</div>
          </div>
        </div>

        <!-- Token summary -->
        <div v-if="tokenSummary" class="space-y-2">
          <div class="text-[9px] font-mono text-base-content/25 uppercase tracking-widest">Tokens</div>
          <div class="grid grid-cols-2 gap-1.5">
            <div class="bg-base-300/40 rounded px-2 py-1.5">
              <div class="text-[9px] font-mono text-base-content/25">in</div>
              <div class="text-[11px] font-mono text-base-content/60">{{ fmtTokens(tokenSummary.input_tokens) }}</div>
            </div>
            <div class="bg-base-300/40 rounded px-2 py-1.5">
              <div class="text-[9px] font-mono text-base-content/25">out</div>
              <div class="text-[11px] font-mono text-base-content/60">{{ fmtTokens(tokenSummary.output_tokens) }}</div>
            </div>
          </div>
          <div v-if="tokenSummary.estimated_usd" class="text-[11px] font-mono text-base-content/40">
            ~${{ tokenSummary.estimated_usd.toFixed(4) }}
          </div>
        </div>

        <!-- Run ID -->
        <div>
          <div class="text-[9px] font-mono text-base-content/20 uppercase tracking-widest mb-0.5">Run ID</div>
          <div class="text-[10px] font-mono text-base-content/30 break-all">{{ runId }}</div>
        </div>

      </aside>

      <!-- Right: content tabs -->
      <div class="flex-1 overflow-hidden flex flex-col">

        <!-- Approval alert (always visible if pending) -->
        <div v-if="pendingApproval" class="px-5 py-3 bg-warning/8 border-b border-warning/25 shrink-0">
          <div class="flex items-center gap-2 mb-2">
            <span class="w-2 h-2 rounded-full bg-warning animate-pulse"></span>
            <span class="text-[11px] font-mono font-semibold text-warning">Approval required — step: {{ pendingApproval.step_id }}</span>
          </div>
          <div v-if="pendingApproval.content" class="mb-3">
            <textarea
              v-model="approvalEdit"
              class="w-full bg-base-300/50 border border-base-300 rounded text-[11px] font-mono p-2.5 h-24 resize-y focus:outline-none focus:border-warning/40"
            />
          </div>
          <div class="flex items-center gap-2">
            <button
              v-if="isDashboardOwned"
              class="btn btn-sm btn-success font-mono text-xs"
              @click="recordApproval('approve')"
            >✓ Approve</button>
            <button
              v-if="isDashboardOwned && pendingApproval.content"
              class="btn btn-sm btn-warning font-mono text-xs"
              @click="recordApproval('edit')"
            >✎ Approve with edit</button>
            <button
              v-if="isDashboardOwned"
              class="btn btn-sm btn-error btn-outline font-mono text-xs"
              @click="recordApproval('deny')"
            >✕ Deny</button>
            <span v-if="!isDashboardOwned" class="text-[10px] font-mono text-base-content/35">
              Approval belongs to the client that started this workflow.
            </span>
            <span v-if="approvalStatus" class="text-[10px] font-mono text-base-content/50 ml-auto">{{ approvalStatus }}</span>
          </div>
        </div>

        <!-- Resume notice -->
        <div v-if="canResume" class="px-5 py-2.5 bg-info/8 border-b border-info/20 shrink-0 flex items-center gap-3">
          <span class="text-[11px] font-mono text-info/70">Approval recorded — ready to resume</span>
          <button class="btn btn-xs btn-info font-mono ml-auto" @click="resumeRun">Resume workflow →</button>
          <span v-if="resumeStatus" class="text-[10px] font-mono text-base-content/40">{{ resumeStatus }}</span>
        </div>

        <!-- Tab bar -->
        <div class="flex gap-0.5 px-4 pt-3 pb-0 bg-base-200/30 border-b border-base-300/40 shrink-0">
          <button
            v-for="[id, lbl, count] in [
              ['output',    'Output',    null],
              ['steps',     'Steps',     workflowSteps.length || null],
              ['artifacts', 'Artifacts', artifacts.length || null],
              ['messages',  'Messages',  runtimeMessages.length || null],
            ]"
            :key="id"
            class="px-3 py-2 text-[11px] font-mono rounded-t border-b-2 transition-colors"
            :class="activeTab === id
              ? 'border-primary text-primary bg-primary/5'
              : 'border-transparent text-base-content/40 hover:text-base-content/70 hover:bg-base-300/30'"
            @click="activeTab = id"
          >
            {{ lbl }}
            <span v-if="count" class="ml-1 text-[9px] opacity-60">({{ count }})</span>
          </button>
        </div>

        <!-- Tab content -->
        <div class="flex-1 overflow-auto p-5">

          <!-- Output tab -->
          <div v-if="activeTab === 'output'">
            <div v-if="!outputText" class="text-[11px] font-mono text-base-content/30 italic">No output yet.</div>
            <div
              v-else
              class="md-preview prose prose-sm max-w-none font-mono text-[12px] text-base-content/80 leading-relaxed"
              v-html="renderedOutput"
            />
          </div>

          <!-- Steps tab -->
          <div v-else-if="activeTab === 'steps'">
            <div v-if="!workflowSteps.length" class="text-[11px] font-mono text-base-content/30 italic">No workflow steps.</div>
            <div v-else class="space-y-1.5">
              <div
                v-for="step in workflowSteps"
                :key="step.id"
                class="px-4 py-3 bg-base-200 border border-base-300 rounded-lg"
              >
                <div class="flex items-center gap-2.5">
                  <span :class="['w-1.5 h-1.5 rounded-full shrink-0', stepStatusDot(step.status)]"></span>
                  <span class="text-[12px] font-mono text-base-content/70 flex-1">{{ step.id }}</span>
                  <span class="text-[10px] font-mono text-base-content/30">{{ step.status }}</span>
                  <span v-if="step.duration_ms" class="text-[10px] font-mono text-base-content/25">
                    {{ (step.duration_ms / 1000).toFixed(1) }}s
                  </span>
                </div>
                <div v-if="step.output_preview" class="mt-2 text-[10px] font-mono text-base-content/40 line-clamp-2 pl-4">
                  {{ step.output_preview }}
                </div>
              </div>
            </div>
          </div>

          <!-- Artifacts tab -->
          <div v-else-if="activeTab === 'artifacts'">
            <div v-if="!artifacts.length" class="text-[11px] font-mono text-base-content/30 italic">No artifacts.</div>
            <div v-else class="space-y-1">
              <!-- File list -->
              <div
                v-for="path in artifacts"
                :key="path"
                class="flex items-center gap-2 px-3 py-2 rounded border border-transparent hover:bg-base-200 hover:border-base-300 cursor-pointer transition-colors"
                :class="artifactPath === path ? 'bg-primary/5 border-primary/20' : ''"
                @click="previewArtifact(path)"
              >
                <span class="text-base-content/30 text-[11px] shrink-0">📄</span>
                <span class="text-[11px] font-mono text-base-content/70 flex-1 truncate">{{ path }}</span>
                <span class="text-[10px] font-mono text-primary/40">preview</span>
              </div>

              <!-- Preview panel -->
              <div v-if="artifactPath" class="mt-4 border border-base-300 rounded-lg overflow-hidden">
                <div class="px-3 py-2 bg-base-200 border-b border-base-300 flex items-center justify-between">
                  <span class="text-[10px] font-mono text-base-content/50">{{ artifactPath }}</span>
                  <a
                    v-if="artifactPreview?.raw_url"
                    :href="artifactPreview.raw_url"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="text-[10px] font-mono text-primary/60 hover:text-primary"
                  >raw ↗</a>
                </div>
                <div v-if="artifactStatus" class="px-3 py-2 text-[11px] font-mono text-base-content/40">{{ artifactStatus }}</div>
                <div v-else-if="artifactPreview">
                  <img
                    v-if="artifactPreview.kind === 'image'"
                    :src="artifactPreview.raw_url"
                    class="max-w-full"
                    alt="artifact preview"
                  />
                  <pre
                    v-else
                    class="text-[11px] font-mono text-base-content/70 p-4 overflow-auto max-h-96 whitespace-pre-wrap"
                  >{{ artifactPreview.content || '(binary / no preview)' }}</pre>
                </div>
              </div>
            </div>
          </div>

          <!-- Messages tab -->
          <div v-else-if="activeTab === 'messages'">
            <div class="space-y-2 mb-4">
              <div v-if="!runtimeMessages.length" class="text-[11px] font-mono text-base-content/30 italic mb-3">
                No messages yet. Add notes or directives below.
              </div>
              <div
                v-for="msg in runtimeMessages"
                :key="msg.id"
                class="px-3 py-2.5 bg-base-200 border border-base-300 rounded-lg"
              >
                <div class="flex items-center gap-2 mb-1">
                  <span class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">{{ msg.kind }}</span>
                  <span class="text-base-content/20 text-[9px]">·</span>
                  <span class="text-[9px] font-mono text-base-content/25">{{ msg.from }}</span>
                  <span v-if="msg.to" class="text-[9px] font-mono text-base-content/25">→ {{ msg.to }}</span>
                </div>
                <p class="text-[11px] font-mono text-base-content/65 leading-relaxed">{{ msg.body }}</p>
              </div>
            </div>

            <!-- Add message -->
            <div class="border border-base-300 rounded-lg p-3 space-y-2">
              <div class="flex gap-2">
                <select
                  v-model="msgKind"
                  class="bg-base-300/60 border border-base-300 rounded text-[11px] font-mono px-2 py-1 focus:outline-none"
                >
                  <option value="note">note</option>
                  <option value="directive">directive</option>
                  <option value="feedback">feedback</option>
                </select>
              </div>
              <textarea
                v-model="msgText"
                rows="2"
                class="w-full bg-base-300/40 border border-base-300 rounded text-[11px] font-mono p-2 resize-y focus:outline-none focus:border-primary/40 placeholder-base-content/25"
                placeholder="Add a note or directive…"
              />
              <div class="flex items-center gap-2">
                <button
                  class="btn btn-xs btn-primary font-mono"
                  :disabled="!msgText.trim() || msgBusy"
                  @click="sendMessage"
                >Send</button>
                <span v-if="msgStatus" class="text-[10px] font-mono text-error/60">{{ msgStatus }}</span>
              </div>
            </div>
          </div>

        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.md-preview :deep(h1),
.md-preview :deep(h2),
.md-preview :deep(h3) {
  font-weight: 700;
  margin-top: 1rem;
  margin-bottom: 0.25rem;
  font-family: ui-monospace, monospace;
}
.md-preview :deep(code) {
  background: oklch(var(--b3) / 0.6);
  padding: 0.1em 0.3em;
  border-radius: 0.25rem;
  font-size: 0.9em;
}
.md-preview :deep(pre) {
  background: oklch(var(--b3) / 0.8);
  padding: 0.75rem 1rem;
  border-radius: 0.5rem;
  overflow-x: auto;
  margin: 0.5rem 0;
}
.md-preview :deep(pre code) {
  background: none;
  padding: 0;
}
.md-preview :deep(ul),
.md-preview :deep(ol) {
  padding-left: 1.25rem;
  margin: 0.25rem 0;
}
.md-preview :deep(p) { margin: 0.25rem 0; }
.md-preview :deep(blockquote) {
  border-left: 2px solid oklch(var(--p) / 0.4);
  padding-left: 0.75rem;
  color: oklch(var(--bc) / 0.5);
}
</style>
