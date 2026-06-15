<script setup lang="ts">
import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ParetoJudgeStatus, JudgeResult, ParetoBeatenReport, BeatenMetricDiff } from "../types/om";

const props = defineProps<{ puzzleId: string }>();

const metricPool = [
  { id: "cost", label: "Cost (G)" }, { id: "cycles", label: "Cycles (C)" },
  { id: "area", label: "Area (A)" }, { id: "instructions", label: "Instr (I)" },
  { id: "height", label: "Height (H)" }, { id: "width", label: "Width (W)" },
  { id: "boundingHex", label: "Bound (B)" }, { id: "rate", label: "Rate (R)" },
];

// ── 输入 ref ──
const cost = ref<number | null>(null);
const cycles = ref<number | null>(null);
const area = ref<number | null>(null);
const instructions = ref<number | null>(null);
const height = ref<number | null>(null);
const width = ref<number | null>(null);
const boundingHex = ref<number | null>(null);
const rate = ref<number | null>(null);
const overlap = ref(false);
const trackless = ref(false);
const activeMetrics = ref<string[]>(["cost", "cycles", "area"]);

// ── 判定结果 ──
const loading = ref(false);
const status = ref<ParetoJudgeStatus | null>(null); // null = 尚未判定
const totalCompared = ref(0);
const reports = ref<ParetoBeatenReport[]>([]);
const errorMsg = ref<string | null>(null);

// ── 草稿缓存：按 puzzleId 保存输入，切换关卡后恢复 ──
interface DraftSnapshot {
  cost: number | null; cycles: number | null; area: number | null;
  instructions: number | null; height: number | null; width: number | null;
  boundingHex: number | null; rate: number | null;
  overlap: boolean; trackless: boolean;
  activeMetrics: string[];
}
const draftCache = ref<Record<string, DraftSnapshot>>({});

function saveDraft() {
  if (!props.puzzleId) return;
  draftCache.value[props.puzzleId] = {
    cost: cost.value, cycles: cycles.value, area: area.value,
    instructions: instructions.value, height: height.value, width: width.value,
    boundingHex: boundingHex.value, rate: rate.value,
    overlap: overlap.value, trackless: trackless.value,
    activeMetrics: [...activeMetrics.value],
  };
}

function restoreDraft(puzzleId: string) {
  const saved = draftCache.value[puzzleId];
  if (saved) {
    cost.value = saved.cost; cycles.value = saved.cycles;
    area.value = saved.area; instructions.value = saved.instructions;
    height.value = saved.height; width.value = saved.width;
    boundingHex.value = saved.boundingHex; rate.value = saved.rate;
    overlap.value = saved.overlap; trackless.value = saved.trackless;
    activeMetrics.value = saved.activeMetrics;
  } else {
    cost.value = cycles.value = area.value = instructions.value = null;
    height.value = width.value = boundingHex.value = rate.value = null;
    overlap.value = false; trackless.value = false;
    activeMetrics.value = ["cost", "cycles", "area"];
  }
}

// ── 关卡切换：恢复草稿 + 清空判定 ──
watch(() => props.puzzleId, (newId) => {
  status.value = null; reports.value = []; totalCompared.value = 0; errorMsg.value = null;
  if (newId) restoreDraft(newId);
}, { immediate: true });

// ── 输入解析（替代 v-model.number，防止中途触发） ──
const toInt = (raw: string): number | null => raw === "" ? null : Math.trunc(Number(raw));
const toFloat = (raw: string): number | null => raw === "" ? null : Number(raw);

// ── 指标切换：清空旧结果，存草稿 ──
const toggleMetric = (metric: string) => {
  const idx = activeMetrics.value.indexOf(metric);
  if (idx >= 0) activeMetrics.value.splice(idx, 1);
  else activeMetrics.value.push(metric);
  status.value = null; reports.value = []; totalCompared.value = 0;
  saveDraft();
};

