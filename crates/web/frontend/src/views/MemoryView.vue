<script setup>
import { ref, computed, onMounted, watch, nextTick } from 'vue'
import { useApi } from '../composables/useApi.js'
import { marked } from 'marked'
import DOMPurify from 'dompurify'

// Configure marked for GFM
marked.setOptions({ gfm: true, breaks: true })

const { api } = useApi()

const SCOPES = ['global', 'project', 'user', 'learning']
const SCOPE_ICONS = { global: '🌐', project: '📁', user: '👤', learning: '📈' }

const activeScope = ref('project')
const keys = ref([])
const selectedKey = ref(null)
const content = ref('')
const loadingKeys = ref(false)
const loadingContent = ref(false)
const contentError = ref(null)
const searchQuery = ref('')
const copyStatus = ref('')
const contentVisible = ref(false)

onMounted(() => loadKeys('project'))

const filteredKeys = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return keys.value
  return keys.value.filter(k => k.toLowerCase().includes(q))
})

// Split colon-key into visual path segments
function keySegments(key) {
  return key.split(':')
}

async function loadKeys(scope) {
  activeScope.value = scope
  selectedKey.value = null
  content.value = ''
  contentError.value = null
  contentVisible.value = false
  searchQuery.value = ''
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
  if (selectedKey.value === key) return
  selectedKey.value = key
  content.value = ''
  contentError.value = null
  contentVisible.value = false
  loadingContent.value = true
  try {
    content.value = await api.getMemory(activeScope.value, key)
    setTimeout(() => { contentVisible.value = true }, 20)
  } catch (e) {
    contentError.value = e.message
    contentVisible.value = true
  } finally {
    loadingContent.value = false
  }
}

async function copyContent() {
  if (!content.value) return
  try {
    await navigator.clipboard.writeText(content.value)
    copyStatus.value = 'Copied!'
    setTimeout(() => { copyStatus.value = '' }, 2000)
  } catch {
    copyStatus.value = 'Failed'
    setTimeout(() => { copyStatus.value = '' }, 2000)
  }
}

// Render markdown via marked (GFM) + DOMPurify (XSS safe)
function renderMarkdown(raw) {
  if (!raw) return ''
  const dirty = marked.parse(raw)
  return DOMPurify.sanitize(dirty, {
    ADD_TAGS: ['pre', 'code'],
    ADD_ATTR: ['class'],
  })
}

