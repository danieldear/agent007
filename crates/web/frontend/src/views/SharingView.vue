<script setup>
import { ref, computed, watch, onMounted } from "vue";
import { useApi } from "../composables/useApi.js";

const { api } = useApi();

// ─── data ────────────────────────────────────────────────────────────────────
const skills = ref([]);
const workflows = ref([]);
const personas = ref([]);
const registryTools = ref([]); // tools from the tool registry (ToolsView)
const registryScripts = ref([]); // script files from .agent007/scripts/

// ─── export state ────────────────────────────────────────────────────────────
const selectedSkills = ref([]);
const selectedWorkflows = ref([]);
const selectedPersonas = ref([]);
const selectedTools = ref([]);
const exportStatus = ref(null);
const exporting = ref(false);

const allSkillsSelected = computed(
    () =>
        skills.value.length > 0 &&
        selectedSkills.value.length === skills.value.length,
);
const allWorkflowsSelected = computed(
    () =>
        workflows.value.length > 0 &&
        selectedWorkflows.value.length === workflows.value.length,
);
const allPersonasSelected = computed(
    () =>
        personas.value.length > 0 &&
        selectedPersonas.value.length === personas.value.length,
);

// All unique tools/scripts discovered across all skills and workflows.
// API paths: tools/ml/infer.py → bundle key: ml/infer.py
//            scripts/train.py  → bundle key: scripts/train.py
function apiPathToBundleKey(path) {
    return path.startsWith("tools/") ? path.slice("tools/".length) : path;
}

const allTools = computed(() => {
    const seen = new Map();

    // 1. Registry tools — actual packages/files in .agent007/tools/ (exist on disk).
    for (const t of registryTools.value) {
        const key = t.name.toLowerCase();
        if (!seen.has(key))
            seen.set(key, {
                key,
                display: `${t.name} · ${t.scope}`,
                type: "tool",
                realFile: true,
                source: t.source,
            });
    }

    // 2. Registry scripts — actual .py/.sh/etc files in .agent007/scripts/ (exist on disk).
    for (const s of registryScripts.value) {
        const key = s.bundle_key; // already "scripts/rel/path.py"
        if (!seen.has(key))
            seen.set(key, {
                key,
                display: `${s.rel_path} · ${s.scope}`,
                type: "script",
                realFile: true,
                source: s.scope,
            });
    }

    // 3. Tools/scripts mentioned in skill/workflow text (may or may not exist on disk).
    //    These are lower priority — only added if not already covered by real files above.
    for (const s of skills.value) {
        for (const t of s.associations?.tools || []) {
            const key = apiPathToBundleKey(t);
            if (!seen.has(key))
                seen.set(key, {
                    key,
                    display: t,
                    type: "tool",
                    realFile: false,
                    source: s.source,
                });
        }
        for (const sc of s.associations?.scripts || []) {
            const key = apiPathToBundleKey(sc);
            if (!seen.has(key))
                seen.set(key, {
                    key,
                    display: sc,
                    type: "script",
                    realFile: false,
                    source: s.source,
                });
        }
    }
    for (const w of workflows.value) {
        for (const t of w.associations?.tools || []) {
            const key = apiPathToBundleKey(t);
            if (!seen.has(key))
                seen.set(key, {
                    key,
                    display: t,
                    type: "tool",
                    realFile: false,
                    source: w.source,
                });
        }
        for (const sc of w.associations?.scripts || []) {
            const key = apiPathToBundleKey(sc);
            if (!seen.has(key))
                seen.set(key, {
                    key,
                    display: sc,
                    type: "script",
                    realFile: false,
                    source: w.source,
                });
        }
    }
    // Sort: real files first, then associations
    return [...seen.values()].sort((a, b) => {
        if (a.realFile !== b.realFile) return a.realFile ? -1 : 1;
        return a.key.localeCompare(b.key);
    });
});

const allToolsSelected = computed(() => {
    const keys = allTools.value.map((t) => t.key);
    return (
        keys.length > 0 && keys.every((k) => selectedTools.value.includes(k))
    );
},
);

