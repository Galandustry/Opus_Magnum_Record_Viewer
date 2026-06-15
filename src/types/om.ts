// ==================== ZLBB 基础架构 DTO ====================

export interface OmCollectionDTO {
  id: string;
  displayName: string;
}

export interface OmGroupDTO {
  id: string;
  displayName: string;
  collection?: OmCollectionDTO;
}

export interface OmPuzzleDTO {
  id: string;
  displayName: string;
  type: string;
  group: OmGroupDTO;
  altIds: string[];
}

export interface OmScoreDTO {
  cost: number;
  cycles: number;
  area: number;
  instructions: number;
  overlap: boolean;
  trackless: boolean;
  height: number | null;
  width: number | null;
  boundingHex: number | null;
  rate: number | null;
  areaINFLevel: number | null;
  areaINFValue: number | null;
  heightINF: number | null;
  widthINF: number | null;
  boundingHexINF: number | null;
}

export interface OmRecordDTO {
  id: string | null;
  puzzle: OmPuzzleDTO | null;
  score: OmScoreDTO | null;
  smartFormattedScore: string | null;
  fullFormattedScore: string | null;
  gif: string | null;
  solution: string | null;
  categoryIds: string[] | null;
  smartFormattedCategories: string | null;
  lastModified: string | null;
  author: string | null;
}

// ==================== Pareto 判定面板类型 ====================

export type ParetoJudgeStatus = "Ok" | "Unknown" | "UnknownBreaking" | "AlreadyPresented" | "NothingBeaten";

export interface OmDraftInput {
  cost: number | null; cycles: number | null;
  area: number | null; instructions: number | null;
  overlap: boolean; trackless: boolean;
  activeMetrics: string[];
}

export interface BeatenMetricDiff {
  actualValue: number;
  absoluteDiff: number;    // record_val - draft_val（负数=纪录比你优秀多少）
  percentageDiff: number;  // (draft / record * 100) 劣化百分比
}

export interface ParetoBeatenReport {
  betterRecord: OmRecordDTO;
  costDiff: BeatenMetricDiff | null;
  cyclesDiff: BeatenMetricDiff | null;
  areaDiff: BeatenMetricDiff | null;
  instructionsDiff: BeatenMetricDiff | null;
  heightDiff: BeatenMetricDiff | null;
  widthDiff: BeatenMetricDiff | null;
  bhexDiff: BeatenMetricDiff | null;
  rateDiff: BeatenMetricDiff | null;
}

export interface JudgeResult {
  status: ParetoJudgeStatus;
  totalCompared: number;
  reports: ParetoBeatenReport[];
}

// ==================== 增量同步 ====================

export interface OmRecordChange {
  type: "ADD" | "REMOVE";
  record: OmRecordDTO;
}

export interface SyncResult {
  newCount: number;
  removedCount: number;
  syncedUntil: string;
  errors: string[];
}

export type SubmitStatus = "SUCCESS" | "NOTHING_BEATEN" | "ALREADY_PRESENT" | "FAILURE";

export interface SubmitResult {
  result: SubmitStatus;
  message: string | null;
}