// After content renders, find all .language-mermaid blocks and render them as SVG.
// Mermaid is lazy-loaded on first use to avoid bundling it into the main chunk.
let mermaidReady = false
async function ensureMermaid() {
  if (mermaidReady) return
  const mermaid = (await import('mermaid')).default
  mermaid.initialize({
    startOnLoad: false,
    theme: 'dark',
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
  mermaidReady = true
  return mermaid
}

watch(contentVisible, async (visible) => {
  if (!visible) return
  await nextTick()
  const blocks = document.querySelectorAll('.md-preview .language-mermaid')
  if (!blocks.length) return
  blocks.forEach(block => {
    const source = block.textContent || ''
    const container = block.closest('pre') || block
    container.outerHTML = `<div class="mermaid-block">${source}</div>`
  })
  await nextTick()
  try {
    const mermaid = await ensureMermaid()
    await mermaid.run({ querySelector: '.mermaid-block' })
  } catch (e) {
    console.warn('mermaid render error:', e)
  }
})
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">

    <!-- ── Top bar ─────────────────────────────────────────────────────── -->
    <div class="flex items-center gap-3 px-5 py-3 border-b border-base-300 bg-base-200 flex-shrink-0">
      <span class="text-primary font-mono font-bold tracking-widest text-sm uppercase">◈ Memory</span>
      <div class="h-4 w-px bg-base-300"></div>
      <!-- Scope pills -->
      <div class="flex gap-1">
        <button
          v-for="s in SCOPES" :key="s"
          class="scope-pill"
          :class="activeScope === s ? 'scope-pill--active' : ''"
          @click="loadKeys(s)"
        >
          <span class="scope-pill-icon">{{ SCOPE_ICONS[s] }}</span>
          <span>{{ s }}</span>
        </button>
      </div>
      <div class="flex-1"></div>
      <!-- Key count badge -->
      <span class="text-xs font-mono text-base-content/30">
        {{ loadingKeys ? '…' : `${filteredKeys.length} / ${keys.length}` }} keys
      </span>
    </div>

    <!-- ── Body ───────────────────────────────────────────────────────── -->
    <div class="flex flex-1 overflow-hidden">

      <!-- Left: key navigator -->
      <div class="key-panel">

        <!-- Search -->
        <div class="px-3 pt-3 pb-2 flex-shrink-0">
          <div class="search-wrap">
            <span class="search-icon">⌕</span>
            <input
              v-model="searchQuery"
              class="search-input"
              placeholder="Filter keys…"
              type="text"
              spellcheck="false"
            />
            <button v-if="searchQuery" class="search-clear" @click="searchQuery = ''">&times;</button>
          </div>
        </div>

        <!-- Key list -->
        <div class="flex-1 overflow-y-auto px-2 pb-3">
          <!-- Skeleton loader -->
          <template v-if="loadingKeys">
            <div v-for="i in 6" :key="i" class="skeleton-key" :style="`opacity:${1 - i * 0.12}`"></div>
          </template>

          <!-- Keys -->
          <template v-else>
            <button
              v-for="key in filteredKeys"
              :key="key"
              class="key-item"
              :class="selectedKey === key ? 'key-item--active' : ''"
              @click="selectKey(key)"
              :title="key"
            >
              <span class="key-item-dot" :class="selectedKey === key ? 'key-item-dot--active' : ''"></span>
              <span class="key-item-body">
                <span
                  v-for="(seg, idx) in keySegments(key)"
                  :key="idx"
                  class="key-seg"
                  :class="{ 'key-seg--leaf': idx === keySegments(key).length - 1 }"
                ><span v-if="idx > 0" class="key-sep"> › </span>{{ seg }}</span>
              </span>
            </button>

            <div v-if="!filteredKeys.length && !loadingKeys" class="empty-keys">
              <div class="empty-keys-icon">{{ searchQuery ? '⊘' : '◈' }}</div>
              <div>{{ searchQuery ? 'No matches' : `Empty scope` }}</div>
            </div>
          </template>
        </div>
      </div>

      <!-- Right: content viewer -->
      <div class="preview-panel">

        <!-- Preview toolbar -->
        <div class="preview-toolbar">
          <template v-if="selectedKey">
            <!-- Breadcrumb -->
            <div class="breadcrumb">
              <span class="breadcrumb-scope">{{ SCOPE_ICONS[activeScope] }} {{ activeScope }}</span>
              <span class="breadcrumb-sep"> / </span>
              <span
                v-for="(seg, idx) in keySegments(selectedKey)"
                :key="idx"
              >
                <span v-if="idx > 0" class="breadcrumb-sep"> › </span>
                <span :class="idx === keySegments(selectedKey).length - 1 ? 'breadcrumb-leaf' : 'breadcrumb-node'">{{ seg }}</span>
              </span>
            </div>
            <button v-if="content && !loadingContent" class="copy-btn" @click="copyContent">
              <span>{{ copyStatus || '⎘ Copy' }}</span>
            </button>
          </template>
          <span v-else class="preview-hint">← select a key</span>
          <span v-if="loadingContent" class="loading loading-spinner loading-xs ml-auto text-primary/40"></span>
        </div>

        <!-- Content -->
        <div class="preview-body">

          <!-- Skeleton content loader -->
          <div v-if="loadingContent" class="skeleton-content">
            <div class="skeleton-line skeleton-line--h1"></div>
            <div class="skeleton-line skeleton-line--p"></div>
            <div class="skeleton-line skeleton-line--p w-3/4"></div>
            <div class="skeleton-line skeleton-line--p w-1/2"></div>
            <div class="mt-4 skeleton-line skeleton-line--h2"></div>
            <div class="skeleton-line skeleton-line--p"></div>
            <div class="skeleton-line skeleton-line--p w-5/6"></div>
          </div>

          <!-- Error -->
          <div v-else-if="contentError" class="error-state">
            <div class="text-2xl mb-2">⚠</div>
            <div class="font-mono text-xs">{{ contentError }}</div>
          </div>

          <!-- Rendered markdown -->
          <transition name="fade-up">
            <div
              v-if="contentVisible && content && !contentError"
              class="md-preview font-mono"
              v-html="renderMarkdown(content)"
            ></div>
          </transition>

          <!-- Empty state -->
          <div v-if="!selectedKey && !loadingContent" class="empty-preview">
            <div class="empty-preview-glyph">◈</div>
            <div class="empty-preview-title">Memory Viewer</div>
            <div class="empty-preview-sub">Select a key from the left panel<br>to read its contents</div>
          </div>
        </div>

      </div>
    </div>
  </div>
</template>

<style scoped>
/* ── Scope pills ─────────────────────────────────────────────────────── */
.scope-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 10px;
  border-radius: 20px;
  font-size: 0.7rem;
  font-family: monospace;
  font-weight: 600;
  letter-spacing: 0.04em;
  border: 1px solid oklch(var(--b3));
  background: oklch(var(--b3) / 0.4);
  color: oklch(var(--bc) / 0.5);
  cursor: pointer;
  transition: all 0.15s;
}
.scope-pill:hover { border-color: oklch(var(--p) / 0.5); color: oklch(var(--bc) / 0.8); }
.scope-pill--active {
  border-color: oklch(var(--p) / 0.7);
  background: oklch(var(--p) / 0.12);
  color: oklch(var(--p));
}
.scope-pill-icon { font-size: 0.72rem; }

/* ── Left key panel ──────────────────────────────────────────────────── */
.key-panel {
  width: 240px;
  flex-shrink: 0;
  border-right: 1px solid oklch(var(--b3));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: oklch(var(--b2) / 0.5);
}

/* ── Search ──────────────────────────────────────────────────────────── */
.search-wrap {
  position: relative;
  display: flex;
  align-items: center;
}
.search-icon {
  position: absolute;
  left: 8px;
  font-size: 0.85rem;
  color: oklch(var(--bc) / 0.3);
  pointer-events: none;
  line-height: 1;
}
.search-input {
  width: 100%;
  background: oklch(var(--b3) / 0.6);
  border: 1px solid oklch(var(--b3));
  border-radius: 6px;
  padding: 5px 28px 5px 26px;
  font-size: 0.75rem;
  font-family: monospace;
  color: oklch(var(--bc));
  outline: none;
  transition: border-color 0.15s;
}
.search-input:focus { border-color: oklch(var(--p) / 0.5); }
.search-input::placeholder { color: oklch(var(--bc) / 0.25); }
.search-clear {
  position: absolute;
  right: 8px;
  font-size: 0.9rem;
  color: oklch(var(--bc) / 0.3);
  cursor: pointer;
  line-height: 1;
  background: none;
  border: none;
}
.search-clear:hover { color: oklch(var(--bc) / 0.7); }

/* ── Key items ───────────────────────────────────────────────────────── */
.key-item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 6px 8px;
  border-radius: 6px;
  border: 1px solid transparent;
  background: none;
  cursor: pointer;
  text-align: left;
  transition: background 0.12s, border-color 0.12s;
  margin-bottom: 2px;
}
.key-item:hover {
  background: oklch(var(--b3) / 0.7);
  border-color: oklch(var(--b3));
}
.key-item--active {
  background: oklch(var(--p) / 0.1);
  border-color: oklch(var(--p) / 0.35);
}
.key-item-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: oklch(var(--bc) / 0.2);
  flex-shrink: 0;
  transition: background 0.15s;
}
.key-item-dot--active { background: oklch(var(--p)); box-shadow: 0 0 6px oklch(var(--p) / 0.5); }
.key-item-body {
  font-size: 0.72rem;
  font-family: monospace;
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}
.key-seg { color: oklch(var(--bc) / 0.45); }
.key-seg--leaf { color: oklch(var(--bc) / 0.85); font-weight: 600; }
.key-item--active .key-seg { color: oklch(var(--p) / 0.6); }
.key-item--active .key-seg--leaf { color: oklch(var(--p)); }
.key-sep { color: oklch(var(--bc) / 0.2); font-size: 0.65rem; }

