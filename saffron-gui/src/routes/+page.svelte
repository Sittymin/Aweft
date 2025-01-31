<!-- TODO: 规范网格轨道index与行列index 与修复网格控制条长度更新 -->
<script lang="ts">
  // ================= 类型定义 =================
  type Panel = {
    id: string;
    gridColumnStart: number;
    gridColumnEnd: number;
    gridRowStart: number;
    gridRowEnd: number;
    content: string;
  };
  interface GridLine {
    direction: "horizontal" | "vertical";
    positionIndex: number;
    // 网格线索引
    // WARN: start at 1
    start: number;
    end: number;
    adjacentPanels: string[];
  }

  // ================= 状态管理 =================
  // 每一行初始状态所占百分比 WARN: 需要总和为100%
  let columns: number[] = $state([30, 40, 30]);
  let rows: number[] = $state([30, 40, 30]);
  let panelGap: number = $state(10);

  // WARN: grid is start at 1
  let panels: Panel[] = [
    {
      id: "topLeft",
      gridColumnStart: 1,
      gridColumnEnd: 2,
      gridRowStart: 1,
      gridRowEnd: 2,
      content: "Top Left",
    },
    {
      id: "topMiddle",
      gridColumnStart: 2,
      gridColumnEnd: 3,
      gridRowStart: 1,
      gridRowEnd: 2,
      content: "Top Middle",
    },
    {
      id: "topRight",
      gridColumnStart: 3,
      gridColumnEnd: 4,
      gridRowStart: 1,
      gridRowEnd: 3, // 竖直延伸两个单位
      content: "Top Right",
    },
    {
      id: "middleLeft",
      gridColumnStart: 1,
      gridColumnEnd: 3, // 水平跨越两个单位
      gridRowStart: 2,
      gridRowEnd: 3,
      content: "Middle Left",
    },
    {
      id: "bottomLeft",
      gridColumnStart: 1,
      gridColumnEnd: 2,
      gridRowStart: 3,
      gridRowEnd: 4,
      content: "Bottom Left",
    },
    {
      id: "bottomMiddle",
      gridColumnStart: 2,
      gridColumnEnd: 4, // 水平跨越两个单位
      gridRowStart: 3,
      gridRowEnd: 4,
      content: "Bottom Middle & Right",
    },
  ];

  // 动态计算分隔线位置
  function findGridLines(panels: Panel[]): GridLine[] {
    const gridLines: GridLine[] = [];
    const processedLines = new Map<string, GridLine>();

    for (let i = 0; i < panels.length; i++) {
      for (let j = i + 1; j < panels.length; j++) {
        const panel1 = panels[i];
        const panel2 = panels[j];

        // 检查垂直相邻
        if (
          panel1.gridColumnEnd === panel2.gridColumnStart &&
          ((panel1.gridRowStart < panel2.gridRowEnd &&
            panel1.gridRowEnd > panel2.gridRowStart) ||
            (panel2.gridRowStart < panel1.gridRowEnd &&
              panel2.gridRowEnd > panel1.gridRowStart))
        ) {
          const key = `v${panel1.gridColumnEnd}`;
          const existingLine = processedLines.get(key);

          if (!existingLine) {
            // 创建新线
            const rowStart = Math.min(panel1.gridRowStart, panel2.gridRowStart);
            const rowEnd = Math.max(panel1.gridRowEnd, panel2.gridRowEnd);
            const newLine: GridLine = {
              direction: "vertical",
              positionIndex: panel1.gridColumnEnd,
              start: rowStart,
              end: rowEnd,
              adjacentPanels: [panel1.id, panel2.id],
            };
            gridLines.push(newLine);
            processedLines.set(key, newLine);
          } else {
            // 合并现有线
            existingLine.start = Math.min(
              existingLine.start,
              panel1.gridRowStart,
              panel2.gridRowStart,
            );
            existingLine.end = Math.max(
              existingLine.end,
              panel1.gridRowEnd,
              panel2.gridRowEnd,
            );
            // 添加新的相邻面板ID（去重）
            if (!existingLine.adjacentPanels.includes(panel1.id)) {
              existingLine.adjacentPanels.push(panel1.id);
            }
            if (!existingLine.adjacentPanels.includes(panel2.id)) {
              existingLine.adjacentPanels.push(panel2.id);
            }
          }
        }

        // 检查水平相邻
        if (
          panel1.gridRowEnd === panel2.gridRowStart &&
          ((panel1.gridColumnStart < panel2.gridColumnEnd &&
            panel1.gridColumnEnd > panel2.gridColumnStart) ||
            (panel2.gridColumnStart < panel1.gridColumnEnd &&
              panel2.gridColumnEnd > panel1.gridColumnStart))
        ) {
          const key = `h${panel1.gridRowEnd}`;
          const existingLine = processedLines.get(key);

          if (!existingLine) {
            // 创建新线
            const colStart = Math.min(
              panel1.gridColumnStart,
              panel2.gridColumnStart,
            );
            const colEnd = Math.max(panel1.gridColumnEnd, panel2.gridColumnEnd);
            const newLine: GridLine = {
              direction: "horizontal",
              positionIndex: panel1.gridRowEnd,
              start: colStart,
              end: colEnd,
              adjacentPanels: [panel1.id, panel2.id],
            };
            gridLines.push(newLine);
            processedLines.set(key, newLine);
          } else {
            // 合并现有线
            existingLine.start = Math.min(
              existingLine.start,
              panel1.gridColumnStart,
              panel2.gridColumnStart,
            );
            existingLine.end = Math.max(
              existingLine.end,
              panel1.gridColumnEnd,
              panel2.gridColumnEnd,
            );
            // 添加新的相邻面板ID（去重）
            if (!existingLine.adjacentPanels.includes(panel1.id)) {
              existingLine.adjacentPanels.push(panel1.id);
            }
            if (!existingLine.adjacentPanels.includes(panel2.id)) {
              existingLine.adjacentPanels.push(panel2.id);
            }
          }
        }
      }
    }

    return gridLines;
  }
  const gridLines = findGridLines(panels);
  console.table(gridLines);

  function rangeSum(arr: Array<number>, start: number, end: number) {
    let sum = 0;

    for (let i = start; i < end; i++) {
      sum += arr[i];
    }

    return sum;
  }
  // ================= 拖拽逻辑 =================
  type DragState = {
    type: "column" | "row";
    index: number;
    startX: number;
    startY: number;
    startSize: number[];
  };

  let currentDragState: DragState | null = null;

  function startDrag(type: "column" | "row", index: number, event: MouseEvent) {
    event.preventDefault();

    // 存储初始状态
    currentDragState = {
      type,
      index,
      startX: event.clientX,
      startY: event.clientY,
      startSize: type === "column" ? [...columns] : [...rows],
    };

    // 添加事件监听
    window.addEventListener("mousemove", handleDragMove);
    window.addEventListener("mouseup", stopDrag, { once: true }); // 使用 once 选项

    // 设置样式
    document.body.style.cursor =
      type === "column" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";
  }

  function handleDragMove(event: MouseEvent) {
    if (!currentDragState) return;

    const container = document.querySelector(".container")!;
    const rect = container.getBoundingClientRect();
    const minSize = 10;

    if (currentDragState.type === "column") {
      const dx = event.clientX - currentDragState.startX;
      const containerWidth = rect.width;
      const deltaPercent = (dx / containerWidth) * 100;

      // 创建新数组而不是修改现有数组
      const newColumns = [...currentDragState.startSize];
      const idx = currentDragState.index - 2;

      if (
        idx >= 0 &&
        idx + 1 < newColumns.length &&
        newColumns[idx] + deltaPercent >= minSize &&
        newColumns[idx + 1] - deltaPercent >= minSize
      ) {
        // 确保数值精确
        newColumns[idx] = +(
          currentDragState.startSize[idx] + deltaPercent
        ).toFixed(2);
        newColumns[idx + 1] = +(
          currentDragState.startSize[idx + 1] - deltaPercent
        ).toFixed(2);

        // 强制更新状态
        columns = newColumns;
      }
      console.log("Drag Column Update:", {
        type: currentDragState.type,
        delta: deltaPercent,
      });
    } else {
      const dy = event.clientY - currentDragState.startY;
      const containerHeight = rect.height;
      const deltaPercent = (dy / containerHeight) * 100;

      const newRows = [...currentDragState.startSize];
      const idx = currentDragState.index - 2;

      if (
        idx >= 0 &&
        idx + 1 < newRows.length &&
        newRows[idx] + deltaPercent >= minSize &&
        newRows[idx + 1] - deltaPercent >= minSize
      ) {
        newRows[idx] = +(
          currentDragState.startSize[idx] + deltaPercent
        ).toFixed(2);
        newRows[idx + 1] = +(
          currentDragState.startSize[idx + 1] - deltaPercent
        ).toFixed(2);

        // 强制更新状态
        rows = newRows;
      }
      console.log("Drag Row Update:", {
        type: currentDragState.type,
        delta: deltaPercent,
      });
    }
  }
  function stopDrag() {
    currentDragState = null;
    // 恢复鼠标样式
    document.body.style.cursor = "";
    document.body.style.userSelect = "";

    window.removeEventListener("mousemove", handleDragMove);
    window.removeEventListener("mouseup", stopDrag);
  }
