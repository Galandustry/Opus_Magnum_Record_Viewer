<script setup lang="ts">
import { ref, watch, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import OmList from "./components/OmList.vue";
import { OmRecordDTO, SyncResult } from "./types/om";

interface UniversalSuggestion {
  id: string;
  displayName: string;
  controller: string;
}

const searchKeyword = ref<string>(""); 
const suggestions = ref<UniversalSuggestion[]>([]); 
const showSuggestions = ref<boolean>(false);

const currentRecords = ref<OmRecordDTO[]>([]);
const loading = ref<boolean>(false);
const errorMessage = ref<string | null>(null);
const hasSearched = ref<boolean>(false);
const cachePath = ref("");
const cacheInfo = ref("");
const forceRefresh = ref(false);
const lastKeyword = ref("");
const syncing = ref(false);
const lastSyncResult = ref<SyncResult | null>(null);
const syncError = ref<string | null>(null);
const timezoneMode = ref<"UTC" | "BEIJING">("UTC");

const formattedCacheInfo = computed(() => {
  if (!cacheInfo.value) return "Last Sync: —";
  // 匹配 ISO (带T带Z) 或已格式化 (空格分隔) 的时间戳
  const m = cacheInfo.value.match(/(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})/);
  if (!m) return cacheInfo.value;
  const d = new Date(m[1] + 'T' + m[2] + 'Z');
  if (isNaN(d.getTime())) return cacheInfo.value;
  const pad = (n: number) => String(n).padStart(2, "0");
  const fmt = (y:number, mo:number, da:number, h:number, mi:number, s:number, tz:string) =>
    `Last Sync: ${y}-${pad(mo)}-${pad(da)} ${pad(h)}:${pad(mi)}:${pad(s)} ${tz}`;
  if (timezoneMode.value === "UTC") {
    return fmt(d.getUTCFullYear(), d.getUTCMonth()+1, d.getUTCDate(), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(), "UTC");
  }
  if (timezoneMode.value === "BEIJING") {
    const bj = new Date(d.getTime() + 8*3600000);
    return fmt(bj.getUTCFullYear(), bj.getUTCMonth()+1, bj.getUTCDate(), bj.getUTCHours(), bj.getUTCMinutes(), bj.getUTCSeconds(), "UTC+8");
  }
  return fmt(d.getUTCFullYear(), d.getUTCMonth()+1, d.getUTCDate(), d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(), "UTC");
});

const updateCacheInfo = async () => {
  try { cacheInfo.value = await invoke<string>("get_cache_info"); } catch { cacheInfo.value = "?"; }
};

const executeRefresh = () => {
  if (lastKeyword.value) {
    executeFinalQuery(lastKeyword.value);
  }
};

const syncNow = async () => {
  syncing.value = true;
  syncError.value = null;
  lastSyncResult.value = null;
  try {
    lastSyncResult.value = await invoke<SyncResult>("sync_incremental", { 
      since: null, 
      controller: "om" 
    });
    updateCacheInfo();
  } catch (err) {
    syncError.value = String(err);
  } finally {
    syncing.value = false;
  }
};

// 获取缓存路径
invoke<string>("get_cache_path").then(p => cachePath.value = p);
updateCacheInfo();
const bootReady = ref<boolean>(false);  // 🚀 新增：启动同步状态

watch(searchKeyword, async (newVal) => {
  const query = newVal.trim();
  if (!query) {
    suggestions.value = [];
    return;
  }
  try {
    suggestions.value = await invoke<UniversalSuggestion[]>("get_live_puzzle_suggestions", { keyword: query });
  } catch (err) {
    console.error(err);
  }
});

const selectSuggestion = (puzzle: UniversalSuggestion) => {
  searchKeyword.value = puzzle.displayName;
  showSuggestions.value = false;
  executeFinalQuery(puzzle.id); 
};

const handleEnterKey = () => {
  showSuggestions.value = false;
  const query = searchKeyword.value.trim();
  if (!query) return;
  // 🚀 如果建议列表中有精确或部分匹配，优先用匹配的 puzzle ID
  const bestMatch = suggestions.value.find(
    s => s.displayName.toLowerCase() === query.toLowerCase()
  ) ?? suggestions.value.find(
    s => s.displayName.toLowerCase().includes(query.toLowerCase())
  );
  executeFinalQuery(bestMatch?.id ?? query);
};

const executeFinalQuery = async (puzzleId: string) => {
  if (!puzzleId) return;
  lastKeyword.value = puzzleId;
  loading.value = true;
  errorMessage.value = null;
  hasSearched.value = true;

  try {
    currentRecords.value = await invoke<OmRecordDTO[]>("search_om_records", { keyword: puzzleId, force: forceRefresh.value });
  } catch (err) {
    errorMessage.value = String(err);
  } finally {
    loading.value = false;
    forceRefresh.value = false;
    updateCacheInfo();
  }
};