// ── 手动触发判定 ──
let judgeVersion = 0;
const doJudge = async () => {
  if (!props.puzzleId || activeMetrics.value.length === 0) return;
  saveDraft();
  const myVersion = ++judgeVersion;
  loading.value = true; errorMsg.value = null;
  try {
    const result = await invoke<JudgeResult>("judge_draft", {
      draft: {
        cost: cost.value, cycles: cycles.value,
        area: area.value, instructions: instructions.value,
        height: height.value ?? null, width: width.value ?? null,
        boundingHex: boundingHex.value ?? null, rate: rate.value ?? null,
        overlap: overlap.value, trackless: trackless.value,
        active_metrics: activeMetrics.value,
      },
      puzzleId: props.puzzleId,
    });
    if (myVersion !== judgeVersion) return;
    status.value = result.status;
    totalCompared.value = result.totalCompared;
    reports.value = result.reports;
    errorMsg.value = null;
  } catch (err) {
    if (myVersion === judgeVersion) { errorMsg.value = String(err); status.value = null; }
  } finally {
    if (myVersion === judgeVersion) loading.value = false;
  }
};

// ── 样式辅助 ──
const statusColor = (): string => {
  switch (status.value) {
    case "Ok": return "#00f5d4";
    case "UnknownBreaking": return "#ffb703";
    case "AlreadyPresented": return "#00b4d8";
    case "NothingBeaten": return "#ff4a4a";
    default: return "#4e5d78";
  }
};

const statusLabel = (): string => {
  switch (status.value) {
    case "Ok": return "PARETO OK";
    case "Unknown": return "UNKNOWN";
    case "UnknownBreaking": return "BREAKING";
    case "AlreadyPresented": return "DUPLICATE";
    case "NothingBeaten": return "DOMINATED";
    default: return "—";
  }
};

// ── Diff 展示逻辑 ──
// 后端：absoluteDiff = draft_val - record_val（正数 = 你比记录差）
//       percentageDiff = draft / record * 100（>100% = 你比记录差）
// 对 Rate 特殊：越高越好，所以正负语义反转
const HIGHER_IS_BETTER = new Set(["rate"]);

interface DiffCell {
  metricKey: string;
  label: string;
  yourVal: number;        // 你输入的值
  recordVal: number;      // 记录的值
  absDiff: number;        // |你 - 记录|，始终为正
  pct: number;            // 劣化百分比（始终 >= 0，0 表示持平）
  dominated: boolean;     // 你在这个维度是否被压制
  tied: boolean;          // 完全持平
}

const METRIC_LABELS: Record<string, string> = {
  cost: "Cost", cycles: "Cycles", area: "Area", instructions: "Instr",
  height: "Height", width: "Width", bhex: "Bound", rate: "Rate",
};

function buildDiffCells(rep: ParetoBeatenReport): DiffCell[] {
  const entries: Array<{ key: string; labelKey: string; diff: BeatenMetricDiff | null }> = [
    { key: "cost",         labelKey: "cost",    diff: rep.costDiff },
    { key: "cycles",       labelKey: "cycles",  diff: rep.cyclesDiff },
    { key: "area",         labelKey: "area",    diff: rep.areaDiff },
    { key: "instructions", labelKey: "instructions", diff: rep.instructionsDiff },
    { key: "height",       labelKey: "height",  diff: rep.heightDiff },
    { key: "width",        labelKey: "width",   diff: rep.widthDiff },
    { key: "boundingHex",  labelKey: "bhex",    diff: rep.bhexDiff },
    { key: "rate",         labelKey: "rate",    diff: rep.rateDiff },
  ];

  const cells: DiffCell[] = [];
  for (const e of entries) {
    if (!e.diff) continue;  // 这个指标没参与比较，不展示

    const { actualValue, absoluteDiff, percentageDiff } = e.diff;
    // absoluteDiff = draft - record（正数 = 你差；对 rate 反转：负数 = 你差）
    const higherBetter = HIGHER_IS_BETTER.has(e.key);
    const rawWorseBy = higherBetter ? -absoluteDiff : absoluteDiff;
    // rawWorseBy > 0 → 你比记录差；< 0 → 你比记录好（理论上 NothingBeaten 不会出现）；= 0 → 持平

    // yourVal = record + absoluteDiff（后端定义：draft = record + absoluteDiff）
    const yourVal = actualValue + absoluteDiff;
    // pct：你比记录差了多少 %。percentageDiff 是 draft/record*100，
    // 对 lower-better：> 100% = 你差，劣化量 = percentageDiff - 100
    // 对 higher-better：< 100% = 你差，劣化量 = 100 - percentageDiff
    const rawPct = higherBetter ? (100 - percentageDiff) : (percentageDiff - 100);
    const dominated = rawWorseBy > 0.00001;
    const tied = Math.abs(rawWorseBy) <= 0.00001;

    cells.push({
      metricKey: e.key,
      label: METRIC_LABELS[e.labelKey] ?? e.labelKey,
      yourVal,
      recordVal: actualValue,
      absDiff: Math.abs(rawWorseBy),
      pct: Math.max(0, rawPct),
      dominated,
      tied,
    });
  }
  return cells;
}

