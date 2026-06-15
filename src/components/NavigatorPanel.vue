<script setup lang="ts">
import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { OmRecordDTO, NavigatorResult } from "../types/om";

const props = defineProps<{ puzzleId: string; records: OmRecordDTO[] }>();

const ALL_DIMS = [
  { key: "cost", label: "G" }, { key: "cycles", label: "C" },
  { key: "area", label: "A" }, { key: "instructions", label: "I" },
  { key: "height", label: "H" }, { key: "width", label: "W" },
  { key: "boundingHex", label: "B" }, { key: "rate", label: "R" },
];
const activeDims = ref<Set<string>>(new Set(["cost","cycles","area"]));

function toggleDim(k: string) {
  const next = new Set(activeDims.value);
  if (next.has(k)) { if (next.size > 1) next.delete(k); }
  else next.add(k);
  activeDims.value = next;
  lastScanKey.value = ""; // 维度变 → 缓存失效
}

const DIM_LABELS: Record<string, string> = {
  cost: "G", cycles: "C", area: "A", instructions: "I",
  height: "H", width: "W", boundingHex: "B", rate: "R",
};
const selDims = () => [...activeDims.value];

const loading = ref(false);
const weakRecords = ref<{ record: OmRecordDTO; nav: NavigatorResult }[]>([]);
const selected = ref<{ record: OmRecordDTO; nav: NavigatorResult } | null>(null);
const error = ref<string | null>(null);
const lastScanKey = ref("");
watch(() => props.puzzleId, () => { weakRecords.value = []; lastScanKey.value = ""; });

// 取指标值
function getVal(s: any, k: string): number {
  if (k === "cost") return s.cost;
  if (k === "cycles") return s.cycles;
  if (k === "area") return s.area;
  if (k === "instructions") return s.instructions;
  if (k === "height") return s.height ?? 0;
  if (k === "width") return s.width ?? 0;
  if (k === "boundingHex") return s.boundingHex ?? 0;
  if (k === "rate") return s.rate ?? 0;
  return 0;
}

