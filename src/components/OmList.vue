<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from "vue";
import { OmRecordDTO } from "../types/om";

const props = defineProps<{
  records: OmRecordDTO[];
}>();

const viewMode = ref<"all" | "pareto">("all");
const sortExpr = ref("CG");
const sortOrder = ref<"asc" | "desc">("asc");

const METRICS: Record<string, string> = { G:"cost", C:"cycles", A:"area", I:"instructions", H:"height", W:"width", B:"bounding", R:"rate" };
const parseSortKeys = () => {
  let expr = sortExpr.value.toUpperCase();
  expr = expr.replace(/S4/g, "CGAI").replace(/S/g, "CGA");
  const chars = expr.replace(/[^GCRAIHWBX]/g, "").split("");
  const result: string[] = [];
  for (let i = 0; i < chars.length; i++) {
    if (chars[i] === "X") {
      const prev = i > 0 ? chars[i - 1] : "";
      if (prev === "C") result.push("x:ga");
      else if (prev === "G") result.push("x:ca");
      else if (prev === "A") result.push("x:gc");
      else result.push("x:ga");
    } else {
      result.push(METRICS[chars[i]] || "cycles");
    }
  }
  return result.filter(Boolean);
};

// 检测 sortExpr 中的 X 前缀，决定 DERIVED 列显示哪个乘法
const xMode = computed(() => {
  const m = sortExpr.value.toUpperCase().match(/([GCA])X/);
  if (!m) return null;
  if (m[1] === "C") return "ga";
  if (m[1] === "G") return "ca";
  if (m[1] === "A") return "gc";
  return null;
});
const filterTrackless = ref(false);
const filterOverlap = ref(true);
const scoreView = ref<"both" | "v" | "inf">("both");

// 解析 fullFormattedScore 为 @V / @∞ 两部分（按空格切分）
const splitScore = (raw: string | null) => {
  if (!raw) return { v: null, inf: null };
  const parts = raw.split(" ");
  if (parts.length >= 2) {
    return { v: parts[0].replace(/@V$/, ""), inf: parts.slice(1).join(" ") };
  }
  if (raw.includes("@∞")) return { v: null, inf: raw };
  return { v: raw.replace(/@V$/, ""), inf: null };
};

const isDominated = (a: OmRecordDTO, b: OmRecordDTO): boolean => {
  if (!a.score || !b.score) return false;
  return (b.score.cost <= a.score.cost && b.score.cycles <= a.score.cycles && b.score.area <= a.score.area) &&
         (b.score.cost < a.score.cost || b.score.cycles < a.score.cycles || b.score.area < a.score.area);
};

// 全局 Pareto 前沿 — 全量记录计算，不受 T/L 过滤影响
const paretoIds = computed(() => {
  const scored = props.records.filter(r => r.score !== null);
  const frontier = scored.filter(current =>
    !scored.some(other => isDominated(current, other))
  );
  return new Set(frontier.map(r => r.id));
});