/* ── Skeleton loaders ────────────────────────────────────────────────── */
.skeleton-key {
  height: 30px;
  border-radius: 6px;
  background: oklch(var(--b3) / 0.5);
  margin-bottom: 4px;
  animation: shimmer 1.4s ease-in-out infinite;
}
.skeleton-content { padding: 4px 0; }
.skeleton-line {
  border-radius: 4px;
  background: oklch(var(--b3) / 0.6);
  margin-bottom: 10px;
  animation: shimmer 1.4s ease-in-out infinite;
}
.skeleton-line--h1 { height: 22px; width: 55%; }
.skeleton-line--h2 { height: 16px; width: 40%; }
.skeleton-line--p  { height: 11px; width: 100%; }
@keyframes shimmer {
  0%, 100% { opacity: 0.5; }
  50%       { opacity: 0.85; }
}

/* ── Empty states ────────────────────────────────────────────────────── */
.empty-keys {
  padding: 24px 8px;
  text-align: center;
  font-size: 0.72rem;
  font-family: monospace;
  color: oklch(var(--bc) / 0.25);
}
.empty-keys-icon { font-size: 1.4rem; margin-bottom: 6px; }

/* ── Preview panel ───────────────────────────────────────────────────── */
.preview-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* ── Preview toolbar ─────────────────────────────────────────────────── */
.preview-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 16px;
  height: 36px;
  background: oklch(var(--b2) / 0.6);
  border-bottom: 1px solid oklch(var(--b3));
  flex-shrink: 0;
}
.breadcrumb {
  flex: 1;
  font-size: 0.72rem;
  font-family: monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.breadcrumb-scope { color: oklch(var(--bc) / 0.4); font-size: 0.7rem; }
.breadcrumb-sep   { color: oklch(var(--bc) / 0.2); }
.breadcrumb-node  { color: oklch(var(--bc) / 0.45); }
.breadcrumb-leaf  { color: oklch(var(--p) / 0.9); font-weight: 600; }
.preview-hint { font-size: 0.72rem; font-family: monospace; color: oklch(var(--bc) / 0.2); font-style: italic; }
.copy-btn {
  font-size: 0.68rem;
  font-family: monospace;
  padding: 3px 10px;
  border-radius: 4px;
  border: 1px solid oklch(var(--b3));
  background: oklch(var(--b3) / 0.5);
  color: oklch(var(--bc) / 0.45);
  cursor: pointer;
  transition: all 0.15s;
  flex-shrink: 0;
}
.copy-btn:hover { border-color: oklch(var(--p) / 0.4); color: oklch(var(--p)); }

/* ── Preview body ────────────────────────────────────────────────────── */
.preview-body {
  flex: 1;
  overflow-y: auto;
  padding: 28px 32px;
  position: relative;
}

.error-state {
  padding: 40px 0;
  text-align: center;
  color: oklch(var(--er));
  font-family: monospace;
}

.empty-preview {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  pointer-events: none;
}
.empty-preview-glyph {
  font-size: 2.5rem;
  color: oklch(var(--p) / 0.12);
  line-height: 1;
}
.empty-preview-title {
  font-size: 0.9rem;
  font-family: monospace;
  font-weight: 700;
  color: oklch(var(--bc) / 0.15);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}
.empty-preview-sub {
  font-size: 0.75rem;
  color: oklch(var(--bc) / 0.2);
  text-align: center;
  line-height: 1.6;
}

/* ── Fade-up transition ──────────────────────────────────────────────── */
.fade-up-enter-active { transition: opacity 0.25s ease, transform 0.25s ease; }
.fade-up-enter-from  { opacity: 0; transform: translateY(6px); }

/* ── Markdown preview (marked GFM output) ────────────────────────────── */
.md-preview { max-width: 760px; }

.md-preview :deep(h1),
.md-preview :deep(h2),
.md-preview :deep(h3),
.md-preview :deep(h4) {
  font-family: monospace;
  font-weight: 700;
  letter-spacing: -0.01em;
  margin: 1.4em 0 0.4em;
  padding-bottom: 0.25em;
  border-bottom: 1px solid oklch(var(--b3));
}
.md-preview :deep(h1) { font-size: 1.5rem;  color: oklch(var(--p)); }
.md-preview :deep(h2) { font-size: 1.2rem;  color: oklch(var(--p) / 0.85); }
.md-preview :deep(h3) { font-size: 1rem;    color: oklch(var(--s)); border-bottom-color: oklch(var(--b3) / 0.5); }
.md-preview :deep(h4) { font-size: 0.88rem; color: oklch(var(--bc) / 0.7); border-bottom: none; font-style: italic; }

.md-preview :deep(p) {
  font-size: 0.84rem;
  line-height: 1.75;
  color: oklch(var(--bc) / 0.82);
  margin: 0.5em 0;
}

.md-preview :deep(hr) {
  border: none;
  border-top: 1px solid oklch(var(--b3));
  margin: 1.2em 0;
}

.md-preview :deep(code) {
  font-family: monospace;
  font-size: 0.79rem;
  background: oklch(var(--b3) / 0.8);
  color: oklch(var(--s));
  padding: 0.1em 0.4em;
  border-radius: 4px;
  border: 1px solid oklch(var(--b3));
}

.md-preview :deep(pre) {
  border-radius: 8px;
  border: 1px solid oklch(var(--b3));
  overflow: hidden;
  margin: 0.9em 0;
  background: oklch(var(--b3) / 0.4);
  position: relative;
}
.md-preview :deep(pre code) {
  display: block;
  font-family: monospace;
  font-size: 0.79rem;
  line-height: 1.6;
  color: oklch(var(--bc) / 0.85);
  background: none;
  border: none;
  padding: 14px 16px;
  overflow-x: auto;
}

.md-preview :deep(ul) {
  margin: 0.4em 0 0.4em 1.4em;
  list-style: disc;
}
.md-preview :deep(ol) {
  margin: 0.4em 0 0.4em 1.4em;
  list-style: decimal;
}
.md-preview :deep(li) {
  font-size: 0.84rem;
  line-height: 1.65;
  color: oklch(var(--bc) / 0.8);
  padding: 1px 0;
}

.md-preview :deep(blockquote) {
  border-left: 3px solid oklch(var(--p) / 0.4);
  margin: 0.8em 0;
  padding: 4px 14px;
  color: oklch(var(--bc) / 0.55);
  font-style: italic;
}

.md-preview :deep(a) {
  color: oklch(var(--p));
  text-decoration: underline;
  text-underline-offset: 2px;
}
.md-preview :deep(a:hover) { color: oklch(var(--s)); }

.md-preview :deep(table) {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.79rem;
  font-family: monospace;
  margin: 0.8em 0;
  border-radius: 6px;
  overflow: hidden;
  border: 1px solid oklch(var(--b3));
}
.md-preview :deep(th) {
  background: oklch(var(--b3) / 0.7);
  color: oklch(var(--bc) / 0.55);
  font-size: 0.68rem;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  padding: 6px 12px;
  text-align: left;
  border-bottom: 1px solid oklch(var(--b3));
}
.md-preview :deep(td) {
  padding: 6px 12px;
  border-bottom: 1px solid oklch(var(--b3) / 0.5);
  color: oklch(var(--bc) / 0.8);
}
.md-preview :deep(tr:last-child td) { border-bottom: none; }
.md-preview :deep(tr:hover td)      { background: oklch(var(--b3) / 0.3); }

.md-preview :deep(strong) { color: oklch(var(--bc)); font-weight: 700; }
.md-preview :deep(em)     { color: oklch(var(--bc) / 0.75); font-style: italic; }
.md-preview :deep(del)    { color: oklch(var(--bc) / 0.35); text-decoration: line-through; }

/* ── Mermaid diagram blocks ──────────────────────────────────────────── */
.md-preview :deep(.mermaid-block) {
  margin: 1em 0;
  padding: 16px;
  border-radius: 8px;
  border: 1px solid oklch(var(--b3));
  background: oklch(var(--b3) / 0.25);
  display: flex;
  justify-content: center;
  overflow-x: auto;
}
.md-preview :deep(.mermaid-block svg) {
  max-width: 100%;
  height: auto;
}
</style>