// 🚀 轮询检查后端的同步状态
const pollBootReady = async () => {
  while (!bootReady.value) {
    try {
      const ready = await invoke<boolean>("check_boot_ready");
      if (ready) bootReady.value = true;
    } catch { /* ignore polling error */ }
    await new Promise(r => setTimeout(r, 300));
  }
};
pollBootReady();

const closeSuggestions = () => {
  setTimeout(() => { showSuggestions.value = false; }, 200);
};
</script>

<template>
  <div class="app-container">
    <aside class="sidebar-panel">
      <div class="brand-zone">
        <div class="logo-icon">▲</div>
        <div class="brand-text">
          <h2>OPUS MAGNUM</h2>
          <span>LEADERBOARD TERMINAL v2.5</span>
        </div>
      </div>
      <hr class="divider" />
      <div class="control-group">
        <label class="section-label">QUERY INSTANCE (MOCK/LIVE FLUX)</label>
        <div class="search-field-wrapper">
          <div class="input-container">
            <input 
              type="text" 
              v-model="searchKeyword" 
              placeholder="Type puzzle name (e.g. Mist)..."
              @keyup.enter="handleEnterKey"
              @focus="showSuggestions = true"
              @blur="closeSuggestions"
              :disabled="loading"
              class="cyber-input"
            />
            <ul v-if="showSuggestions && suggestions.length > 0" class="suggestions-menu">
              <li v-for="s in suggestions" :key="s.id" @mousedown="selectSuggestion(s)" class="suggestion-item">
                <span class="s-name">{{ s.displayName }}</span>
                <span class="s-id">[{{ s.controller.toUpperCase() }}] #{{ s.id }}</span>
              </li>
            </ul>
          </div>
          <button @click="handleEnterKey" :disabled="loading || !searchKeyword.trim()" class="btn-execute">EXECUTE</button>
          <button @click="forceRefresh = !forceRefresh; if (forceRefresh) executeRefresh()" class="btn-execute" style="margin-top:6px" :class="{ active: forceRefresh }">REFRESH</button>
          <button @click="syncNow" :disabled="syncing" class="btn-execute btn-sync" style="margin-top:6px">{{ syncing ? 'SYNCING…' : 'SYNC' }}</button>
        </div>
      </div>
      <div class="status-zone">
        <label class="section-label">DIAGNOSTICS</label>
        <div class="status-card" :class="{ 'is-loading': loading, 'is-error': errorMessage, 'is-syncing': !bootReady }">
          <div :class="['pulse-light', loading ? 'warning' : !bootReady ? 'syncing' : errorMessage ? 'danger' : 'success']"></div>
          <span class="status-text">{{ !bootReady ? "SYNCING" : loading ? "FETCHING" : errorMessage ? "MISMATCH" : "READY" }}</span>
        </div>
        <p v-if="errorMessage" class="error-details">> {{ errorMessage }}</p>
        <div v-if="lastSyncResult" class="sync-result">
          <p class="sync-summary">SYNC: +{{ lastSyncResult.newCount }} / -{{ lastSyncResult.removedCount }}</p>
          <p v-if="lastSyncResult.errors.length > 0" class="sync-errors">
            <span v-for="(e, i) in lastSyncResult.errors" :key="i">⚠ {{ e }}</span>
          </p>
        </div>
        <p v-if="syncError" class="error-details">> SYNC: {{ syncError }}</p>
        <p class="cache-path">CACHE: {{ cachePath }}</p>
        <div class="timezone-switch">
          <span class="tz-label">TZ</span>
          <button :class="{ active: timezoneMode === 'UTC' }" @click="timezoneMode = 'UTC'">UTC</button>
          <button :class="{ active: timezoneMode === 'BEIJING' }" @click="timezoneMode = 'BEIJING'">UTC+8</button>
        </div>
        <p class="cache-ts">{{ formattedCacheInfo }}</p>
      </div>
    </aside>
    <main class="main-workspace">
      <header class="workspace-header">
        <div class="breadcrumb">CLUSTER / DATA_FLOW / RECORD_GRID</div>
        <div class="timestamp">SYS_STATUS: ONLINE</div>
      </header>
      <section class="data-view-panel">
        <div v-if="!hasSearched" class="dashboard-welcome">
          <div class="welcome-terminal">
            <p class="console-line">> MULTI-GAME DATA ENGINE ACTIVE.</p>
            <p class="console-line">> INPUT FRAGMENT KEYWORD TO POLL INDEX MATRIX...</p>
          </div>
        </div>
        <OmList v-else :records="currentRecords" />
      </section>
    </main>
  </div>
</template>