// ── 聚合：T/L 过滤 → ViewMode → 指纹去重 → 排序 ──
const aggregatedRecords = computed(() => {
  let baseList = props.records.filter(r => r.score !== null);

  // T / !O 过滤器
  if (filterTrackless.value) {
    baseList = baseList.filter(r => r.score!.trackless);
  }
  if (filterOverlap.value) {
    baseList = baseList.filter(r => !r.score!.overlap);
  }

  // RECORD 模式：仅保留有分类标签的记录；FRONTIER 模式显示全部
  if (viewMode.value === "all") {
    baseList = baseList.filter(r => r.categoryIds && r.categoryIds.length > 0);
  }

  // 预先从全量数据（不受 T/L 过滤）算出哪些指纹属于 Pareto
  const paretoPrints = new Set<string>();
  for (const r of props.records) {
    if (r.score && paretoIds.value.has(r.id)) {
      const s = r.score;
      paretoPrints.add(`${s.cost}-${s.cycles}-${s.area}-${s.instructions}-${s.trackless}-${s.overlap}`);
    }
  }

  const map = new Map<string, { record: OmRecordDTO; categories: string[]; hasPareto: boolean }>();

  for (const r of baseList) {
    const s = r.score!;
    const fingerprint = `${s.cost}-${s.cycles}-${s.area}-${s.instructions}-${s.trackless}-${s.overlap}`;
    
    let currentCats: string[] = [];
    if (r.categoryIds && r.categoryIds.length > 0) {
      const seen = new Set<string>();
      for (const id of r.categoryIds) {
        const cat = id.split("_").pop()?.toUpperCase() || "";
        if (cat.length > 0 && !seen.has(cat)) { seen.add(cat); currentCats.push(cat); }
      }
    }

    const isPareto = paretoPrints.has(fingerprint);

    if (map.has(fingerprint)) {
      const existing = map.get(fingerprint)!;
      for (const cat of currentCats) {
        if (!existing.categories.includes(cat)) {
          existing.categories.push(cat);
        }
      }
      // Pareto 记录并入时升级标记
      if (isPareto) existing.hasPareto = true;
    } else {
      map.set(fingerprint, {
        record: r,
        categories: currentCats,
        hasPareto: isPareto
      });
    }
  }

  const result = Array.from(map.values()).map(item => {
    const s = item.record.score!;
    const sum = s.cost + s.cycles + s.area;
    const sum4 = sum + s.instructions;
    const ga = s.cost * s.area;
    const ca = s.cycles * s.area;
    const gc = s.cost * s.cycles;

    return {
      ...item,
      derived: { sum, sum4, ga, ca, gc },
      isPareto: item.hasPareto
    };
  });

  const keys = parseSortKeys();
  const getVal = (r: typeof result[0], k: string) => {
    const s = r.record.score!;
    switch (k) {
      case "cost": return s.cost;
      case "cycles": return s.cycles;
      case "area": return s.area;
      case "instructions": return s.instructions;
      case "height": return s.height ?? 0;
      case "width": return Math.round((s.width ?? 0) * 10);
      case "bounding": return s.boundingHex ?? 0;
      case "rate": return Math.round((s.rate ?? 0) * 10);
      case "x:ga": return s.cost * s.area;
      case "x:ca": return s.cycles * s.area;
      case "x:gc": return s.cost * s.cycles;
      default: return 0;
    }
  };
  result.sort((a, b) => {
    for (const k of keys) {
      const va = getVal(a, k);
      const vb = getVal(b, k);
      if (va !== vb) return sortOrder.value === "asc" ? va - vb : vb - va;
    }
    return 0;
  });

  return result;
});

const puzzleInfo = computed(() => {
  if (props.records.length === 0) return null;
  const p = props.records[0].puzzle;
  return { id: p.id, name: p.displayName, chapter: p.group.displayName };
});

const selectedRecord = ref<OmRecordDTO | null>(null);

const onKeydown = (e: KeyboardEvent) => { if (e.key === "Escape") selectedRecord.value = null; };
watch(selectedRecord, (v) => {
  if (v) document.addEventListener("keydown", onKeydown);
  else document.removeEventListener("keydown", onKeydown);
});
onUnmounted(() => document.removeEventListener("keydown", onKeydown));

const formatDate = (raw: string | null) => {
  if (!raw) return "—";
  try {
    const d = new Date(raw);
    return d.toLocaleString("zh-CN", { hour12: false });
  } catch {
    return raw;
  }
};

const copyToClipboard = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // fallback
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    document.body.removeChild(ta);
  }
};

const handleHeaderClick = (expr: string) => {
  if (sortExpr.value === expr) {
    sortOrder.value = sortOrder.value === "asc" ? "desc" : "asc";
  } else {
    sortExpr.value = expr;
    sortOrder.value = "asc";
  }
};
</script>