const allFilled = (): boolean => activeMetrics.value.every(m => {
  switch (m) {
    case "cost": return cost.value !== null;
    case "cycles": return cycles.value !== null;
    case "area": return area.value !== null;
    case "instructions": return instructions.value !== null;
    case "height": return height.value !== null;
    case "width": return width.value !== null;
    case "boundingHex": return boundingHex.value !== null;
    case "rate": return rate.value !== null;
    default: return false;
  }
});
</script>

<template>
  <div class="judge-panel">
    <div class="judge-header">
      <span class="judge-title">PARETO DRAFT JUDGE</span>
      <span v-if="props.puzzleId" class="judge-puzzle">#{{ props.puzzleId }}</span>
    </div>

    <!-- 指标选择 -->
    <div class="chip-row">
      <span class="chip-label">TARGETS:</span>
      <label v-for="m in metricPool" :key="m.id" class="chip" :class="{ checked: activeMetrics.includes(m.id) }">
        <input type="checkbox" :checked="activeMetrics.includes(m.id)" @change="toggleMetric(m.id)" /> {{ m.label }}
      </label>
    </div>

    <!-- O / T 开关 -->
    <div class="toggle-row">
      <button :class="{ active: overlap }" @click="overlap = !overlap; status = null">OVERLAP</button>
      <button :class="{ active: trackless }" @click="trackless = !trackless; status = null">TRACKLESS</button>
    </div>

    <!-- 输入框 -->
    <div class="inputs-grid">
      <div v-if="activeMetrics.includes('cost')" class="field"><label>Cost</label><input type="number" :value="cost" @input="cost = toInt(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('cycles')" class="field"><label>Cycles</label><input type="number" :value="cycles" @input="cycles = toInt(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('area')" class="field"><label>Area</label><input type="number" :value="area" @input="area = toInt(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('instructions')" class="field"><label>Instr</label><input type="number" :value="instructions" @input="instructions = toInt(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('height')" class="field"><label>Height</label><input type="number" :value="height" @input="height = toInt(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('width')" class="field"><label>Width</label><input type="number" step="0.1" :value="width" @input="width = toFloat(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('boundingHex')" class="field"><label>B-Hex</label><input type="number" :value="boundingHex" @input="boundingHex = toInt(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
      <div v-if="activeMetrics.includes('rate')" class="field"><label>Rate</label><input type="number" step="0.1" :value="rate" @input="rate = toFloat(($event.target as HTMLInputElement).value)" class="no-spin" placeholder="—" /></div>
    </div>

    <!-- 判定按钮 -->
    <button
      class="judge-btn"
      :class="{ loading }"
      :disabled="loading || !props.puzzleId || activeMetrics.length === 0"
      @click="doJudge"
    >
      {{ loading ? 'JUDGING…' : 'JUDGE' }}
    </button>

    <!-- 结果区 -->
    <div v-if="status !== null" class="result" :class="status.toLowerCase()">
      <div class="badge-row">
        <span class="badge" :style="{ background: statusColor() }">{{ statusLabel() }}</span>
        <span v-if="totalCompared > 0" class="vs">vs {{ totalCompared }} records</span>
      </div>

      <p v-if="status === 'Ok' && allFilled()" class="msg ok">🏆 NEW PARETO FRONTIER — UNBEATEN ACROSS ALL DIMENSIONS</p>
      <p v-if="status === 'Ok' && !allFilled()" class="msg ok">✅ NOT DOMINATED — FILL ALL METRICS TO CONFIRM PARETO</p>
      <p v-if="status === 'UnknownBreaking'" class="msg breaking">🔥 RECORD BREAKING — BEATS HISTORICAL BEST ON ≥1 METRIC</p>
      <p v-if="status === 'Unknown'" class="msg warn">… FILL MORE METRICS TO DETERMINE STATUS</p>
      <p v-if="status === 'AlreadyPresented'" class="msg warn">◆ DUPLICATE — THIS EXACT COMBINATION ALREADY EXISTS</p>

      <!-- NothingBeaten：逐条对手展示 -->
      <div v-if="status === 'NothingBeaten' && reports.length > 0" class="beaten-zone">
        <p class="beaten-title">❌ DOMINATED BY {{ reports.length }} RECORD{{ reports.length > 1 ? 'S' : '' }}</p>

        <div v-for="(rep, ri) in reports" :key="ri" class="beaten-card">
          <!-- 对手标题 -->
          <div class="beaten-meta">
            <span class="beaten-score">{{ rep.betterRecord.fullFormattedScore || `${rep.betterRecord.score?.cost}g/${rep.betterRecord.score?.cycles}c/${rep.betterRecord.score?.area}a` }}</span>
            <span v-if="rep.betterRecord.author" class="by"> · {{ rep.betterRecord.author }}</span>
          </div>

          <!-- 逐维度对比表 -->
          <table class="diff-table">
            <thead>
              <tr>
                <th>Metric</th>
                <th>Yours</th>
                <th>Record</th>
                <th>Gap</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="cell in buildDiffCells(rep)"
                :key="cell.metricKey"
                :class="{ 'row-dominated': cell.dominated, 'row-tied': cell.tied }"
              >
                <td class="metric-name">{{ cell.label }}</td>
                <!-- 你的值：被压制时标红，持平时灰 -->
                <td :class="cell.dominated ? 'val-bad' : 'val-ok'">{{ cell.yourVal }}</td>
                <!-- 记录的值 -->
                <td class="val-record">{{ cell.recordVal }}</td>
                <!-- Gap：只在被压制时显示具体差距 -->
                <td class="val-gap">
                  <template v-if="cell.dominated">
                    <span class="gap-abs">+{{ cell.absDiff % 1 === 0 ? cell.absDiff : cell.absDiff.toFixed(2) }}</span>
                    <span class="gap-pct"> (+{{ cell.pct.toFixed(1) }}%)</span>
                  </template>
                  <template v-else-if="cell.tied">
                    <span class="gap-tied">TIED</span>
                  </template>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <p v-if="errorMsg" class="err">⚠ {{ errorMsg }}</p>
    </div>

    <!-- 尚未判定时的提示 -->
    <div v-else-if="!loading" class="idle-hint">
      Fill in your scores and press JUDGE
    </div>
  </div>
