// types.ts
export interface Panel {
  id: number;
  content: string;
}

export interface GridArea {
  startCol: number;
  endCol: number;
  startRow: number;
  endRow: number;
}

export interface PanelLayout {
  panel: Panel;
  area: GridArea;
}

export type SplitterType = "vertical" | "horizontal";

export interface Splitter {
  id: string;
  type: SplitterType;
  position: number; // 网格线位置（1-based）
  start: number; // 开始的行/列
  end: number; // 结束的行/列
}
