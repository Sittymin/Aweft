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
    type: "horizontal" | "vertical";
    index: number;
    // 添加范围属性
    start: number; // 起始网格线索引（从1开始）
    end: number; // 结束网格线索引（从1开始）
    adjacentPanels: string[];
  }

  // ================= 状态管理 =================
  // 每一行初始状态所占百分比
  let columns: number[] = $state([50, 50]);
  let rows: number[] = $state([50, 50]);
  let gridGap = $state(10);

  // 网格边缘是从 1 开始的
  let panels: Panel[] = [
    {
      id: "panel1",
      gridColumnStart: 1,
      gridColumnEnd: 2,
      gridRowStart: 1,
      gridRowEnd: 2,
      content: "Left Top",
    },
    {
      id: "panel2",
      gridColumnStart: 1,
      gridColumnEnd: 2,
      gridRowStart: 2,
      gridRowEnd: 3,
      content: "Left Bottom",
    },
    {
      id: "panel3",
      gridColumnStart: 2,
      gridColumnEnd: 3,
      gridRowStart: 1,
      gridRowEnd: 3,
      content: "Right Full",
    },
  ];

  // 动态计算分隔线位置
  function findGridLines(panels: Panel[]): GridLine[] {
    const gridLines: GridLine[] = [];
    const processedLines = new Set<string>();

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
          if (!processedLines.has(key)) {
            // 计算这条垂直线的行范围
            const rowStart = Math.min(panel1.gridRowStart, panel2.gridRowStart);
            const rowEnd = Math.max(panel1.gridRowEnd, panel2.gridRowEnd);

            gridLines.push({
              type: "vertical",
              index: panel1.gridColumnEnd,
              start: rowStart, // 行范围开始
              end: rowEnd, // 行范围结束
              adjacentPanels: [panel1.id, panel2.id],
            });
            processedLines.add(key);
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
          if (!processedLines.has(key)) {
            // 计算这条水平线的列范围
            const colStart = Math.min(
              panel1.gridColumnStart,
              panel2.gridColumnStart,
            );
            const colEnd = Math.max(panel1.gridColumnEnd, panel2.gridColumnEnd);

            gridLines.push({
              type: "horizontal",
              index: panel1.gridRowEnd,
              start: colStart, // 列范围开始
              end: colEnd, // 列范围结束
              adjacentPanels: [panel1.id, panel2.id],
            });
            processedLines.add(key);
          }
        }
      }
    }

    return gridLines;
  }
  const gridLines = findGridLines(panels);
  //
  // [
  //  {
  //    type: 'horizontal',
  //    index: 2,             // 第2条水平网格线
  //    start: 1,            // 从第1列开始
  //    end: 2,              // 到第2列结束
  //    adjacentPanels: ['panel1', 'panel2']
  //  },
  //  {
  //    type: 'vertical',
  //    index: 2,             // 第2条垂直网格线
  //    start: 1,            // 从第1行开始
  //    end: 3,              // 到第3行结束
  //    adjacentPanels: ['panel1', 'panel2', 'panel3']
  //  }
  //]
  //
  function sumToIndex(arr: number[], index: number): number {
    let sum = 0;
    for (let i = 0; i < index; i++) {
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

<div
  class="container"
  style:grid-template-columns="{columns.join('% ')}%"
  style:grid-template-rows="{rows.join('% ')}%"
  style:gap="{gridGap}px"
>
  {#each panels as panel (panel.id)}
    <div
      class="panel"
      style:grid-column-start={panel.gridColumnStart}
      style:grid-column-end={panel.gridColumnEnd}
      style:grid-row-start={panel.gridRowStart}
      style:grid-row-end={panel.gridRowEnd}
    >
      {panel.content}
    </div>
  {/each}

  <!-- 动态生成垂直分隔线 -->
  {#each gridLines as line}
    {#if line.type === "vertical"}
      <button
        type="button"
        aria-label="垂直分隔线"
        class="splitter"
        data-orientation="vertical"
        style:left="{sumToIndex(columns, line.index - 1)}%"
        style:height="{((line.end - line.start) / rows.length) * 100}%"
        style:top="{((line.start - 1) / rows.length) * 100}%"
        onmousedown={(e) => startDrag("column", line.index, e)}
      ></button>
    {:else}
      <button
        type="button"
        aria-label="水平分隔线"
        class="splitter"
        data-orientation="horizontal"
        style:top="{sumToIndex(rows, line.index - 1)}%"
        style:width="{((line.end - line.start) / columns.length) * 100}%"
        style:left="{((line.start - 1) / columns.length) * 100}%"
        onmousedown={(e) => startDrag("row", line.index, e)}
      ></button>
    {/if}
  {/each}
</div>

<style>
  .container {
    display: grid;
    position: fixed; /* 不受父标签影响 */
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
  }

  .panel {
    background: #2d2d2d;
    border: 1px solid #444;
    position: relative;
    overflow: hidden;
  }

  .splitter {
    position: absolute;
    z-index: 1;
    transition: background-color 0.2s;
    appearance: none;
    -webkit-appearance: none;
    padding: 0;
    margin: 0;
    border: none;
    background: blue;
    transform: translate(-50%, -50%);

    &[data-orientation="vertical"] {
      width: 5px;
      height: 100%;
      cursor: col-resize;
    }

    &[data-orientation="horizontal"] {
      height: 5px;
      width: 100%;
      cursor: row-resize;
    }

    &:hover {
      background: green;
    }
    &:active {
      background: black;
    }
  }
</style>
