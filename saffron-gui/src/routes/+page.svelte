<script lang="ts">
  import { onMount } from "svelte";
  // ================= 类型定义 =================
  interface Panel {
    id: string;
    gridColumnStart: number;
    gridColumnEnd: number;
    gridRowStart: number;
    gridRowEnd: number;
  }
  interface GridLine {
    direction: "horizontal" | "vertical";
    positionIndex: number;
    // 网格线索引
    // WARN: start at 1
    start: number;
    end: number;
    adjacentPanels: string[];
  }
  interface PanelCorner {
    panelId: string;
    position: "top-left" | "top-right" | "bottom-left" | "bottom-right";
    columnLine: number;
    rowLine: number;
  }

  // ================= 常量 =================

  const SPLITTERZINDEX: number = 1;
  const CONTAINERZINDEX: number = 2;
  const PANELCORNERZINDEX: number = 3;
  // ================= 状态管理 =================
  // 每一行初始状态所占百分比 WARN: 需要总和为100%
  let columns = $state<number[]>([30, 40, 30]);
  const columnsLength = $derived(columns.length);
  $inspect(columns).with((_type, data) => {
    const tableData = [
      {
        ratios: data.map(
          (value, _index) => `${((value / 100) * 100).toFixed(1)}%`,
        ),
      },
    ];
    console.table(tableData[0]);
  });
  let rows = $state<number[]>([30, 40, 30]);
  const rowsLength = $derived(rows.length);
  $inspect(rows).with((_type, data) => {
    const tableData = [
      {
        ratios: data.map(
          (value, _index) => `${((value / 100) * 100).toFixed(1)}%`,
        ),
      },
    ];
    console.table(tableData[0]);
  });

  // 累积百分比用于定位
  let cumulativeColumns = $derived.by(() => {
    let sum = 0;
    return [0, ...columns.map((value) => (sum += value))];
  });
  let cumulativeRows = $derived.by(() => {
    let sum = 0;
    return [0, ...rows.map((value) => (sum += value))];
  });

  let panelGap = $state<number>(10);

  // WARN: grid is start at 1
  let panels = $state<Panel[]>([
    {
      id: crypto.randomUUID(),
      gridColumnStart: 1,
      gridColumnEnd: 2,
      gridRowStart: 1,
      gridRowEnd: 2,
    },
    {
      id: crypto.randomUUID(),
      gridColumnStart: 2,
      gridColumnEnd: 3,
      gridRowStart: 1,
      gridRowEnd: 2,
    },
    {
      id: crypto.randomUUID(),
      gridColumnStart: 3,
      gridColumnEnd: 4,
      gridRowStart: 1,
      gridRowEnd: 3, // 竖直延伸两个单位
    },
    {
      id: crypto.randomUUID(),
      gridColumnStart: 1,
      gridColumnEnd: 3, // 水平跨越两个单位
      gridRowStart: 2,
      gridRowEnd: 3,
    },
    {
      id: crypto.randomUUID(),
      gridColumnStart: 1,
      gridColumnEnd: 2,
      gridRowStart: 3,
      gridRowEnd: 4,
    },
    {
      id: crypto.randomUUID(),
      gridColumnStart: 2,
      gridColumnEnd: 4, // 水平跨越两个单位
      gridRowStart: 3,
      gridRowEnd: 4,
    },
  ]);

  let panelCorners = $derived.by<PanelCorner[]>(() => {
    return panels.flatMap((panel) => getPanelCorners(panel));
  });

  let gridLines = $state<GridLine[]>([]);
  $inspect(gridLines).with((type, data) => {
    if (type == "update") {
      console.table(data);
    }
  });

  // ================ 函数定义 ================

  // 动态计算分隔线位置 WARN: 假定命名为 panels
  function findGridLines(): GridLine[] {
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
  $effect(() => {
    if (columnsLength && rowsLength) {
      console.log("获得更新");
      gridLines = findGridLines();
    }
  });

  function rangeSum(arr: Array<number>, start: number, end: number) {
    let sum = 0;

    for (let i = start; i < end; i++) {
      sum += arr[i];
    }

    return sum;
  }

  function getPanelCorners(panel: Panel): PanelCorner[] {
    return [
      {
        panelId: panel.id,
        position: "top-left",
        columnLine: panel.gridColumnStart,
        rowLine: panel.gridRowStart,
      },
      {
        panelId: panel.id,
        position: "top-right",
        columnLine: panel.gridColumnEnd,
        rowLine: panel.gridRowStart,
      },
      {
        panelId: panel.id,
        position: "bottom-left",
        columnLine: panel.gridColumnStart,
        rowLine: panel.gridRowEnd,
      },
      {
        panelId: panel.id,
        position: "bottom-right",
        columnLine: panel.gridColumnEnd,
        rowLine: panel.gridRowEnd,
      },
    ];
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

  onMount(() => {
    // gridLines = findGridLines();
  });
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
  style:pointer-events="none"
  style:z-index={CONTAINERZINDEX}
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
      style:background="#2d2d2d"
      style:border="1px solid #444"
      style:position="relative"
      style:border-radius="10px"
    ></div>
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
      style:left="{cumulativeColumns[gridLine.positionIndex - 1]}%"
      style:height="{rangeSum(rows, gridLine.start - 1, gridLine.end - 1)}%"
      style:top="{((gridLine.start - 1) / rows.length) * 100}%"
      style:z-index={SPLITTERZINDEX}
      onmousedown={(e) => startDrag("column", gridLine.positionIndex, e)}
    ></button>
  {:else}
    <button
      type="button"
      aria-label="水平分隔线"
      class="splitter"
      data-orientation="horizontal"
      style:top="{cumulativeRows[gridLine.positionIndex - 1]}%"
      style:width="{rangeSum(columns, gridLine.start - 1, gridLine.end - 1)}%"
      style:left="{((gridLine.start - 1) / columns.length) * 100}%"
      style:z-index={SPLITTERZINDEX}
      onmousedown={(e) => startDrag("row", gridLine.positionIndex, e)}
    ></button>
  {/if}
{/each}
{#each panelCorners as panelCorner}
  {#if panelCorner.position === "top-left"}
    <button
      style:position="fixed"
      style:background-color="red"
      style:left="{cumulativeColumns[panelCorner.columnLine - 1]}%"
      style:top="{cumulativeRows[panelCorner.rowLine - 1]}%"
      style:z-index={PANELCORNERZINDEX}
    >
      左上
    </button>
  {:else if panelCorner.position === "top-right"}
    <button
      style:position="fixed"
      style:background-color="purple"
      style:left="{cumulativeColumns[panelCorner.columnLine - 1]}%"
      style:top="{cumulativeRows[panelCorner.rowLine - 1]}%"
      style:transform="translateX(-100%)"
      style:z-index={PANELCORNERZINDEX}
    >
      右上
    </button>
  {:else if panelCorner.position === "bottom-left"}
    <button
      style:position="fixed"
      style:background-color="lime"
      style:left="{cumulativeColumns[panelCorner.columnLine - 1]}%"
      style:top="{cumulativeRows[panelCorner.rowLine - 1]}%"
      style:transform="translateY(-100%)"
      style:z-index={PANELCORNERZINDEX}
    >
      左下
    </button>
  {:else if panelCorner.position === "bottom-right"}
    <button
      style:position="fixed"
      style:background-color="blue"
      style:left="{cumulativeColumns[panelCorner.columnLine - 1]}%"
      style:top="{cumulativeRows[panelCorner.rowLine - 1]}%"
      style:transform="translate(-100%, -100%)"
      style:z-index={PANELCORNERZINDEX}
    >
      右下
    </button>
  {/if}
{/each}

<style>
  .splitter {
    position: absolute;
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
