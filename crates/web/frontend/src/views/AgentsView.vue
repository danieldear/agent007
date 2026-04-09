<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api, loading } = useApi()
const personas = ref([])
const showForm = ref(false)
const editTarget = ref(null)

const KNOWN_TOOLS = [
  { name: 'bash',       desc: 'shell execution' },
  { name: 'file_read',  desc: 'read files' },
  { name: 'file_write', desc: 'create files' },
  { name: 'file_edit',  desc: 'modify files' },
  { name: 'web_search', desc: 'search the web' },
  { name: 'web_fetch',  desc: 'fetch URLs' },
]
const KNOWN_TOOL_NAMES = KNOWN_TOOLS.map(t => t.name)

const form = ref({
  name: '',
  description: '',
  preferred_model: 'codex',
  allowed_tools: [],
  custom_tools: '',
  system_prompt: '',
})

onMounted(loadPersonas)

async function loadPersonas() {
  const data = await api.listPersonas()
  if (data) personas.value = data
}

function openCreate() {
  editTarget.value = null
  form.value = { name: '', description: '', preferred_model: 'codex', allowed_tools: [], custom_tools: '', system_prompt: '' }
  showForm.value = true
}

function openEdit(p) {
  editTarget.value = p.name
  const all = p.allowed_tools || []
  form.value = {
    name: p.name,
    description: p.description || '',
    preferred_model: p.preferred_model || 'codex',
    allowed_tools: all.filter(t => KNOWN_TOOL_NAMES.includes(t)),
    custom_tools: all.filter(t => !KNOWN_TOOL_NAMES.includes(t)).join(', '),
    system_prompt: p.system_prompt || '',
  }
  showForm.value = true
}

async function savePersona() {
  const custom = form.value.custom_tools.split(',').map(s => s.trim()).filter(Boolean)
  const payload = {
    name: form.value.name,
    description: form.value.description,
    preferred_model: form.value.preferred_model,
    system_prompt: form.value.system_prompt,
    allowed_tools: [...form.value.allowed_tools, ...custom],
  }
  await api.savePersona(payload)
  showForm.value = false
  await loadPersonas()
}

async function deletePersona(name) {
  if (!confirm(`Delete agent "${name}"?`)) return
  await api.deletePersona(name)
  await loadPersonas()
}

function selectAllTools() {
  form.value.allowed_tools = [...KNOWN_TOOL_NAMES]
}