function toggleAllSkills() {
    selectedSkills.value = allSkillsSelected.value
        ? []
        : skills.value.map((s) => s.trigger.replace(/^\//, ""));
}
function toggleAllWorkflows() {
    selectedWorkflows.value = allWorkflowsSelected.value
        ? []
        : workflows.value.map((w) => w.name);
}
function toggleAllPersonas() {
    selectedPersonas.value = allPersonasSelected.value
        ? []
        : personas.value.map((p) => p.name);
}
function toggleAllTools() {
    selectedTools.value = allToolsSelected.value
        ? []
        : allTools.value.map((t) => t.key);
}

// ─── auto-select tools from selected skills/workflows ───────────────────────
watch(
    [selectedSkills, selectedWorkflows, skills, workflows],
    () => {
        const auto = new Set();
        for (const s of skills.value) {
            const key = s.trigger.replace(/^\//, "");
            if (!selectedSkills.value.includes(key)) continue;
            for (const t of s.associations?.tools || [])
                auto.add(apiPathToBundleKey(t));
            for (const sc of s.associations?.scripts || [])
                auto.add(apiPathToBundleKey(sc));
        }
        for (const w of workflows.value) {
            if (!selectedWorkflows.value.includes(w.name)) continue;
            for (const t of w.associations?.tools || [])
                auto.add(apiPathToBundleKey(t));
            for (const sc of w.associations?.scripts || [])
                auto.add(apiPathToBundleKey(sc));
        }
        // Preserve manual tool selections. Auto-detected dependencies are additive.
        selectedTools.value = [...new Set([...selectedTools.value, ...auto])];
    },
    { deep: true },
);

// ─── cascade: selecting a workflow auto-selects its skill and persona deps ───
watch(
    selectedWorkflows,
    (newVal, oldVal) => {
        const added = newVal.filter((n) => !(oldVal || []).includes(n));
        for (const name of added) {
            const wf = workflows.value.find((w) => w.name === name);
            if (!wf) continue;
            for (const skillRef of wf.skill_refs || []) {
                const normalized = skillRef.replace(/^\//, "");
                if (!selectedSkills.value.includes(normalized))
                    selectedSkills.value.push(normalized);
            }
            for (const agentRef of wf.agent_refs || []) {
                const lower = agentRef.toLowerCase();
                const persona = personas.value.find(
                    (p) => p.name.toLowerCase() === lower,
                );
                if (persona && !selectedPersonas.value.includes(persona.name)) {
                    selectedPersonas.value.push(persona.name);
                }
            }
        }
    },
    { deep: true },
);

// ─── import state ────────────────────────────────────────────────────────────
const fileInput = ref(null);
const bundleData = ref(null);
const bundlePreview = ref(null);
const parseError = ref(null);
const overwrite = ref(false);
const importStatus = ref(null);
const importing = ref(false);
const isImportDragActive = ref(false);

// ─── load data ────────────────────────────────────────────────────────────────
onMounted(async () => {
    const [sk, wf, ps, tl, sl] = await Promise.all([
        api.listSkills(),
        api.listWorkflows(),
        api.listPersonas(),
        api.listTools().catch(() => []),
        api.listScripts().catch(() => []),
    ]);
    if (sk) {
        skills.value = sk;
        selectedSkills.value = sk.map((s) => s.trigger.replace(/^\//, ""));
    }
    if (wf) {
        workflows.value = wf;
        selectedWorkflows.value = wf.map((w) => w.name);
    }
    if (ps) {
        personas.value = ps;
        selectedPersonas.value = ps.map((p) => p.name);
    }
    // Auto-select all real tools and scripts that actually exist on disk.
    const autoToolKeys = [];
    if (Array.isArray(tl) && tl.length) {
        registryTools.value = tl;
        autoToolKeys.push(...tl.map((t) => t.name.toLowerCase()));
    }
    if (Array.isArray(sl) && sl.length) {
        registryScripts.value = sl;
        autoToolKeys.push(...sl.map((s) => s.bundle_key));
    }
    if (autoToolKeys.length) {
        selectedTools.value = [...new Set([...selectedTools.value, ...autoToolKeys])];
    }
});

function assocTools(item) {
    return item?.associations?.tools || [];
}
function assocScripts(item) {
    return item?.associations?.scripts || [];
}
function previewAssociations(values, limit = 2) {
    return values.slice(0, limit);
}
function compactRef(value) {
    if (!value) return "";
    const normalized = String(value).replace(/^\.?\/*/, "");
    const parts = normalized.split("/").filter(Boolean);
    if (parts.length <= 2) return normalized;
    return `${parts[0]}/…/${parts[parts.length - 1]}`;
}

const totalSelected = computed(
    () =>
        selectedSkills.value.length +
        selectedWorkflows.value.length +
        selectedPersonas.value.length +
        selectedTools.value.length,
);

// ─── export ──────────────────────────────────────────────────────────────────
async function exportBundle() {
    exporting.value = true;
    exportStatus.value = null;
    try {
        const res = await api.exportBundle(
            selectedSkills.value,
            selectedWorkflows.value,
            selectedPersonas.value,
            selectedTools.value,
        );
        if (!res.ok) {
            const body = await res.json().catch(() => ({}));
            exportStatus.value = {
                type: "error",
                message: body?.error || `Export failed (${res.status})`,
            };
            return;
        }
        const blob = await res.blob();
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "agent007-bundle.a7bundle";
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
        exportStatus.value = { type: "success", message: "Bundle downloaded!" };
        setTimeout(() => {
            exportStatus.value = null;
        }, 4000);
    } catch (e) {
        exportStatus.value = {
            type: "error",
            message: e.message || "Export failed",
        };
    } finally {
        exporting.value = false;
    }
}

// ─── import ──────────────────────────────────────────────────────────────────
function openFilePicker() {
    fileInput.value?.click();
}

function resetImportParseState() {
    bundleData.value = null;
    bundlePreview.value = null;
    parseError.value = null;
    importStatus.value = null;
}

async function parseBundleFromFile(file) {
    resetImportParseState();
    if (!file) return;

    try {
        const text = await file.text();
        const parsed = JSON.parse(text);

        const skillCount = Array.isArray(parsed.skills)
            ? parsed.skills.length
            : null;
        const workflowCount = Array.isArray(parsed.workflows)
            ? parsed.workflows.length
            : null;
        const toolsCount = Array.isArray(parsed.tools)
            ? parsed.tools.length
            : null;
        const personasCount = Array.isArray(parsed.personas)
            ? parsed.personas.length
            : null;

        if (
            skillCount === null &&
            workflowCount === null &&
            toolsCount === null &&
            personasCount === null
        ) {
            parseError.value =
                "File does not look like an agent007 bundle (missing skills/workflows/tools/personas arrays)";
            return;
        }

        bundleData.value = parsed;
        bundlePreview.value = {
            skillCount: skillCount ?? 0,
            workflowCount: workflowCount ?? 0,
            toolsCount: toolsCount ?? 0,
            personasCount: personasCount ?? 0,
        };
    } catch (err) {
        parseError.value = `Could not parse bundle: ${err.message}`;
    }
}

async function handleFileChange(e) {
    const file = e.target.files?.[0];
    if (!file) return;
    e.target.value = "";
    await parseBundleFromFile(file);
}

function handleImportDragEnter(e) {
    e.preventDefault();
    isImportDragActive.value = true;
}

function handleImportDragOver(e) {
    e.preventDefault();
    isImportDragActive.value = true;
}

function handleImportDragLeave(e) {
    e.preventDefault();
    if (
        e.currentTarget &&
        e.relatedTarget &&
        e.currentTarget.contains(e.relatedTarget)
    ) {
        return;
    }
    isImportDragActive.value = false;
}

async function handleImportDrop(e) {
    e.preventDefault();
    isImportDragActive.value = false;
    const file = e.dataTransfer?.files?.[0];
    if (!file) return;
    await parseBundleFromFile(file);
}

async function importBundle() {
    if (!bundleData.value) return;
    importing.value = true;
    importStatus.value = null;
    try {
        const result = await api.importBundle(
            bundleData.value,
            overwrite.value,
        );
        if (result) {
            const parts = [];
            if (result.imported != null)
                parts.push(`Imported: ${result.imported}`);
            if (result.skipped != null)
                parts.push(`Skipped: ${result.skipped}`);
            if (result.overwritten != null)
                parts.push(`Overwritten: ${result.overwritten}`);
            importStatus.value = {
                type: "success",
                message: parts.length ? parts.join(" · ") : "Import complete",
            };
        } else {
            importStatus.value = {
                type: "success",
                message: "Import complete",
            };
        }
        bundleData.value = null;
        bundlePreview.value = null;
    } catch (e) {
        importStatus.value = {
            type: "error",
            message: e.message || "Import failed",
        };
    } finally {
        importing.value = false;
    }
}
</script>

<template>
    <div class="flex flex-col h-full">
        <!-- Header -->
        <div
            class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0"
        >
            <div>
                <span
                    class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40"
                    >Sharing</span
                >
                <p class="text-[11px] font-mono text-base-content/30 mt-0.5">
                    export &amp; import .a7bundle files
                </p>
            </div>
        </div>

        <div class="flex-1 overflow-auto">
            <div class="p-5 lg:p-6 flex flex-col gap-4">
                <div class="grid grid-cols-1 gap-4 order-1">
                    <div
                        class="card bg-base-200 border border-base-300 shadow-sm order-1"
                    >
                        <div class="card-body p-0">
                            <div
                                class="flex items-center gap-2.5 px-5 pt-4 pb-3"
                            >
                                <div
                                    class="w-7 h-7 rounded-lg bg-primary/12 flex items-center justify-center shrink-0"
                                >
                                    <span
                                        class="text-primary text-sm leading-none"
                                        >↓</span
                                    >
                                </div>
                                <div>
                                    <h2
                                        class="font-bold font-mono text-sm text-base-content/80"
                                    >
                                        Export Bundle
                                    </h2>
                                    <p
                                        class="text-[10px] font-mono text-base-content/35 uppercase tracking-wider"
                                    >
                                        pack &amp; download
                                    </p>
                                </div>
                            </div>

                            <div class="px-5 pb-3 text-xs text-base-content/55">
                                Select skills, workflows, personas, and tools to
                                bundle into a
                                <code
                                    class="font-mono bg-base-300/70 px-1 rounded"
                                    >.a7bundle</code
                                >
                                file. Selecting a workflow auto-selects its
                                dependent assets.
                            </div>

                            <div
                                class="border-t border-base-300 p-3 grid grid-cols-1 md:grid-cols-2 xl:grid-cols-4 gap-2.5"
                            >
                                <div
                                    class="rounded-xl border border-secondary/30 bg-base-300/20"
                                >
                                    <div
                                        class="flex items-center justify-between px-3 py-2 border-b border-base-300/60"
                                    >
                                        <span
                                            class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/45"
                                            >⬡ Workflows</span
                                        >
                                        <label
                                            class="flex items-center gap-1.5 cursor-pointer"
                                        >
                                            <span
                                                class="text-[10px] text-base-content/30 font-mono"
                                                >all</span
                                            >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary"
                                                :checked="allWorkflowsSelected"
                                                @change="toggleAllWorkflows"
                                            />
                                        </label>
                                    </div>
                                    <div
                                        class="overflow-y-auto"
                                        style="max-height: 20rem"
                                    >
                                        <div
                                            v-if="!workflows.length"
                                            class="text-[11px] text-base-content/30 italic px-3 py-2.5"
                                        >
                                            No workflows saved
                                        </div>
                                        <label
                                            v-for="w in workflows"
                                            :key="w.name"
                                            class="flex items-start gap-2 cursor-pointer hover:bg-base-300/50 px-3 py-2 border-b border-base-300/30 last:border-0"
                                        >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary shrink-0 mt-1"
                                                :value="w.name"
                                                v-model="selectedWorkflows"
                                            />
                                            <div class="flex-1 min-w-0">
                                                <div
                                                    class="text-[12px] font-mono truncate"
                                                >
                                                    {{ w.name }}
                                                </div>
                                                <div
                                                    v-if="w.description"
                                                    class="text-[10px] text-base-content/45 truncate mt-0.5"
                                                >
                                                    {{ w.description }}
                                                </div>
                                                <div
                                                    v-if="
                                                        w.skill_refs?.length ||
                                                        w.agent_refs?.length ||
                                                        assocTools(w).length
                                                    "
                                                    class="flex flex-wrap gap-1 mt-1"
                                                >
                                                    <span
                                                        v-for="sk in previewAssociations(
                                                            w.skill_refs || [],
                                                            2,
                                                        )"
                                                        :key="`wsk-${w.name}-${sk}`"
                                                        class="inline-flex rounded border border-primary/30 px-1 py-0 text-[9px] font-mono text-primary/70"
                                                        >{{ sk }}</span
                                                    >
                                                    <span
                                                        v-for="ag in previewAssociations(
                                                            w.agent_refs || [],
                                                            2,
                                                        )"
                                                        :key="`wag-${w.name}-${ag}`"
                                                        class="inline-flex rounded border border-accent/30 px-1 py-0 text-[9px] font-mono text-accent/75"
                                                        >{{ ag }}</span
                                                    >
                                                    <span
                                                        v-for="t in previewAssociations(
                                                            assocTools(w),
                                                            2,
                                                        )"
                                                        :key="`wt-${w.name}-${t}`"
                                                        class="inline-flex rounded border border-info/30 px-1 py-0 text-[9px] font-mono text-info/75"
                                                        >{{
                                                            compactRef(t)
                                                        }}</span
                                                    >
                                                </div>
                                            </div>
                                            <span
                                                v-if="w.steps"
                                                class="text-[9px] font-mono text-base-content/30 shrink-0 mt-1"
                                                >{{ w.steps }}s</span
                                            >
                                        </label>
                                    </div>
                                </div>

                                <div
                                    class="rounded-xl border border-primary/30 bg-base-300/20"
                                >
                                    <div
                                        class="flex items-center justify-between px-3 py-2 border-b border-base-300/60"
                                    >
                                        <span
                                            class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/45"
                                            >⚡ Skills</span
                                        >
                                        <label
                                            class="flex items-center gap-1.5 cursor-pointer"
                                        >
                                            <span
                                                class="text-[10px] text-base-content/30 font-mono"
                                                >all</span
                                            >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary"
                                                :checked="allSkillsSelected"
                                                @change="toggleAllSkills"
                                            />
                                        </label>
                                    </div>
                                    <div
                                        class="overflow-y-auto"
                                        style="max-height: 20rem"
                                    >
                                        <div
                                            v-if="!skills.length"
                                            class="text-[11px] text-base-content/30 italic px-3 py-2.5"
                                        >
                                            No skills installed
                                        </div>
                                        <label
                                            v-for="s in skills"
                                            :key="s.trigger"
                                            class="flex items-start gap-2 cursor-pointer hover:bg-base-300/50 px-3 py-2 border-b border-base-300/30 last:border-0"
                                        >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary shrink-0 mt-1"
                                                :value="
                                                    s.trigger.replace(/^\//, '')
                                                "
                                                v-model="selectedSkills"
                                            />
                                            <div class="flex-1 min-w-0">
                                                <div
                                                    class="text-[12px] font-mono truncate"
                                                >
                                                    {{ s.trigger }}
                                                </div>
                                                <div
                                                    v-if="s.description"
                                                    class="text-[10px] text-base-content/45 truncate mt-0.5"
                                                >
                                                    {{ s.description }}
                                                </div>
                                            </div>
                                        </label>
                                    </div>
                                </div>

                                <div
                                    class="rounded-xl border border-accent/30 bg-base-300/20"
                                >
                                    <div
                                        class="flex items-center justify-between px-3 py-2 border-b border-base-300/60"
                                    >
                                        <span
                                            class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/45"
                                            >👤 Personas</span
                                        >
                                        <label
                                            class="flex items-center gap-1.5 cursor-pointer"
                                        >
                                            <span
                                                class="text-[10px] text-base-content/30 font-mono"
                                                >all</span
                                            >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary"
                                                :checked="allPersonasSelected"
                                                @change="toggleAllPersonas"
                                            />
                                        </label>
                                    </div>
                                    <div
                                        class="overflow-y-auto"
                                        style="max-height: 20rem"
                                    >
                                        <div
                                            v-if="!personas.length"
                                            class="text-[11px] text-base-content/30 italic px-3 py-2.5"
                                        >
                                            No personas installed
                                        </div>
                                        <label
                                            v-for="p in personas"
                                            :key="p.name"
                                            class="flex items-start gap-2 cursor-pointer hover:bg-base-300/50 px-3 py-2 border-b border-base-300/30 last:border-0"
                                        >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary shrink-0 mt-1"
                                                :value="p.name"
                                                v-model="selectedPersonas"
                                            />
                                            <div class="flex-1 min-w-0">
                                                <div
                                                    class="text-[12px] font-mono truncate"
                                                >
                                                    {{ p.name }}
                                                </div>
                                                <div
                                                    v-if="p.description"
                                                    class="text-[10px] text-base-content/45 truncate mt-0.5"
                                                >
                                                    {{ p.description }}
                                                </div>
                                            </div>
                                        </label>
                                    </div>
                                </div>

                                <div
                                    class="rounded-xl border border-info/30 bg-base-300/20"
                                >
                                    <div
                                        class="flex items-center justify-between px-3 py-2 border-b border-base-300/60"
                                    >
                                        <span
                                            class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/45"
                                            >🛠 Tools &amp; Scripts</span
                                        >
                                        <label
                                            class="flex items-center gap-1.5 cursor-pointer"
                                        >
                                            <span
                                                class="text-[10px] text-base-content/30 font-mono"
                                                >all</span
                                            >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary"
                                                :checked="allToolsSelected"
                                                @change="toggleAllTools"
                                            />
                                        </label>
                                    </div>
                                    <div
                                        class="overflow-y-auto"
                                        style="max-height: 20rem"
                                    >
                                        <div
                                            v-if="!allTools.length"
                                            class="text-[11px] text-base-content/30 italic px-3 py-2.5"
                                        >
                                            No tools found — import tools in the Tools tab
                                        </div>
                                        <label
                                            v-for="t in allTools"
                                            :key="t.key"
                                            class="flex items-start gap-2 cursor-pointer hover:bg-base-300/50 px-3 py-2 border-b border-base-300/30 last:border-0"
                                        >
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-xs checkbox-primary shrink-0 mt-1"
                                                :value="t.key"
                                                v-model="selectedTools"
                                            />
                                            <div class="flex-1 min-w-0">
                                                <div class="flex items-center gap-1.5 min-w-0">
                                                    <span
                                                        class="text-[12px] font-mono truncate"
                                                        :title="t.key"
                                                    >{{ compactRef(t.key) }}</span>
                                                    <span
                                                        v-if="t.type === 'script'"
                                                        class="badge badge-xs badge-warning shrink-0"
                                                    >script</span>
                                                    <span
                                                        v-else-if="t.type === 'tool'"
                                                        class="badge badge-xs badge-info shrink-0"
                                                    >tool</span>
                                                    <span
                                                        v-if="!t.realFile"
                                                        class="badge badge-xs badge-ghost shrink-0 opacity-50"
                                                        title="Reference only — file not found on disk"
                                                    >ref</span>
                                                </div>
                                                <div
                                                    class="text-[10px] mt-0.5 truncate"
                                                    :class="t.realFile ? 'text-base-content/45' : 'text-warning/60'"
                                                >
                                                    {{ t.realFile ? t.display : '⚠ file not found on disk' }}
                                                </div>
                                            </div>
                                        </label>
                                    </div>
                                </div>
                            </div>

                            <div
                                class="px-5 py-3 border-t border-base-300 flex items-center justify-between gap-3 flex-wrap"
                            >
                                <div class="flex items-center gap-2 flex-wrap">
                                    <span
                                        v-if="selectedSkills.length"
                                        class="badge badge-primary badge-sm font-mono"
                                        >{{ selectedSkills.length }} skill{{
                                            selectedSkills.length !== 1
                                                ? "s"
                                                : ""
                                        }}</span
                                    >
                                    <span
                                        v-if="selectedWorkflows.length"
                                        class="badge badge-secondary badge-sm font-mono"
                                        >{{
                                            selectedWorkflows.length
                                        }}
                                        workflow{{
                                            selectedWorkflows.length !== 1
                                                ? "s"
                                                : ""
                                        }}</span
                                    >
                                    <span
                                        v-if="selectedPersonas.length"
                                        class="badge badge-accent badge-sm font-mono"
                                        >{{ selectedPersonas.length }} persona{{
                                            selectedPersonas.length !== 1
                                                ? "s"
                                                : ""
                                        }}</span
                                    >
                                    <span
                                        v-if="selectedTools.length"
                                        class="badge badge-info badge-sm font-mono"
                                        >{{ selectedTools.length }} tool{{
                                            selectedTools.length !== 1
                                                ? "s"
                                                : ""
                                        }}</span
                                    >
                                    <span
                                        v-if="
                                            !selectedSkills.length &&
                                            !selectedWorkflows.length &&
                                            !selectedPersonas.length &&
                                            !selectedTools.length
                                        "
                                        class="text-[11px] font-mono text-base-content/30"
                                        >nothing selected</span
                                    >
                                </div>
                                <button
                                    class="btn btn-sm btn-primary font-mono"
                                    :disabled="exporting || totalSelected === 0"
                                    @click="exportBundle"
                                >
                                    <span
                                        v-if="exporting"
                                        class="loading loading-spinner loading-xs"
                                    ></span>
                                    <span v-else>↓</span>
                                    {{
                                        exporting
                                            ? "Exporting…"
                                            : "Export .a7bundle"
                                    }}
                                </button>
                            </div>
                        </div>
                    </div>

                    <div
                        class="card bg-base-200 border border-base-300 shadow-sm order-1"
                    >
                        <div class="card-body p-0">
                            <div class="px-5 pt-4 pb-3">
                                <div class="flex items-center gap-2.5">
                                    <div
                                        class="w-7 h-7 rounded-lg bg-secondary/12 flex items-center justify-center shrink-0"
                                    >
                                        <span
                                            class="text-secondary text-sm leading-none"
                                            >↑</span
                                        >
                                    </div>
                                    <div>
                                        <h2
                                            class="font-bold font-mono text-sm text-base-content/80"
                                        >
                                            Import Bundle
                                        </h2>
                                        <p
                                            class="text-[10px] font-mono text-base-content/35 uppercase tracking-wider"
                                        >
                                            select &amp; import
                                        </p>
                                    </div>
                                </div>
                                <p class="text-xs text-base-content/50 mt-3">
                                    Import a
                                    <code
                                        class="font-mono bg-base-300/80 px-1 rounded"
                                        >.a7bundle</code
                                    >
                                    file to add skills, workflows, tools, and
                                    personas to this instance.
                                </p>
                            </div>

                            <div class="px-5 pb-4">
                                <input
                                    ref="fileInput"
                                    type="file"
                                    accept=".a7bundle,.json"
                                    class="hidden"
                                    @change="handleFileChange"
                                />
                                <div
                                    class="rounded-xl border border-dashed p-8 text-center cursor-pointer transition-colors"
                                    :class="
                                        isImportDragActive
                                            ? 'border-primary/60 bg-primary/10'
                                            : 'border-base-300 hover:border-primary/45 hover:bg-base-300/30'
                                    "
                                    @click="openFilePicker"
                                    @dragenter="handleImportDragEnter"
                                    @dragover="handleImportDragOver"
                                    @dragleave="handleImportDragLeave"
                                    @drop="handleImportDrop"
                                >
                                    <div class="text-xl mb-2 opacity-65">
                                        📦
                                    </div>
                                    <div
                                        class="text-lg font-mono text-base-content/70"
                                    >
                                        Click to select a bundle
                                    </div>
                                    <div
                                        class="text-md font-mono text-base-content/35 mt-1"
                                    >
                                        .a7bundle · .json
                                    </div>
                                </div>

                                <div
                                    v-if="bundlePreview"
                                    class="mt-3 grid grid-cols-4 gap-2 text-center"
                                >
                                    <div>
                                        <div
                                            class="text-sm font-mono text-primary"
                                        >
                                            {{ bundlePreview.skillCount }}
                                        </div>
                                        <div
                                            class="text-[10px] text-base-content/45"
                                        >
                                            skills
                                        </div>
                                    </div>
                                    <div>
                                        <div
                                            class="text-sm font-mono text-secondary"
                                        >
                                            {{ bundlePreview.workflowCount }}
                                        </div>
                                        <div
                                            class="text-[10px] text-base-content/45"
                                        >
                                            flows
                                        </div>
                                    </div>
                                    <div>
                                        <div
                                            class="text-sm font-mono text-info"
                                        >
                                            {{ bundlePreview.toolsCount }}
                                        </div>
                                        <div
                                            class="text-[10px] text-base-content/45"
                                        >
                                            tools
                                        </div>
                                    </div>
                                    <div>
                                        <div
                                            class="text-sm font-mono text-accent"
                                        >
                                            {{ bundlePreview.personasCount }}
                                        </div>
                                        <div
                                            class="text-[10px] text-base-content/45"
                                        >
                                            personas
                                        </div>
                                    </div>
                                </div>

                                <div
                                    v-if="bundlePreview"
                                    class="mt-3 flex items-center justify-end gap-2"
                                >
                                    <label
                                        class="flex items-center gap-2 cursor-pointer mr-auto"
                                    >
                                        <input
                                            type="checkbox"
                                            class="toggle toggle-xs toggle-warning"
                                            v-model="overwrite"
                                        />
                                        <span
                                            class="text-xs font-mono text-base-content/50"
                                            >overwrite existing</span
                                        >
                                    </label>
                                    <button
                                        class="btn btn-sm btn-primary font-mono"
                                        :disabled="importing"
                                        @click="importBundle"
                                    >
                                        <span
                                            v-if="importing"
                                            class="loading loading-spinner loading-xs"
                                        ></span>
                                        {{
                                            importing ? "Importing…" : "Import"
                                        }}
                                    </button>
                                </div>

                                <div
                                    v-if="parseError"
                                    class="alert alert-error mt-3 py-2"
                                >
                                    <span class="text-xs font-mono">{{
                                        parseError
                                    }}</span>
                                </div>
                                <div
                                    v-if="importStatus"
                                    class="alert mt-3 py-2"
                                    :class="
                                        importStatus.type === 'success'
                                            ? 'alert-success'
                                            : 'alert-error'
                                    "
                                >
                                    <span class="text-xs font-mono">{{
                                        importStatus.message
                                    }}</span>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                <!-- ── Format Reference ────────────────────────────────────────────── -->
                <div
                    class="card bg-base-200 border border-base-300 order-2"
                >
                    <div class="card-body p-5">
                        <h3
                            class="font-bold font-mono text-[10px] uppercase tracking-widest text-base-content/40 mb-4"
                        >
                            Bundle Format Reference
                        </h3>
                        <div class="grid grid-cols-1 md:grid-cols-5 gap-5">
                            <div class="flex items-start gap-3">
                                <span class="text-primary opacity-60 mt-0.5"
                                    >⚡</span
                                >
                                <div>
                                    <p
                                        class="text-xs font-mono font-semibold text-base-content/70 mb-1"
                                    >
                                        Skills
                                    </p>
                                    <p
                                        class="text-xs text-base-content/45 leading-relaxed"
                                    >
                                        Markdown files with YAML frontmatter —
                                        trigger, name, description, model, and a
                                        prompt template using
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >&#123;&#123;args&#125;&#125;</code
                                        >.
                                    </p>
                                </div>
                            </div>
                            <div class="flex items-start gap-3">
                                <span class="text-secondary opacity-60 mt-0.5"
                                    >⬡</span
                                >
                                <div>
                                    <p
                                        class="text-xs font-mono font-semibold text-base-content/70 mb-1"
                                    >
                                        Workflows
                                    </p>
                                    <p
                                        class="text-xs text-base-content/45 leading-relaxed"
                                    >
                                        YAML pipelines with named persona steps,
                                        optional
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >depends_on</code
                                        >, and special node types (evaluator,
                                        router, approval, orchestrator).
                                    </p>
                                </div>
                            </div>
                            <div class="flex items-start gap-3">
                                <span class="text-info opacity-60 mt-0.5"
                                    >🛠</span
                                >
                                <div>
                                    <p
                                        class="text-xs font-mono font-semibold text-base-content/70 mb-1"
                                    >
                                        Tools / Scripts
                                    </p>
                                    <p
                                        class="text-xs text-base-content/45 leading-relaxed"
                                    >
                                        Executable assets in
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >tools/</code
                                        >
                                        and
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >scripts/</code
                                        >
                                        that skills depend on — auto-selected
                                        when their parent skill is selected.
                                    </p>
                                </div>
                            </div>
                            <div class="flex items-start gap-3">
                                <span class="text-accent opacity-60 mt-0.5"
                                    >👤</span
                                >
                                <div>
                                    <p
                                        class="text-xs font-mono font-semibold text-base-content/70 mb-1"
                                    >
                                        Personas
                                    </p>
                                    <p
                                        class="text-xs text-base-content/45 leading-relaxed"
                                    >
                                        TOML agent persona files referenced by
                                        workflow
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >agent:</code
                                        >
                                        steps. Auto-selected when a workflow
                                        that uses them is selected.
                                    </p>
                                </div>
                            </div>
                            <div class="flex items-start gap-3">
                                <span
                                    class="text-base-content opacity-40 mt-0.5"
                                    >◈</span
                                >
                                <div>
                                    <p
                                        class="text-xs font-mono font-semibold text-base-content/70 mb-1"
                                    >
                                        Container
                                    </p>
                                    <p
                                        class="text-xs text-base-content/45 leading-relaxed"
                                    >
                                        JSON envelope with
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >version</code
                                        >,
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >created_at</code
                                        >, and arrays of
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >skills</code
                                        >,
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >workflows</code
                                        >,
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >tools</code
                                        >, and
                                        <code class="bg-base-300 px-0.5 rounded"
                                            >personas</code
                                        >
                                        — each with filename, content, sha256.
                                    </p>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>