</template>

<style scoped>
.judge-panel { background: #121620; border: 1px solid #262e3f; border-radius: 6px; padding: 16px; font-family: monospace; }
.judge-header { display: flex; justify-content: space-between; margin-bottom: 12px; }
.judge-title { color: #00b4d8; font-size: 0.82rem; font-weight: bold; }
.judge-puzzle { color: #00f5d4; font-size: 0.72rem; }

.chip-row { display: flex; flex-wrap: wrap; align-items: center; gap: 5px; margin-bottom: 10px; }
.chip-label { color: #4e5d78; font-size: 0.68rem; font-weight: bold; }
.chip { background: #0a0d14; border: 1px solid #262e3f; color: #4e5d78; padding: 2px 7px; border-radius: 3px; cursor: pointer; font-size: 0.68rem; }
.chip input { display: none; }
.chip.checked { background: rgba(0,245,212,0.08); border-color: #00f5d4; color: #00f5d4; font-weight: bold; }

.toggle-row { display: flex; gap: 6px; margin-bottom: 10px; }
.toggle-row button { background: #0a0d14; border: 1px solid #262e3f; color: #4e5d78; padding: 3px 14px; cursor: pointer; font: inherit; font-size: 0.72rem; font-weight: bold; border-radius: 3px; }
.toggle-row button.active { background: #ef4444; color: #fff; border-color: #ef4444; }

.inputs-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(90px, 1fr)); gap: 6px; margin-bottom: 12px; }
.field { display: flex; flex-direction: column; gap: 1px; }
.field label { color: #4e5d78; font-size: 0.65rem; }
.field input { background: #0a0d14; border: 1px solid #262e3f; color: #00f5d4; padding: 5px 6px; border-radius: 3px; font: inherit; font-size: 0.8rem; outline: none; }
.field input:focus { border-color: #00b4d8; }
.no-spin::-webkit-outer-spin-button, .no-spin::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }
.no-spin { -moz-appearance: textfield; }

.judge-btn {
  width: 100%; padding: 10px; margin-bottom: 14px;
  background: #00b4d8; color: #000; border: none; border-radius: 4px;
  font: inherit; font-size: 0.85rem; font-weight: bold; cursor: pointer;
  transition: background 0.15s;
}
.judge-btn:hover:not(:disabled) { background: #00f5d4; }
.judge-btn:disabled { opacity: 0.45; cursor: not-allowed; }
.judge-btn.loading { background: #262e3f; color: #4e5d78; }

.result { border-left: 3px solid #262e3f; padding: 10px 12px; background: #0a0d14; border-radius: 0 4px 4px 0; }
.result.ok            { background: rgba(0,245,212,0.04); border-color: #00f5d4; }
.result.unknownbreaking { background: rgba(255,183,3,0.04); border-color: #ffb703; }
.result.nothingbeaten { background: rgba(255,74,74,0.04); border-color: #ff4a4a; }
.result.alreadypresented { background: rgba(0,180,216,0.04); border-color: #00b4d8; }
.result.unknown       { background: #0a0d14; border-color: #262e3f; }

.badge-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.badge { color: #000; padding: 3px 10px; border-radius: 3px; font-size: 0.75rem; font-weight: bold; white-space: nowrap; }
.vs { color: #4e5d78; font-size: 0.7rem; }
.msg { font-size: 0.78rem; margin-top: 8px; font-weight: bold; }
.msg.ok { color: #00f5d4; }
.msg.breaking { color: #ffb703; animation: blink 1.5s infinite; }
.msg.warn { color: #ffb703; border-left: 2px solid #ffb703; padding-left: 8px; font-weight: normal; font-size: 0.72rem; }
@keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0.5; } }

.idle-hint { color: #4e5d78; font-size: 0.72rem; text-align: center; padding: 12px 0; }

/* NothingBeaten 区域 */
.beaten-zone { margin-top: 12px; }
.beaten-title { color: #ff4a4a; font-size: 0.75rem; font-weight: bold; margin-bottom: 8px; }
.beaten-card { background: rgba(255,74,74,0.03); border: 1px solid #1a1f2c; border-radius: 4px; padding: 8px 10px; margin-bottom: 8px; }
.beaten-meta { font-size: 0.72rem; margin-bottom: 6px; }
.beaten-score { color: #00f5d4; font-weight: bold; }
.by { color: #4e5d78; }

/* 对比表 */
.diff-table { width: 100%; border-collapse: collapse; font-size: 0.72rem; }
.diff-table th {
  color: #4e5d78; text-align: left; padding: 3px 8px;
  border-bottom: 1px solid #1a1f2c; font-weight: normal;
}
.diff-table td { padding: 3px 8px; }

/* 行级高亮 */
.row-dominated { background: rgba(255,74,74,0.05); }
.row-tied      { background: transparent; }

.metric-name { color: #4e5d78; }

/* 你的值 */
.val-bad { color: #ff4a4a; font-weight: bold; }  /* 被压制 */
.val-ok  { color: #e2e8f0; }                     /* 持平（tied 时也是这列） */

/* 记录的值 */
.val-record { color: #00f5d4; }

/* Gap 列 */
.val-gap { white-space: nowrap; }
.gap-abs { color: #ff4a4a; font-weight: bold; }
.gap-pct { color: #ff7070; font-size: 0.68rem; }
.gap-tied { color: #4e5d78; font-size: 0.68rem; letter-spacing: 0.05em; }

.err { color: #ff4a4a; font-size: 0.7rem; margin-top: 8px; }
</style>
