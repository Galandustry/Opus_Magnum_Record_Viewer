export interface OmGroupDTO {
  id: string;
  displayName: string;
}

export interface OmPuzzleDTO {
  id: string;
  displayName: string;
  type: string;
  group: OmGroupDTO;
  altIds: string[];
}

// src/types/om.ts 修改这一段
export interface OmScoreDTO {
  cost: number;
  cycles: number;
  area: number;
  instructions: number;
  overlap: boolean;
  trackless: boolean;
  height?: number;
  width?: number;
  boundingHex?: number;
  rate?: number;
}

export interface OmRecordDTO {
  id: string | null;
  puzzle: OmPuzzleDTO;
  score: OmScoreDTO | null;
  smartFormattedScore: string | null;
  fullFormattedScore: string | null;
  categoryIds: string[] | null;
  author: string | null;
  gif: string | null;
  solution: string | null;
  lastModified: string | null;
}