<style>
:root { --bg-deep: #0a0d14; --bg-panel: #121620; --bg-input: #1a1f2c; --border-color: #262e3f; --color-primary: #00b4d8; --color-accent: #00f5d4; --color-warn: #ffb703; --color-danger: #ff4a4a; --color-text: #e2e8f0; --color-text-muted: #4e5d78; }
body { margin: 0; padding: 0; background-color: var(--bg-deep); color: var(--color-text); font-family: monospace; overflow: hidden; }
.app-container { display: grid; grid-template-columns: 340px 1fr; height: 100vh; width: 100vw; }
.sidebar-panel { background-color: var(--bg-panel); border-right: 1px solid var(--border-color); padding: 24px; display: flex; flex-direction: column; gap: 24px; box-shadow: 4px 0 15px rgba(0,0,0,0.3); }
.brand-zone { display: flex; align-items: center; gap: 12px; } .logo-icon { font-size: 26px; color: var(--color-accent); }
.brand-text h2 { margin: 0; font-size: 1.1rem; color: var(--color-primary); } .brand-text span { font-size: 0.65rem; color: var(--color-text-muted); }
.divider { border: 0; height: 1px; background: var(--border-color); margin: 0; }
.section-label { display: block; font-size: 0.72rem; color: var(--color-text-muted); margin-bottom: 6px; }
.search-field-wrapper { display: flex; flex-direction: column; gap: 12px; } .input-container { position: relative; width: 100%; }
.cyber-input { width: 100%; background-color: var(--bg-input); border: 1px solid var(--border-color); color: #fff; padding: 10px 12px; border-radius: 4px; box-sizing: border-box; font-size: 0.9rem; font-family: monospace; outline: none; }
.suggestions-menu { position: absolute; top: calc(100% + 4px); left: 0; width: 100%; background-color: #151a26; border: 1px solid var(--color-primary); border-radius: 4px; padding: 0; margin: 0; list-style: none; z-index: 999; max-height: 180px; overflow-y: auto; }
.suggestion-item { padding: 10px 12px; cursor: pointer; display: flex; justify-content: space-between; align-items: center; font-size: 0.85rem; border-bottom: 1px solid #1e2535; }
.suggestion-item:hover { background-color: var(--bg-input); } .s-name { color: #fff; } .s-id { color: var(--color-accent); font-size: 0.75rem; }
.btn-execute { background-color: var(--color-primary); color: #000; border: none; padding: 12px; border-radius: 4px; font-weight: bold; cursor: pointer; }
.btn-execute:hover { background-color: var(--color-accent); }
.btn-execute.active { background-color: #ffb703; color: #000; }
.btn-execute:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-sync { background-color: #7c3aed; color: #fff; }
.btn-sync:hover { background-color: #a78bfa; }

.status-card { background-color: var(--bg-input); border: 1px solid var(--border-color); padding: 12px; border-radius: 4px; display: flex; align-items: center; gap: 12px; }
.pulse-light { width: 8px; height: 8px; border-radius: 50%; } .pulse-light.success { background-color: var(--color-accent); } .pulse-light.warning { background-color: var(--color-warn); animation: pulse 1s infinite; } .pulse-light.danger { background-color: var(--color-danger); } .pulse-light.syncing { background-color: var(--color-primary); animation: pulse 0.6s infinite; }
.status-text { font-size: 0.85rem; } .error-details { color: var(--color-danger); font-size: 0.75rem; margin: 6px 0 0 0; }
.sync-result { margin-top: 8px; }
.sync-summary { color: var(--color-accent); font-size: 0.78rem; margin: 0; }
.sync-errors { display: flex; flex-direction: column; gap: 2px; margin: 4px 0 0 0; }
.sync-errors span { color: var(--color-warn); font-size: 0.68rem; }
.cache-path { color: #4e5d78; font-size: 0.65rem; margin: 8px 0 0 0; word-break: break-all; }
.cache-ts { color: #00f5d4; font-size: 0.75rem; font-weight: bold; margin: 8px 0 0 0; font-family: monospace; }
.timezone-switch { display: flex; align-items: center; gap: 4px; margin-top: 10px; }
.tz-label { color: #4e5d78; font-size: 0.65rem; margin-right: 4px; }
.timezone-switch button { background: #0a0d14; border: 1px solid #262e3f; color: #4e5d78; padding: 2px 8px; border-radius: 3px; cursor: pointer; font: inherit; font-size: 0.65rem; }
.timezone-switch button.active { background: #00b4d8; color: #000; border-color: #00b4d8; }
.main-workspace { display: flex; flex-direction: column; height: 100vh; background-color: var(--bg-deep); }
.workspace-header { background-color: var(--bg-panel); border-bottom: 1px solid var(--border-color); padding: 14px 24px; display: flex; justify-content: space-between; align-items: center; font-size: 0.8rem; }
.breadcrumb { color: var(--color-accent); font-weight: bold; } .timestamp { color: var(--color-text-muted); }
.data-view-panel { flex: 1; padding: 24px; overflow-y: auto; } .dashboard-welcome { height: 100%; display: flex; align-items: center; justify-content: center; }
.welcome-terminal { background-color: #07090f; border: 1px solid var(--border-color); padding: 30px; border-radius: 4px; width: 85%; max-width: 600px; }
.console-line { color: #4af626; margin: 6px 0; } @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.3; } }
</style>