<template>
  <div class="list-wrapper">
    <div class="action-dashboard">
      <div class="dashboard-group">
        <div class="view-mode-selector">
          <button class="mode-btn" :class="{ active: viewMode === 'all' }" @click="viewMode = 'all'">RECORD</button>
          <button class="mode-btn" :class="{ active: viewMode === 'pareto' }" @click="viewMode = 'pareto'">FRONTIER</button>
        </div>
      </div>
      <div class="dashboard-group score-view-group">
        <button class="mode-btn" :class="{ active: scoreView === 'both' }" @click="scoreView = 'both'">@V+∞</button>
        <button class="mode-btn" :class="{ active: scoreView === 'v' }" @click="scoreView = 'v'">@V</button>
        <button class="mode-btn" :class="{ active: scoreView === 'inf' }" @click="scoreView = 'inf'">@∞</button>
      </div>
      <div class="dashboard-group filter-group">
        <label class="filter-checkbox" :class="{ checked: filterTrackless }">
          <input type="checkbox" v-model="filterTrackless" />
          <span>T</span>
        </label>
        <label class="filter-checkbox" :class="{ checked: filterOverlap }">
          <input type="checkbox" v-model="filterOverlap" />
          <span>!O</span>
        </label>
      </div>
      <div class="dashboard-group sort-selector">
        <span class="control-text">SORT:</span>
        <input type="text" v-model="sortExpr" class="sort-input" placeholder="CGA..." title="G/C/A/I/H/W/B/R  S=Sum S4=Sum4  CX=sort by Cycles then G*A" />
        <select v-model="sortOrder">
          <option value="asc">ASC</option>
          <option value="desc">DESC</option>
        </select>
      </div>
    </div>

    <div v-if="puzzleInfo" class="puzzle-header">
      <span class="puzzle-chapter">{{ puzzleInfo.chapter }}</span>
      <span class="puzzle-sep">/</span>
      <span class="puzzle-name">{{ puzzleInfo.name }}</span>
      <span class="puzzle-id">#{{ puzzleInfo.id }}</span>
    </div>

    <div class="matrix-container">
      <table class="matrix-table">
        <thead>
          <tr>
            <th style="width: 18%">CATEGORY</th>
            <th style="width: 40%">SCORE (@V / @∞)</th>
            <th @click="handleHeaderClick('S')" class="sortable">Sum</th>
            <th @click="handleHeaderClick('S4')" class="sortable">Sum4</th>
            <th style="width: 16%">Derived</th>
            <th style="width: 4%"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="item in aggregatedRecords" :key="item.record.id || Math.random()" class="matrix-row" :class="{ pareto: item.isPareto }">
            <td class="category-cell">
              <span class="cat-prefix">{{ item.categories.length > 0 ? item.categories.join(", ") : "—" }}</span>
            </td>
            <td class="score-string">
              <template v-if="item.record.fullFormattedScore">
                <span v-if="scoreView !== 'inf'" class="score-line">{{ splitScore(item.record.fullFormattedScore).v }}</span>
                <span v-if="scoreView !== 'v' && splitScore(item.record.fullFormattedScore).inf" class="score-line score-inf">{{ splitScore(item.record.fullFormattedScore).inf }}</span>
              </template>
              <template v-else>
                <span class="score-full">{{ item.record.score!.cost }}g/{{ item.record.score!.cycles }}c/{{ item.record.score!.area }}a/{{ item.record.score!.instructions }}i<span v-if="item.record.score!.height != null">/{{ item.record.score!.height }}h</span><span v-if="item.record.score!.width != null">/{{ item.record.score!.width }}w</span><span v-if="item.record.score!.boundingHex != null">/{{ item.record.score!.boundingHex }}b</span><span v-if="item.record.score!.trackless">/T</span><span v-if="!item.record.score!.overlap">/L</span></span>
              </template>
            </td>
            <td class="num-val">{{ item.derived.sum }}</td>
            <td class="num-val">{{ item.derived.sum4 }}</td>
            <td class="multiplier-cell">
              <span v-if="xMode === 'ga'" class="m-item">g·a={{ item.derived.ga }}</span>
              <span v-else-if="xMode === 'ca'" class="m-item">c·a={{ item.derived.ca }}</span>
              <span v-else-if="xMode === 'gc'" class="m-item">g·c={{ item.derived.gc }}</span>
              <span v-else class="m-item">g·a={{ item.derived.ga }}</span>
              <span v-if="item.record.score!.rate != null" class="m-item rate-item">r={{ item.record.score!.rate }}</span>
            </td>
            <td class="detail-cell">
              <button class="detail-btn" @click.stop="selectedRecord = item.record" title="Details">+</button>
            </td>
          </tr>
          <tr v-if="aggregatedRecords.length === 0">
            <td colspan="6" class="matrix-empty">NO INTEGRATED RECORDS RETURNED.</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- 详情弹窗 -->
    <div v-if="selectedRecord" class="detail-overlay" @click.self="selectedRecord = null">
      <div class="detail-modal">
        <div class="detail-header">
          <span class="detail-title">RECORD DETAIL</span>
          <button class="detail-close" @click="selectedRecord = null">×</button>
        </div>
        <div class="detail-body">
          <div class="detail-row">
            <span class="detail-label">AUTHOR</span>
            <span class="detail-value">{{ selectedRecord.author || "—" }}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">UPDATED</span>
            <span class="detail-value">{{ formatDate(selectedRecord.lastModified) }}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">GIF</span>
            <code class="detail-link">{{ selectedRecord.gif || "—" }}</code>
            <button class="copy-btn" @click="copyToClipboard(selectedRecord.gif!)" :disabled="!selectedRecord.gif">COPY</button>
          </div>
          <div class="detail-row">
            <span class="detail-label">SOLUTION</span>
            <code class="detail-link">{{ selectedRecord.solution || "—" }}</code>
            <button class="copy-btn" @click="copyToClipboard(selectedRecord.solution!)" :disabled="!selectedRecord.solution">COPY</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.action-dashboard { background-color: #161a22; border: 1px solid #262e3f; padding: 8px 14px; border-radius: 4px; margin-bottom: 12px; display: flex; justify-content: space-between; align-items: center; }
.view-mode-selector { display: flex; background-color: #0a0d14; padding: 2px; border-radius: 4px; border: 1px solid #262e3f; }
.mode-btn { background: none; border: none; color: #4e5d78; padding: 5px 12px; font-family: monospace; font-size: 0.78rem; font-weight: bold; cursor: pointer; border-radius: 3px; }
.mode-btn.active { background-color: #00b4d8; color: #000; }
.control-text { font-family: monospace; font-size: 0.75rem; color: #4e5d78; margin-right: 4px; }
.sort-input { background-color: #0a0d14; color: #e2e8f0; border: 1px solid #262e3f; padding: 4px 8px; border-radius: 4px; font-family: monospace; font-size: 0.75rem; outline: none; width: 80px; }
.sort-input:focus { border-color: #00b4d8; }
.sort-selector select { background-color: #0a0d14; color: #e2e8f0; border: 1px solid #262e3f; padding: 4px 8px; border-radius: 4px; font-family: monospace; font-size: 0.75rem; outline: none; }
.matrix-container { background-color: #121620; border: 1px solid #262e3f; border-radius: 4px; overflow: hidden; }
.matrix-table { width: 100%; border-collapse: collapse; font-family: Consolas, Monaco, monospace; font-size: 0.88rem; text-align: left; }
th, td { padding: 10px 12px; border-bottom: 1px solid #1a1f2c; }
th { background-color: #161a22; color: #4e5d78; font-size: 0.75rem; font-weight: normal; border-bottom: 2px solid #262e3f; }
.sortable { cursor: pointer; user-select: none; } .sortable:hover { background-color: #1a1f2c; color: #fff; } .sortable.active { color: #ffb703; font-weight: bold; }
.matrix-row:hover { background-color: #1a1f2c; } .category-cell { color: #00f5d4; font-weight: bold; font-size: 0.82rem; }
.score-string { color: #e2e8f0; letter-spacing: 0.3px; font-size: 0.82rem; line-height: 1.4; display: flex; flex-direction: column; gap: 2px; }
.score-line { word-break: break-all; }
.score-inf { color: #00b4d8; }
.score-full { word-break: break-all; }
.score-string span { color: #ffb703; font-weight: bold; }
.num-val { color: #00b4d8; } .num-val.sort-highlight { color: #ffb703; font-weight: bold; background-color: rgba(255, 183, 3, 0.03); }
.multiplier-cell { color: #4e5d78; font-size: 0.8rem; display: flex; gap: 10px; } .m-item { background-color: #0a0d14; padding: 2px 6px; border-radius: 2px; border: 1px solid #1a1f2c; }
.puzzle-header { background-color: #121620; border: 1px solid #262e3f; border-bottom: none; border-radius: 4px 4px 0 0; padding: 10px 16px; font-family: monospace; font-size: 0.82rem; display: flex; align-items: center; gap: 6px; }
.puzzle-chapter { color: #4e5d78; }
.puzzle-sep { color: #262e3f; }
.puzzle-name { color: #e2e8f0; font-weight: bold; }
.puzzle-id { color: #00b4d8; margin-left: 8px; font-size: 0.72rem; }


.matrix-empty { text-align: center; color: #4e5d78; padding: 40px; font-style: italic; }

/* ── Dashboard 分区 ── */
.dashboard-group { display: flex; align-items: center; gap: 8px; }
.dashboard-group + .dashboard-group { border-left: 1px solid #262e3f; padding-left: 12px; }

/* ── T/L 过滤器 ── */
.filter-group { gap: 4px; }
.filter-checkbox { display: flex; align-items: center; gap: 3px; cursor: pointer; font-family: monospace; font-size: 0.72rem; color: #4e5d78; padding: 2px 8px; border-radius: 3px; border: 1px solid transparent; user-select: none; }
.filter-checkbox input { display: none; }
.filter-checkbox span { font-weight: bold; }
.filter-checkbox.checked { color: #ffb703; border-color: #ffb703; background: rgba(255, 183, 3, 0.06); }
.filter-checkbox:hover { border-color: #4e5d78; }

/* ── Pareto 前沿标记 ── */

.matrix-row.pareto { background-color: rgba(255, 183, 3, 0.03); }
.matrix-row.pareto:hover { background-color: rgba(255, 183, 3, 0.08); }
/* ── 详情按钮 ── */
.detail-cell { text-align: center; }
.detail-btn { background: none; border: 1px solid #262e3f; color: #4e5d78; font-size: 0.9rem; cursor: pointer; padding: 0 6px; border-radius: 3px; font-family: monospace; line-height: 1; }
.detail-btn:hover { color: #00b4d8; border-color: #00b4d8; }

/* ── 详情弹窗 ── */
.detail-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.7); z-index: 100; display: flex; align-items: center; justify-content: center; }
.detail-modal { width: 560px; max-height: 80vh; font-size: 0.85rem; background: #121620; border: 1px solid #262e3f; border-radius: 6px; padding: 24px; overflow-y: auto; font-family: monospace; box-shadow: 0 8px 32px rgba(0,0,0,0.5); }
.detail-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
.detail-title { color: #00b4d8; font-size: 0.85rem; font-weight: bold; }
.detail-close { background: none; border: 1px solid #262e3f; color: #4e5d78; font-size: 1.2rem; cursor: pointer; padding: 2px 8px; border-radius: 3px; }
.detail-close:hover { color: #ff4a4a; border-color: #ff4a4a; }
.detail-body { display: flex; flex-direction: column; gap: 14px; }
.detail-row { display: flex; flex-wrap: wrap; align-items: flex-start; gap: 6px; }
.detail-label { color: #4e5d78; font-size: 0.78rem; min-width: 70px; }
.detail-value { color: #e2e8f0; font-size: 0.85rem; }
.detail-link { color: #e2e8f0; font-size: 0.8rem; word-break: break-all; flex: 1; min-width: 0; background: #0a0d14; padding: 3px 6px; border-radius: 3px; border: 1px solid #1a1f2c; }
.copy-btn { background: #1a1f2c; border: 1px solid #262e3f; color: #4e5d78; font-size: 0.72rem; padding: 2px 8px; border-radius: 3px; cursor: pointer; font-family: monospace; white-space: nowrap; }
.copy-btn:hover { color: #ffb703; border-color: #ffb703; }
.copy-btn:disabled { opacity: 0.3; cursor: default; }
.copy-btn:disabled:hover { color: #4e5d78; border-color: #262e3f; }
.muted { color: #4e5d78; font-style: italic; }
</style>