async function scanAll() {
  if (!props.puzzleId || loading.value) return;
  const scanKey = props.puzzleId + "|" + [...activeDims.value].sort().join(",");
  // 缓存命中直接恢复
  if (scanKey === lastScanKey.value && weakRecords.value.length) return;
  lastScanKey.value = scanKey;
  loading.value = true; error.value = null;
  weakRecords.value = []; selected.value = null;
  const dims = [...activeDims.value];

  // Step 1: 收集有效记录
  const all = props.records.filter(r => r.score && r.score.cost > 0 && r.score.cycles > 0 && r.score.area > 0 && r.score.instructions > 0);
  if (!all.length) { loading.value = false; return; }

  // Step 2: 按 O/T 分组，每组内算 Pareto 前沿
  const n = all.length;
  const groups = new Map<string, number[]>();
  for (let i = 0; i < n; i++) {
    const s = all[i].score!;
    const key = `${s.overlap}|${s.trackless}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(i);
  }
  const frontierIdx = new Set<number>();
  for (const [, idxs] of groups) {
    const gn = idxs.length;
    for (let a = 0; a < gn; a++) {
      const i = idxs[a];
      let dominated = false;
      for (let b = 0; b < gn; b++) {
        if (a === b) continue;
        const j = idxs[b];
        let anyStrict = false;
        let dominates = true;
        for (const d of dims) {
          const vi = getVal(all[i].score!, d);
          const vj = getVal(all[j].score!, d);
          if (vi < vj - 1e-9) { dominates = false; break; }
          if (vj < vi - 1e-9) anyStrict = true;
        }
        if (dominates && anyStrict) { dominated = true; break; }
      }
      if (!dominated) frontierIdx.add(i);
    }
  }

  // Step 3: 只对前沿记录跑 LP
  const out: typeof weakRecords.value = [];
  for (const i of frontierIdx) {
    const rec = all[i];
    const s = rec.score!;
    try {
      const nav = await invoke<NavigatorResult>("navigate_pareto", {
        draft: {
          cost: s.cost, cycles: s.cycles, area: s.area, instructions: s.instructions,
          height: s.height ?? null, width: s.width ?? null,
          boundingHex: s.boundingHex ?? null, rate: s.rate ?? null,
          overlap: s.overlap, trackless: s.trackless,
          active_metrics: dims,
        },
        puzzleId: props.puzzleId,
      });
      if (nav.isWeak) out.push({ record: rec, nav });
    } catch (e) {
      error.value = String(e);
      break;
    }
  }
  out.sort((a, b) => b.nav.weaknessGap - a.nav.weaknessGap);
  weakRecords.value = out;
  loading.value = false;
}

const modalRef = ref<HTMLDivElement | null>(null);
const copying = ref(false);

async function copyModal() {
  if (!selected.value || copying.value) return;
  copying.value = true;
  try {
    const dims = selDims();
    const dimLabels = dims.map(d => DIM_LABELS[d]);
    const draftVals = dims.map(d => (selected.value!.record.score as any)?.[d] ?? '—');
    const targetVals = dims.map(d => ((selected.value!.record.score as any)?.[d] + (selected.value!.nav.deltaMetrics[d] || 0)).toFixed(1));
    const allies = selected.value!.nav.lambdaAllies;

    const ROW_H = 18, HEAD_H = 22, PAD = 10, COL_W = 70, NAME_W = 200;
    const W = NAME_W + dims.length * COL_W + PAD * 2;
    const H = HEAD_H + (3 + allies.length) * ROW_H + PAD * 3 + 20;

    const trunc = (ctx: CanvasRenderingContext2D, text: string, maxW: number) => {
      if (ctx.measureText(text).width <= maxW) return text;
      let s = text;
      while (s.length > 3 && ctx.measureText(s + "…").width > maxW) s = s.slice(0, -1);
      return s + "…";
    };

    const canvas = document.createElement("canvas");
    canvas.width = W * 2; canvas.height = H * 2;
    const ctx = canvas.getContext("2d")!;
    ctx.scale(2, 2);
    ctx.fillStyle = "#121620"; ctx.fillRect(0, 0, W, H);

    // Title
    ctx.fillStyle = "#a78bfa"; ctx.font = "bold 12px monospace";
    ctx.fillText("CONCAVE WEAKNESS", PAD, 18);

    // Delta line
    const deltaStr = "Target Δ: " + dims.map(d => {
      const dv = selected.value!.nav.deltaMetrics[d];
      return (dv >= 0 ? "+" : "") + (dv % 1 === 0 ? dv : dv.toFixed(1)) + DIM_LABELS[d];
    }).join(" ");
    ctx.fillStyle = "#ffb703"; ctx.font = "11px monospace";
    ctx.fillText(deltaStr, PAD, HEAD_H + PAD);

    const top = HEAD_H + PAD + 8;
    function row(y: number, cells: string[], cls: string) {
      if (cls === "draft") { ctx.fillStyle = "#ff4a4a"; }
      else if (cls === "target") { ctx.fillStyle = "#00f5d4"; ctx.font = "bold 11px monospace"; }
      else if (cls === "ally") { ctx.fillStyle = "#a78bfa"; }
      else { ctx.fillStyle = "#4e5d78"; }
      if (cls !== "target") ctx.font = "11px monospace";
      ctx.fillText(cells[0], PAD, y);
      for (let i = 0; i < dims.length; i++) {
        ctx.textAlign = "right";
        ctx.fillText(cells[i + 1], NAME_W + (i + 1) * COL_W - 4, y);
      }
      ctx.textAlign = "left";
    }
    // Header
    ctx.fillStyle = "#4e5d78"; ctx.font = "10px monospace";
    ctx.fillText("", PAD, top);
    for (let i = 0; i < dimLabels.length; i++) {
      ctx.textAlign = "right";
      ctx.fillText(dimLabels[i], NAME_W + (i + 1) * COL_W - 4, top);
    }
    ctx.textAlign = "left";
    // Line
    ctx.strokeStyle = "#1a1f2c"; ctx.beginPath();
    ctx.moveTo(PAD, top + 3); ctx.lineTo(W - PAD, top + 3); ctx.stroke();

    row(top + ROW_H, ["YOU", ...draftVals.map(String)], "draft");
    row(top + ROW_H * 2, ["TARGET", ...targetVals], "target");
    for (let ai = 0; ai < allies.length; ai++) {
      const a = allies[ai];
      const name = Math.round(a.weight * 100) + "% " + (a.author || a.recordId);
      row(top + ROW_H * (3 + ai), [
        trunc(ctx, name, NAME_W - 12),
        ...dims.map(d => a.metricValues[d] != null ? String(a.metricValues[d]) : '—')
      ], "ally");
    }

    canvas.toBlob(async (blob) => {
      if (blob) {
        try { await navigator.clipboard.write([new ClipboardItem({"image/png": blob})]); }
        catch(e) { console.error(e); }
      }
    }, "image/png");
  } catch(e) { console.error(e); }
  copying.value = false;
}
</script>

<template>
  <div class="nav-panel">
    <div class="nav-header">
      <span class="nav-title">PARETO NAVIGATOR</span>
      <span v-if="props.puzzleId" class="nav-puzzle">#{{ props.puzzleId }}</span>
    </div>

    <div class="dim-row">
      <span class="dim-label">DIMS:</span>
      <label v-for="d in ALL_DIMS" :key="d.key" class="dim-chip" :class="{ on: activeDims.has(d.key) }">
        <input type="checkbox" :checked="activeDims.has(d.key)" @change="toggleDim(d.key)" />
        {{ d.label }}
      </label>
    </div>

    <button class="nav-btn" :class="{ loading }" :disabled="loading || !props.puzzleId" @click="scanAll">
      {{ loading ? 'SCANNING…' : 'SCAN ALL' }}
    </button>

    <p v-if="error" class="err">⚠ {{ error }}</p>

    <!-- 弱解列表 -->
    <div v-if="weakRecords.length" class="weak-list">
      <div class="weak-count">{{ weakRecords.length }} weak record{{ weakRecords.length>1?'s':'' }} (sorted by gap)</div>
      <div
        v-for="(item, i) in weakRecords" :key="i"
        class="weak-row"
        :class="{ sel: selected === item }"
        @click="selected = selected === item ? null : item"
      >
        <span class="weak-score">{{ item.record.fullFormattedScore || item.record.smartFormattedScore || '—' }}</span>
        <span class="weak-gap">gap: {{ item.nav.weaknessGap.toFixed(6) }}</span>
      </div>
    </div>
    <p v-else-if="!loading && weakRecords.length === 0 && !error" class="nav-empty">No concave-weak records found.</p>

    <!-- 选中弱解的弹窗 -->
    <div v-if="selected" class="nav-overlay" @click.self="selected = null">
      <div class="nav-modal" ref="modalRef">
        <div class="nav-modal-hdr">
          <span class="nav-modal-title">CONCAVE WEAKNESS DETAIL</span>
          <div class="nav-modal-acts">
            <button @click="copyModal" class="nav-copy-btn" :disabled="copying">{{ copying ? '…' : 'COPY' }}</button>
            <button @click="selected = null" class="nav-close-btn">×</button>
          </div>
        </div>
        <div class="nav-vector">
          <span class="vl">Target Δ:</span>
          <span v-for="d in selDims()" :key="d" class="vi">
            {{ (selected.nav.deltaMetrics[d] >= 0 ? '+' : '') + (selected.nav.deltaMetrics[d] % 1 === 0 ? selected.nav.deltaMetrics[d] : selected.nav.deltaMetrics[d].toFixed(1)) }}{{ DIM_LABELS[d] }}
          </span>
        </div>
        <div v-if="selected.nav.lambdaAllies.length > 0" class="cmp-table-wrap">
          <table class="cmp-table">
            <thead>
              <tr><th></th><th v-for="d in selDims()" :key="d">{{ DIM_LABELS[d] }}</th></tr>
            </thead>
            <tbody>
              <tr class="draft"><td>YOU</td><td v-for="d in selDims()" :key="d">{{ (selected.record.score as any)?.[d] ?? '—' }}</td></tr>
              <tr class="target"><td>TARGET</td><td v-for="d in selDims()" :key="d">{{ ((selected.record.score as any)?.[d] + (selected.nav.deltaMetrics[d] || 0)).toFixed(1) }}</td></tr>
              <tr v-for="(ally, ai) in selected.nav.lambdaAllies" :key="ai" class="ally">
                <td><span class="aw">{{ (ally.weight * 100).toFixed(0) }}%</span> {{ ally.author || ally.recordId }}</td>
                <td v-for="d in selDims()" :key="d">{{ ally.metricValues[d] ?? '—' }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.nav-panel { background: #121620; border: 1px solid #262e3f; border-radius: 6px; padding: 16px; font-family: monospace; }
.nav-header { display: flex; justify-content: space-between; margin-bottom: 8px; }
.nav-title { color: #a78bfa; font-size: 0.82rem; font-weight: bold; }
.nav-puzzle { color: #c084fc; font-size: 0.72rem; }

.dim-row { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; margin-bottom: 8px; }
.dim-label { color: #4e5d78; font-size: 0.65rem; font-weight: bold; margin-right: 4px; }
.dim-chip { background: #0a0d14; border: 1px solid #262e3f; color: #4e5d78; padding: 2px 7px; border-radius: 3px; cursor: pointer; font-size: 0.68rem; user-select: none; }
.dim-chip input { display: none; }
.dim-chip.on { background: rgba(167,139,250,0.08); border-color: #a78bfa; color: #a78bfa; font-weight: bold; }

.nav-btn { width: 100%; padding: 10px; margin-bottom: 12px; background: #6d28d9; color: #fff; border: none; border-radius: 4px; font: inherit; font-size: 0.85rem; font-weight: bold; cursor: pointer; }
.nav-btn:hover:not(:disabled) { background: #7c3aed; }
.nav-btn:disabled { opacity: 0.45; }
.nav-btn.loading { background: #262e3f; color: #4e5d78; }

.weak-list { display: flex; flex-direction: column; gap: 2px; max-height: 400px; overflow-y: auto; }
.weak-list::-webkit-scrollbar { width: 4px; }
.weak-list::-webkit-scrollbar-track { background: transparent; }
.weak-list::-webkit-scrollbar-thumb { background: #262e3f; border-radius: 2px; }
.weak-list::-webkit-scrollbar-thumb:hover { background: #3a4560; }
.weak-count { color: #ff4a4a; font-size: 0.7rem; font-weight: bold; padding: 4px 0; border-bottom: 1px solid #1a1f2c; margin-bottom: 4px; }
.weak-row { display: flex; justify-content: space-between; align-items: center; padding: 5px 8px; border-radius: 3px; cursor: pointer; border: 1px solid transparent; }
.weak-row:hover { background: #1a1f2c; }
.weak-row.sel { border-color: #a78bfa; background: rgba(167,139,250,0.06); }
.weak-score { color: #e2e8f0; font-size: 0.72rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 70%; }
.weak-gap { color: #ff4a4a; font-size: 0.65rem; font-weight: bold; white-space: nowrap; }

.nav-empty { color: #4e5d78; font-size: 0.7rem; text-align: center; padding: 12px 0; }

.nav-result { background: rgba(255,74,74,0.04); border: 1px solid #ff4a4a; border-radius: 4px; padding: 10px 12px; margin-top: 10px; }
.nav-vector { color: #ffb703; font-size: 0.68rem; margin-bottom: 6px; }
.vec-label { color: #4e5d78; margin-right: 4px; }
.vec-item { color: #00f5d4; margin: 0 2px; }

.cmp-table-wrap { margin-top: 6px; overflow-x: auto; }
.cmp-table { width: 100%; border-collapse: collapse; font-size: 0.68rem; }
.cmp-table th { color: #4e5d78; text-align: right; padding: 2px 6px; border-bottom: 1px solid #1a1f2c; white-space: nowrap; }
.cmp-table td { text-align: right; padding: 2px 6px; white-space: nowrap; color: #e2e8f0; }
.cmp-table td:first-child { text-align: left; }
.cmp-draft td { color: #ff4a4a; }
.cmp-target td { color: #00f5d4; font-weight: bold; }
.cmp-ally td { color: #a78bfa; }
.aw { color: #ffb703; font-weight: bold; }

/* Modal table */
.nav-modal .cmp-table { width: 100%; border-collapse: collapse; font-size: 0.7rem; margin-top: 6px; }
.nav-modal .cmp-table th { color: #4e5d78; text-align: right; padding: 3px 8px; border-bottom: 1px solid #1a1f2c; white-space: nowrap; }
.nav-modal .cmp-table td { text-align: right; padding: 3px 8px; white-space: nowrap; color: #e2e8f0; }
.nav-modal .cmp-table td:first-child { text-align: left; }
.nav-modal .draft td { color: #ff4a4a; }
.nav-modal .target td { color: #00f5d4; font-weight: bold; }
.nav-modal .ally td { color: #a78bfa; }

.err { color: #ff4a4a; font-size: 0.7rem; margin-top: 8px; }

/* Modal */
.nav-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.75); z-index: 200; display: flex; align-items: center; justify-content: center; }
.nav-modal { background: #121620; border: 1px solid #a78bfa; border-radius: 6px; padding: 20px; max-width: 600px; width: 90%; max-height: 80vh; overflow-y: auto; font-family: monospace; }
.nav-modal-hdr { display: flex; justify-content: space-between; align-items: center; margin-bottom: 14px; }
.nav-modal-title { color: #a78bfa; font-size: 0.78rem; font-weight: bold; }
.nav-modal-acts { display: flex; gap: 6px; }
.nav-copy-btn { background: transparent; border: 1px solid #00b4d8; color: #00b4d8; font: inherit; font-size: 0.62rem; padding: 2px 8px; border-radius: 3px; cursor: pointer; }
.nav-copy-btn:hover { background: #00b4d8; color: #000; }
.nav-copy-btn:disabled { opacity: 0.5; }
.nav-close-btn { background: none; border: 1px solid #4e5d78; color: #4e5d78; font-size: 1rem; cursor: pointer; padding: 0 6px; border-radius: 3px; line-height: 1; }
.nav-close-btn:hover { border-color: #ff4a4a; color: #ff4a4a; }

.nav-vector { color: #ffb703; font-size: 0.68rem; margin-bottom: 8px; }
.vl { color: #4e5d78; margin-right: 4px; }
.vi { color: #00f5d4; margin: 0 2px; }
</style>