function clearAllTools() {
  form.value.allowed_tools = []
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Agents · Personas</span>
      <button class="btn btn-sm btn-primary font-mono text-xs px-4" @click="openCreate">+ new</button>
    </div>

    <div class="flex-1 overflow-auto p-5">
      <!-- Agent cards grid -->
      <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        <div
          v-for="p in personas"
          :key="p.name"
          class="bg-base-200 border border-base-300 rounded-lg border-l-2 border-l-primary/50 hover:border-l-primary transition-colors"
        >
          <div class="p-4">
            <div class="flex items-start justify-between gap-2">
              <div class="min-w-0">
                <div class="font-mono text-sm font-bold text-primary truncate">{{ p.name }}</div>
                <p class="text-[11px] font-mono text-base-content/45 mt-1 leading-relaxed">{{ p.description }}</p>
              </div>
              <div class="flex gap-1 shrink-0">
                <button class="btn btn-ghost btn-xs font-mono text-[11px]" @click="openEdit(p)">edit</button>
                <button class="btn btn-ghost btn-xs font-mono text-[11px] text-error/70 hover:text-error" @click="deletePersona(p.name)">del</button>
              </div>
            </div>
            <div class="mt-3 flex flex-wrap gap-1.5">
              <span class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-info/30 text-info/70 bg-info/5">{{ p.preferred_model }}</span>
              <span
                v-for="tool in (p.allowed_tools || []).slice(0, 5)"
                :key="tool"
                class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-base-300 text-base-content/40"
              >{{ tool }}</span>
              <span
                v-if="(p.allowed_tools || []).length > 5"
                class="text-[10px] font-mono text-base-content/30"
              >+{{ p.allowed_tools.length - 5 }} more</span>
            </div>
            <details class="mt-3">
              <summary class="text-[10px] font-mono cursor-pointer text-base-content/30 hover:text-base-content/60 uppercase tracking-wider">
                ▸ system prompt
              </summary>
              <pre class="mt-2 text-[11px] font-mono bg-base-300/50 p-3 rounded border border-base-300/60 max-h-32 overflow-auto whitespace-pre-wrap text-base-content/70">{{ p.system_prompt }}</pre>
            </details>
          </div>
        </div>
      </div>

      <div v-if="!personas.length && !loading" class="text-center font-mono text-base-content/30 py-16 text-sm">
        <div class="text-3xl mb-3 text-base-content/10">◉</div>
        no agents found — click <span class="text-primary">+ new</span> to create one
      </div>
    </div>

    <!-- Create/Edit modal -->
    <dialog :open="showForm" class="modal" :class="{ 'modal-open': showForm }">
      <div class="modal-box max-w-2xl bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden">
        <!-- Header bar -->
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">
            {{ editTarget ? `edit · ${editTarget}` : 'create agent' }}
          </span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="showForm = false; editTarget = null">✕</button>
        </div>

        <!-- Body -->
        <div class="p-5 space-y-4">
          <!-- Name -->
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">name</div>
            <input
              v-model="form.name"
              class="persona-input w-full"
              placeholder="My Agent"
              :disabled="!!editTarget"
              :class="{ 'opacity-40 cursor-not-allowed': !!editTarget }"
            />
          </div>

          <!-- Description -->
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">description</div>
            <input v-model="form.description" class="persona-input w-full" placeholder="What this agent specialises in" />
          </div>

          <!-- Preferred model (pill buttons) -->
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">preferred model</div>
            <div class="flex gap-1.5 flex-wrap">
              <button
                v-for="m in ['codex', 'gpt-5.3-codex', 'claude', 'claude-sonnet-4-6', 'ollama']"
                :key="m"
                type="button"
                class="px-3 py-1 text-[11px] font-mono rounded border transition-colors"
                :class="form.preferred_model === m
                  ? 'bg-primary/15 border-primary/50 text-primary'
                  : 'bg-base-200 border-base-300 text-base-content/50 hover:border-base-content/30'"
                @click="form.preferred_model = m"
              >{{ m }}</button>
            </div>
          </div>

          <!-- Allowed tools -->
          <div>
            <div class="flex items-center justify-between mb-2">
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest">allowed tools</div>
              <div class="flex gap-2 items-center">
                <button type="button" class="text-[10px] font-mono text-primary/60 hover:text-primary transition-colors" @click="selectAllTools">all</button>
                <span class="text-base-content/20 text-[10px]">·</span>
                <button type="button" class="text-[10px] font-mono text-base-content/35 hover:text-base-content/70 transition-colors" @click="clearAllTools">none</button>
              </div>
            </div>
            <div class="grid grid-cols-2 sm:grid-cols-3 gap-1.5">
              <label
                v-for="tool in KNOWN_TOOLS"
                :key="tool.name"
                class="flex items-center gap-2 px-2.5 py-2 rounded border cursor-pointer transition-all select-none"
                :class="form.allowed_tools.includes(tool.name)
                  ? 'bg-primary/10 border-primary/40 text-primary'
                  : 'bg-base-200 border-base-300 text-base-content/50 hover:border-base-content/30'"
              >
                <input type="checkbox" class="hidden" :value="tool.name" v-model="form.allowed_tools" />
                <span
                  class="w-3 h-3 rounded-sm border flex-shrink-0 flex items-center justify-center text-[8px] leading-none transition-colors"
                  :class="form.allowed_tools.includes(tool.name)
                    ? 'bg-primary border-primary text-primary-content'
                    : 'bg-base-300 border-base-content/20'"
                >{{ form.allowed_tools.includes(tool.name) ? '✓' : '' }}</span>
                <span class="text-[11px] font-mono font-semibold">{{ tool.name }}</span>
                <span class="text-[9px] font-mono text-base-content/30 ml-auto truncate">{{ tool.desc }}</span>
              </label>
            </div>
            <!-- Custom tools fallback -->
            <div class="mt-2">
              <input
                v-model="form.custom_tools"
                class="persona-input w-full text-[11px]"
                placeholder="+ additional tools (comma-separated)"
              />
            </div>
          </div>

          <!-- System prompt -->
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">system prompt</div>
            <textarea
              v-model="form.system_prompt"
              class="w-full bg-base-200 border border-base-content/15 rounded text-[13px] font-mono text-base-content/80 p-3 h-52 resize-y focus:outline-none focus:border-primary/50 transition-colors leading-relaxed"
              placeholder="You are a specialized agent that..."
            />
          </div>
        </div>

        <!-- Footer -->
        <div class="flex items-center justify-end gap-2 px-5 py-3 bg-base-200 border-t border-base-300">
          <button class="btn btn-sm btn-ghost font-mono text-xs px-4" @click="showForm = false; editTarget = null">cancel</button>
          <button
            class="btn btn-sm btn-primary font-mono text-xs px-4"
            @click="savePersona"
            :disabled="!form.name"
          >{{ editTarget ? 'save changes' : 'save agent' }}</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showForm = false; editTarget = null">close</button></form>
    </dialog>
  </div>
</template>

<style scoped>
.persona-input {
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
.persona-input:focus {
  border-color: color-mix(in oklch, var(--color-primary, var(--p)) 50%, transparent);
}
.persona-input:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>