</script>

<!-- 由于内联样式优先级高，所以可能内嵌的 style 不会被应用 -->
<!-- 将网格容器不获取鼠标事件来让下面的分割线可以被点到 -->
<div
  class="container"
  style:position="fixed"
  style:display="grid"
  style:grid-template-columns="{columns.join('% ')}%"
  style:grid-template-rows="{rows.join('% ')}%"
  style:top="0"
  style:left="0"
  style:right="0"
  style:bottom="0"
  style:overflow="auto"
  style:pointer-events="none"
>
  {#each panels as panel (panel.id)}
    <div
      class="panel"
      style:grid-column-start={panel.gridColumnStart}
      style:grid-column-end={panel.gridColumnEnd}
      style:grid-row-start={panel.gridRowStart}
      style:grid-row-end={panel.gridRowEnd}
      style:pointer-events="auto"
      style:margin="{panelGap}px"
    >
      {panel.content}
    </div>
  {/each}
</div>

<!-- 动态生成垂直分隔线 -->
{#each gridLines as gridLine}
  {#if gridLine.direction === "vertical"}
    <button
      type="button"
      aria-label="垂直分隔线"
      class="splitter"
      data-orientation="vertical"
      style:left="{rangeSum(columns, 0, gridLine.positionIndex - 1)}%"
      style:height="{rangeSum(rows, gridLine.start - 1, gridLine.end - 1)}%"
      style:top="{((gridLine.start - 1) / rows.length) * 100}%"
      onmousedown={(e) => startDrag("column", gridLine.positionIndex, e)}
    ></button>
  {:else}
    <button
      type="button"
      aria-label="水平分隔线"
      class="splitter"
      data-orientation="horizontal"
      style:top="{rangeSum(rows, 0, gridLine.positionIndex - 1)}%"
      style:width="{rangeSum(columns, gridLine.start - 1, gridLine.end - 1)}%"
      style:left="{((gridLine.start - 1) / columns.length) * 100}%"
      onmousedown={(e) => startDrag("row", gridLine.positionIndex, e)}
    ></button>
  {/if}
{/each}

<style>
  .panel {
    background: #2d2d2d;
    border: 1px solid #444;
    position: relative;
    border-radius: 10px;
    overflow: hidden;
  }

  .splitter {
    position: absolute;
    z-index: -1;
    transition: background-color 0.2s;
    appearance: none;
    -webkit-appearance: none;
    padding: 0;
    margin: 0;
    border: none;
    background: blue;

    &[data-orientation="vertical"] {
      width: 5px;
      height: 100%;
      cursor: col-resize;
      transform: translateX(-50%);
    }

    &[data-orientation="horizontal"] {
      height: 5px;
      width: 100%;
      cursor: row-resize;
      transform: translateY(-50%);
    }

    &:hover {
      background: green;
    }
    &:active {
      background: black;
    }
  }
</style>
