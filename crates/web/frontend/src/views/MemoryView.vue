<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()

const SCOPES = ['global', 'project', 'user', 'learning']
const activeScope = ref('project')
const keys = ref([])
const selectedKey = ref(null)
const content = ref('')
const loadingKeys = ref(false)
const loadingContent = ref(false)
const contentError = ref(null)

onMounted(() => loadKeys('project'))

async function loadKeys(scope) {
  activeScope.value = scope
  selectedKey.value = null
  content.value = ''
  contentError.value = null
  loadingKeys.value = true
  try {
    keys.value = (await api.listMemory(scope)) || []
  } catch {
    keys.value = []
  } finally {
    loadingKeys.value = false
  }
}

async function selectKey(key) {
  selectedKey.value = key
  content.value = ''
  contentError.value = null
  loadingContent.value = true
  try {
    content.value = await api.getMemory(activeScope.value, key)
  } catch (e) {
    contentError.value = e.message
  } finally {
    loadingContent.value = false
  }
}

/**
 * Safe markdown renderer.
 * Step 1: escape ALL HTML entities in raw input (prevents XSS).
 * Step 2: apply markdown patterns only to the already-escaped text,
 *         injecting a small set of safe tags with no user-controlled attributes.
 */
function renderMarkdown(raw) {
  if (!raw) return ''

  // 1 — escape ALL HTML so no injected markup survives
  let out = raw
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')

  // 2 — fenced code blocks (``` ... ```)
  out = out.replace(/```([^\n]*)\n([\s\S]*?)```/g, (_, lang, code) =>
    `<pre class="md-pre"><code>${code}</code></pre>`)

  // 3 — inline code (must run before bold/italic)
  out = out.replace(/`([^`\n]+)`/g, '<code class="md-code">$1</code>')

  // 4 — headings (only at start of line)
  out = out.replace(/^### (.+)$/gm, '<h3 class="md-h3">$1</h3>')
  out = out.replace(/^## (.+)$/gm,  '<h2 class="md-h2">$1</h2>')
  out = out.replace(/^# (.+)$/gm,   '<h1 class="md-h1">$1</h1>')

  // 5 — bold and italic
  out = out.replace(/\*\*\*([^*\n]+)\*\*\*/g, '<strong><em>$1</em></strong>')
  out = out.replace(/\*\*([^*\n]+)\*\*/g,     '<strong>$1</strong>')
  out = out.replace(/\*([^*\n]+)\*/g,          '<em>$1</em>')

  // 6 — horizontal rule
  out = out.replace(/^[-*]{3,}$/gm, '<hr class="md-hr" />')

  // 7 — unordered list items
  out = out.replace(/^[*\-] (.+)$/gm, '<li class="md-li">$1</li>')

  // 8 — blank lines become paragraph breaks; single newlines become <br>
  out = out
    .split(/\n{2,}/)
    .map(block => {
      if (/^<(h[1-3]|pre|li|hr)/.test(block)) return block
      return `<p class="md-p">${block.replace(/\n/g, '<br>')}</p>`
    })
    .join('\n')

  return out
}
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">
    <!-- Header -->
    <div class="p-4 border-b border-base-300 bg-base-200 flex-shrink-0">
      <h2 class="text-lg font-bold">Memory Store</h2>
    </div>

    <!-- Scope tabs -->
    <div class="flex gap-2 px-4 py-2 border-b border-base-300 bg-base-200 flex-shrink-0">
      <button
        v-for="s in SCOPES"
        :key="s"
        class="btn btn-xs"
        :class="activeScope === s ? 'btn-primary' : 'btn-ghost'"
        @click="loadKeys(s)"
      >{{ s }}</button>
    </div>

    <!-- Two-column body -->
    <div class="flex flex-1 overflow-hidden">

      <!-- Left: key list -->
      <div class="w-56 flex-shrink-0 border-r border-base-300 flex flex-col overflow-hidden">
        <div class="px-3 py-1.5 text-xs font-bold uppercase tracking-wider text-base-content/40 border-b border-base-300 bg-base-200 flex-shrink-0">
          {{ loadingKeys ? 'Loading…' : `${keys.length} keys` }}
        </div>
        <div class="flex-1 overflow-y-auto">
          <div
            v-for="key in keys"
            :key="key"
            class="px-3 py-1.5 text-xs font-mono cursor-pointer border-b border-base-300/40 transition-colors"
            :class="selectedKey === key
              ? 'bg-primary/20 text-primary border-l-2 border-l-primary'
              : 'hover:bg-base-300/40 text-base-content/70'"
            @click="selectKey(key)"
            :title="key"
          >
            <span class="block truncate">{{ key }}</span>
          </div>
          <div v-if="!loadingKeys && !keys.length" class="p-4 text-center text-base-content/30 text-xs">
            No keys in "{{ activeScope }}"
          </div>
        </div>
      </div>

      <!-- Right: markdown preview -->
      <div class="flex-1 flex flex-col overflow-hidden">
        <!-- Preview header -->
        <div class="px-4 py-1.5 border-b border-base-300 bg-base-200 text-xs font-mono text-base-content/40 flex-shrink-0 flex items-center gap-2">
          <span v-if="selectedKey" class="text-info font-semibold">{{ activeScope }}/{{ selectedKey }}</span>
          <span v-else class="italic">select a key to preview</span>
          <span v-if="loadingContent" class="loading loading-spinner loading-xs ml-auto"></span>
        </div>

        <!-- Content area -->
        <div class="flex-1 overflow-y-auto p-4">
          <div v-if="contentError" class="text-error text-sm font-mono">
            Error: {{ contentError }}
          </div>
          <div
            v-else-if="content"
            class="md-preview prose prose-sm max-w-none"
            v-html="renderMarkdown(content)"
          ></div>
          <div v-else-if="!selectedKey" class="h-full flex items-center justify-center text-base-content/20 text-sm">
            <div class="text-center">
              <div class="text-4xl mb-2">🧠</div>
              Click a memory key to read its contents
            </div>
          </div>
        </div>
      </div>

    </div>
  </div>
</template>

<style scoped>
/* Markdown preview styles */
.md-preview :deep(h1.md-h1) { font-size: 1.25rem; font-weight: 700; margin: 0.75rem 0 0.4rem; color: var(--fallback-p, oklch(var(--p))); }
.md-preview :deep(h2.md-h2) { font-size: 1.1rem;  font-weight: 700; margin: 0.65rem 0 0.35rem; color: var(--fallback-p, oklch(var(--p))); }
.md-preview :deep(h3.md-h3) { font-size: 0.95rem; font-weight: 600; margin: 0.5rem 0 0.3rem;  color: var(--fallback-s, oklch(var(--s))); }
.md-preview :deep(p.md-p)   { margin: 0.3rem 0; line-height: 1.6; font-size: 0.82rem; }
.md-preview :deep(li.md-li) { margin-left: 1.2rem; list-style: disc; font-size: 0.82rem; line-height: 1.5; }
.md-preview :deep(hr.md-hr) { border: none; border-top: 1px solid oklch(var(--b3)); margin: 0.6rem 0; }
.md-preview :deep(code.md-code) {
  background: oklch(var(--b3));
  color: oklch(var(--s));
  padding: 0.1em 0.35em;
  border-radius: 3px;
  font-size: 0.78rem;
  font-family: monospace;
}
.md-preview :deep(pre.md-pre) {
  background: oklch(var(--b3));
  border: 1px solid oklch(var(--b2));
  border-radius: 6px;
  padding: 0.75rem 1rem;
  overflow-x: auto;
  margin: 0.5rem 0;
}
.md-preview :deep(pre.md-pre code) {
  background: none;
  padding: 0;
  color: oklch(var(--bc));
  font-size: 0.78rem;
  line-height: 1.5;
}
</style>

