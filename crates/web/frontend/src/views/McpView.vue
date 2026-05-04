<script setup>
import { ref, computed, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()

// ── state ──────────────────────────────────────────────────────────────
const servers   = ref([])
const activeTab = ref('servers')
const selected  = ref(null)
const loading   = ref(false)

// toast
const toast = ref(null)
let toastTimer = null
function showToast(msg, type = 'success') {
  clearTimeout(toastTimer)
  toast.value = { msg, type }
  toastTimer = setTimeout(() => { toast.value = null }, 3500)
}

// add-server form
const addForm = ref({
  source_kind: 'npm',
  source_ref:  '',
  name:        '',
  command:     '',
  args_raw:    '',
  scope:       'project',
})
const addBusy = ref(false)

// connect/approve busy state per server name
const busyMap = ref({})

// ── computed ────────────────────────────────────────────────────────────
function statusDot(status) {
  return {
    connected:    'bg-success shadow-[0_0_4px_theme(colors.success)]',
    connecting:   'bg-warning animate-pulse',
    disconnected: 'bg-base-content/30',
    error:        'bg-error shadow-[0_0_4px_theme(colors.error)]',
  }[status] || 'bg-base-content/20'
}

function kindBadge(kind) {
  return {
    npm:    'badge-accent',
    local:  'badge-neutral',
    github: 'badge-info',
    http:   'badge-secondary',
    manual: 'badge-ghost',
  }[kind] || 'badge-ghost'
}

// ── lifecycle ───────────────────────────────────────────────────────────
onMounted(refresh)

async function refresh() {
  loading.value = true
  try {
    const data = await api.listMcpServers()
    servers.value = data?.servers ?? []
    if (selected.value) {
      selected.value = servers.value.find(s => s.name === selected.value.name) ?? null
    }
  } catch (e) {
    showToast(e.message, 'error')
  } finally {
    loading.value = false
  }
}

function selectServer(s) {
  selected.value = selected.value?.name === s.name ? null : s
  if (activeTab.value === 'add') activeTab.value = 'servers'
}

// ── operations ──────────────────────────────────────────────────────────
async function connectServer(name) {
  busyMap.value[name] = 'connecting'
  try {
    const updated = await api.connectMcpServer(name)
    const idx = servers.value.findIndex(s => s.name === name)
    if (idx !== -1) servers.value[idx] = updated
    if (selected.value?.name === name) selected.value = updated
    showToast(updated.status === 'connected'
      ? `Connected — ${updated.tools.length} tool(s) discovered`
      : `Connection error: ${updated.error_msg}`,
      updated.status === 'connected' ? 'success' : 'error')
  } catch (e) {
    showToast(e.message, 'error')
  } finally {
    delete busyMap.value[name]
  }
}

async function approveServer(name) {
  busyMap.value[name] = 'approving'
  try {
    const updated = await api.approveMcpServer(name)
    const idx = servers.value.findIndex(s => s.name === name)
    if (idx !== -1) servers.value[idx] = updated
    if (selected.value?.name === name) selected.value = updated
    showToast(`${name} approved for LLM use`)
  } catch (e) {
    showToast(e.message, 'error')
  } finally {
    delete busyMap.value[name]
  }
}

async function deleteServer(name) {
  if (!confirm(`Remove ${name} from the MCP registry?`)) return
  try {
    await api.deleteMcpServer(name)
    servers.value = servers.value.filter(s => s.name !== name)
    if (selected.value?.name === name) selected.value = null
    showToast(`${name} removed`)
  } catch (e) {
    showToast(e.message, 'error')
  }
}

async function generateSkill(tool) {
  if (!selected.value) return
  try {
    const description = tool.description || `Use the ${tool.name} MCP tool from the ${selected.value.name} MCP server`
    await api.generateSkill({ name: tool.name, description, category: 'mcp' })
    showToast(`Skill /${tool.name} generated`)
  } catch (e) {
    showToast(e.message, 'error')
  }
}

async function submitAdd() {
  if (!addForm.value.source_ref.trim()) {
    showToast('Source ref is required', 'error')
    return
  }
  addBusy.value = true
  try {
    const payload = {
      source_kind: addForm.value.source_kind,
      source_ref:  addForm.value.source_ref.trim(),
      scope:       addForm.value.scope,
    }
    if (addForm.value.name.trim())    payload.name    = addForm.value.name.trim()
    if (addForm.value.command.trim()) payload.command = addForm.value.command.trim()
    if (addForm.value.args_raw.trim()) {
      payload.args = addForm.value.args_raw.trim().split(/\s+/)
    }
    const entry = await api.addMcpServer(payload)
    servers.value.push(entry)
    showToast(`${entry.name} added — click Connect to discover tools`)
    activeTab.value = 'servers'
    selected.value  = entry
    addForm.value   = { source_kind: 'npm', source_ref: '', name: '', command: '', args_raw: '', scope: 'project' }
  } catch (e) {
    showToast(e.message, 'error')
  } finally {
    addBusy.value = false
  }
}
</script>

<template>
  <div class="flex h-full overflow-hidden">

    <!-- ── sidebar: server list only ────────────────────────────────── -->
    <div class="w-72 shrink-0 border-r border-base-300/80 flex flex-col bg-base-200 overflow-hidden">
      <!-- sidebar header -->
      <div class="px-4 py-3.5 border-b border-base-300/80 flex items-center justify-between gap-2 shrink-0">
        <div class="flex items-center gap-2">
          <span class="font-mono text-xs font-bold uppercase tracking-widest text-base-content/50">MCP Servers</span>
          <span class="badge badge-sm badge-outline font-mono">{{ servers.length }}</span>
        </div>
        <button
          class="btn btn-ghost btn-xs font-mono"
          :class="loading && 'loading'"
          @click="refresh"
          title="Refresh"
        >⟳</button>
      </div>

      <!-- server list -->
      <div class="flex-1 overflow-y-auto py-1.5">
        <div v-if="!servers.length" class="px-4 py-10 text-center">
          <div class="text-3xl opacity-10 mb-2">⬡</div>
          <p class="font-mono text-xs text-base-content/30">No servers registered.</p>
          <button class="btn btn-primary btn-xs font-mono mt-3" @click="activeTab = 'add'">+ Add server</button>
        </div>

        <button
          v-for="s in servers" :key="s.name"
          class="w-full text-left px-4 py-3 flex items-start gap-2.5 transition-colors border-b border-base-300/20 last:border-0"
          :class="selected?.name === s.name && activeTab === 'servers'
            ? 'bg-primary/10 text-primary border-l-2 border-l-primary'
            : 'hover:bg-base-300/40 text-base-content/80'"
          @click="selectServer(s)"
        >
          <span class="mt-1.5 w-1.5 h-1.5 rounded-full shrink-0" :class="statusDot(s.status)" />
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-1.5 flex-wrap">
              <span class="font-mono text-xs font-semibold">{{ s.name }}</span>
              <span class="badge badge-xs font-mono" :class="kindBadge(s.source_kind)">{{ s.source_kind }}</span>
              <span v-if="s.approved" class="badge badge-xs badge-success font-mono">✓</span>
            </div>
            <div class="text-xs font-mono text-base-content/40 mt-0.5 truncate">{{ s.source_ref }}</div>
            <div v-if="s.tools?.length" class="text-xs font-mono text-base-content/50 mt-0.5">
              {{ s.tools.length }} tool{{ s.tools.length !== 1 ? 's' : '' }}
            </div>
          </div>
        </button>
      </div>
    </div>

    <!-- ── main panel ────────────────────────────────────────────────── -->
    <div class="flex-1 flex flex-col overflow-hidden bg-base-100">

      <!-- top bar with tabs -->
      <div class="px-5 py-3 border-b border-base-300/80 bg-base-200 flex items-center gap-4 shrink-0">
        <div class="flex gap-1">
          <button
            v-for="t in [{ id: 'servers', label: 'Servers' }, { id: 'add', label: '+ Add Server' }]"
            :key="t.id"
            class="px-3 py-1 text-xs font-mono rounded transition-colors"
            :class="activeTab === t.id
              ? 'bg-primary/15 text-primary border border-primary/30'
              : 'text-base-content/50 hover:text-base-content/80 border border-transparent'"
            @click="activeTab = t.id"
          >{{ t.label }}</button>
        </div>
        <div class="flex-1" />
        <span class="text-xs font-mono text-base-content/30">
          MCP servers extend your AI with external tools via subprocess
        </span>
      </div>

      <!-- panel content -->
      <div class="flex-1 overflow-y-auto">

        <!-- ── SERVERS TAB ── -->
        <template v-if="activeTab === 'servers'">

          <!-- empty state (no server selected) -->
          <div v-if="!selected" class="flex flex-col items-center justify-center h-full gap-3 text-base-content/30">
            <span class="text-6xl opacity-10">⬡</span>
            <p class="font-mono text-sm">Select a server to inspect</p>
            <p class="font-mono text-xs opacity-60">or click <strong class="font-mono">+ Add Server</strong> to register one</p>
          </div>

          <!-- server detail -->
          <div v-else class="p-6 max-w-3xl">

            <!-- header row -->
            <div class="flex items-start justify-between gap-4 mb-6">
              <div>
                <div class="flex items-center gap-2.5 flex-wrap mb-1">
                  <span class="w-2 h-2 rounded-full shrink-0" :class="statusDot(selected.status)" />
                  <h2 class="font-mono text-lg font-bold text-base-content/90">{{ selected.name }}</h2>
                  <span class="badge badge-sm font-mono" :class="kindBadge(selected.source_kind)">{{ selected.source_kind }}</span>
                  <span v-if="selected.approved" class="badge badge-sm badge-success font-mono">✓ approved</span>
                </div>
                <p class="font-mono text-xs text-base-content/40">{{ selected.source_ref }}</p>
              </div>
              <div class="flex gap-2 shrink-0 flex-wrap">
                <button
                  class="btn btn-sm btn-outline font-mono"
                  :class="busyMap[selected.name] === 'connecting' && 'loading'"
                  :disabled="!!busyMap[selected.name]"
                  @click="connectServer(selected.name)"
                >
                  {{ selected.status === 'connected' ? '↺ Reconnect' : '⚡ Connect' }}
                </button>
                <button
                  v-if="!selected.approved"
                  class="btn btn-sm btn-success font-mono"
                  :class="busyMap[selected.name] === 'approving' && 'loading'"
                  :disabled="!!busyMap[selected.name]"
                  @click="approveServer(selected.name)"
                >
                  🛡 Approve
                </button>
                <button class="btn btn-sm btn-ghost text-error font-mono" @click="deleteServer(selected.name)">✕</button>
              </div>
            </div>

            <!-- approval warning -->
            <div v-if="!selected.approved" class="alert alert-warning mb-5 py-2.5 px-4 gap-2">
              <span class="text-base">🛡</span>
              <div>
                <p class="text-sm font-mono font-semibold">Not approved for LLM use</p>
                <p class="text-xs font-mono opacity-80 mt-0.5">Connect first to review discovered tools, then click Approve to enable this server in agent workflows.</p>
              </div>
            </div>

            <!-- stats row -->
            <div class="grid grid-cols-3 gap-3 mb-5">
              <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
                <p class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider mb-1.5">Status</p>
                <div class="flex items-center gap-1.5">
                  <span class="w-1.5 h-1.5 rounded-full" :class="statusDot(selected.status)" />
                  <span class="font-mono text-xs text-base-content/80">{{ selected.status }}</span>
                </div>
                <p v-if="selected.error_msg" class="font-mono text-xs text-error/80 mt-1 leading-tight">{{ selected.error_msg }}</p>
              </div>
              <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
                <p class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider mb-1.5">Tools</p>
                <span class="font-mono text-xl font-bold text-primary">{{ selected.tools?.length ?? 0 }}</span>
              </div>
              <div class="bg-base-200 rounded-lg p-3 border border-base-300/50">
                <p class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider mb-1.5">Scope</p>
                <span class="badge badge-sm badge-outline font-mono">{{ selected.scope }}</span>
              </div>
            </div>

            <!-- command -->
            <div class="bg-base-200 rounded-lg border border-base-300/50 px-4 py-3 mb-5">
              <p class="text-[10px] font-mono text-base-content/40 uppercase tracking-wider mb-1.5">Command</p>
              <code class="font-mono text-xs text-base-content/70 break-all">{{ selected.command }} {{ selected.args?.join(' ') }}</code>
            </div>

            <!-- tools list -->
            <div v-if="selected.tools?.length" class="mb-5">
              <h3 class="font-mono text-xs font-bold text-base-content/50 uppercase tracking-wider mb-2.5">
                Discovered Tools ({{ selected.tools.length }})
              </h3>
              <div class="space-y-1.5">
                <div
                  v-for="tool in selected.tools" :key="tool.name"
                  class="bg-base-200 rounded-lg border border-base-300/50 px-4 py-2.5 flex items-start justify-between gap-4"
                >
                  <div class="min-w-0 flex-1">
                    <span class="font-mono text-sm font-semibold text-primary">{{ tool.name }}</span>
                    <p v-if="tool.description" class="font-mono text-xs text-base-content/50 mt-0.5 leading-relaxed">
                      {{ tool.description }}
                    </p>
                  </div>
                  <button
                    class="btn btn-xs btn-ghost font-mono shrink-0 text-xs gap-1"
                    @click="generateSkill(tool)"
                    title="Generate a skill for this tool"
                  >
                    ⚡ skill
                  </button>
                </div>
              </div>
            </div>

            <div v-else-if="selected.status === 'connected'" class="bg-base-200 rounded-lg border border-base-300/50 px-4 py-6 text-center mb-5">
              <p class="font-mono text-xs text-base-content/40">No tools discovered from this server.</p>
            </div>

            <div v-else class="bg-base-200 rounded-lg border border-dashed border-base-300 px-4 py-8 text-center mb-5">
              <p class="font-mono text-xs text-base-content/30 mb-3">Connect the server to discover its tools.</p>
              <button
                class="btn btn-sm btn-primary font-mono"
                :class="busyMap[selected.name] === 'connecting' && 'loading'"
                :disabled="!!busyMap[selected.name]"
                @click="connectServer(selected.name)"
              >⚡ Connect Now</button>
            </div>

            <!-- env vars -->
            <div v-if="selected.env && Object.keys(selected.env).length" class="mb-5">
              <h3 class="font-mono text-xs font-bold text-base-content/50 uppercase tracking-wider mb-2">Environment</h3>
              <div class="bg-base-200 rounded-lg border border-base-300/50 p-3 font-mono text-xs space-y-1">
                <div v-for="(val, key) in selected.env" :key="key" class="flex gap-2">
                  <span class="text-accent">{{ key }}</span>
                  <span class="text-base-content/50">=</span>
                  <span class="text-base-content/70">{{ val }}</span>
                </div>
              </div>
            </div>

          </div>
        </template>

        <!-- ── ADD SERVER TAB ── -->
        <div v-if="activeTab === 'add'" class="p-6 max-w-xl">
          <h2 class="font-mono text-base font-bold text-base-content/80 mb-1">Register MCP Server</h2>
          <p class="font-mono text-xs text-base-content/40 mb-6">
            MCP servers extend your AI assistant with tools via a subprocess. npm packages are launched via <code class="bg-base-200 px-1 rounded">npx -y</code>.
          </p>

          <div class="space-y-4">

            <div class="form-control gap-1.5">
              <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Source Kind</span></label>
              <select v-model="addForm.source_kind" class="select select-bordered select-sm font-mono text-sm w-full">
                <option value="npm">npm package (launched via npx)</option>
                <option value="local">local binary or script</option>
                <option value="github">github repository</option>
                <option value="http">http endpoint</option>
                <option value="manual">manual (custom command)</option>
              </select>
            </div>

            <div class="form-control gap-1.5">
              <label class="label py-0">
                <span class="label-text text-xs font-mono font-semibold text-base-content/60">
                  {{ addForm.source_kind === 'npm' ? 'npm Package Name' : 'Source Reference' }}
                </span>
              </label>
              <input
                v-model="addForm.source_ref"
                type="text"
                class="input input-bordered input-sm font-mono text-sm w-full"
                :placeholder="addForm.source_kind === 'npm'
                  ? '@modelcontextprotocol/server-filesystem'
                  : addForm.source_kind === 'local' ? '/usr/local/bin/my-mcp-server' : '/path/to/server or URL'"
              />
              <div v-if="addForm.source_kind === 'npm' && addForm.source_ref" class="text-xs font-mono text-info/70 flex items-center gap-1.5 mt-1">
                <span>→</span>
                <span>Will run: <code class="bg-base-200 px-1 rounded">npx -y {{ addForm.source_ref }}</code></span>
              </div>
            </div>

            <template v-if="addForm.source_kind !== 'npm'">
              <div class="form-control gap-1.5">
                <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Command</span></label>
                <input v-model="addForm.command" type="text" class="input input-bordered input-sm font-mono text-sm w-full" placeholder="node server.js" />
              </div>
              <div class="form-control gap-1.5">
                <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Args (space-separated)</span></label>
                <input v-model="addForm.args_raw" type="text" class="input input-bordered input-sm font-mono text-sm w-full" placeholder="--port 3000 /workspace" />
              </div>
            </template>

            <div class="grid grid-cols-2 gap-3">
              <div class="form-control gap-1.5">
                <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Name <span class="font-normal opacity-60">(optional)</span></span></label>
                <input v-model="addForm.name" type="text" class="input input-bordered input-sm font-mono text-sm w-full" placeholder="auto-derived" />
              </div>
              <div class="form-control gap-1.5">
                <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Scope</span></label>
                <select v-model="addForm.scope" class="select select-bordered select-sm font-mono text-sm w-full">
                  <option value="project">project</option>
                  <option value="global">global</option>
                </select>
              </div>
            </div>

            <div class="flex justify-end pt-2">
              <button
                class="btn btn-primary btn-sm font-mono px-6"
                :class="addBusy && 'loading'"
                :disabled="addBusy || !addForm.source_ref.trim()"
                @click="submitAdd"
              >Register Server</button>
            </div>
          </div>
        </div>

      </div>
    </div>
  </div>

  <!-- toast -->
  <Transition name="toast">
    <div v-if="toast" class="fixed bottom-6 right-6 z-50">
      <div class="alert shadow-lg py-2 px-4 font-mono text-sm max-w-xs"
        :class="{
          'alert-success': toast.type === 'success',
          'alert-error':   toast.type === 'error',
          'alert-warning': toast.type === 'warning',
        }">
        {{ toast.msg }}
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.toast-enter-active, .toast-leave-active { transition: all .25s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(8px); }
</style>
