<script lang="ts">
  // ================= 类型定义 =================
  type Panel = {
    id: string;
    gridColumn: string;
    gridRow: string;
    content: string;
  };

  // ================= 状态管理 =================
  // 每一行初始状态所占百分比
  let columns: number[] = $state([50, 50]);
  let rows: number[] = $state([50, 50]);

  let panels: Panel[] = [
    { id: "panel1", gridColumn: "1", gridRow: "1", content: "Left Top" },
    { id: "panel2", gridColumn: "1", gridRow: "2", content: "Left Bottom" },
    {
      id: "panel3",
      gridColumn: "2",
      gridRow: "1 / span 2",
      content: "Right Full",
    },
  ];

  // 动态计算分隔线位置
  let verticalSeparatorPositions = $derived(
    columns
      .slice(0, -1)
      .map((_, i) => columns.slice(0, i + 1).reduce((a, b) => a + b, 0)),
  );

  let horizontalSeparatorPositions = $derived(
    rows
      .slice(0, -1)
      .map((_, j) => rows.slice(0, j + 1).reduce((a, b) => a + b, 0)),
  );

  // ================= 拖拽逻辑 =================
  type DragState = {
    type: "column" | "row";
    index: number;
    startX: number;
    startY: number;
    startSize: number[];
  };

  let dragState: DragState | null = null;

  function startDrag(type: "column" | "row", index: number, event: MouseEvent) {
    event.preventDefault();
    dragState = {
      type,
      index,
      startX: event.clientX,
      startY: event.clientY,
      startSize: type === "column" ? [...columns] : [...rows],
    };

    document.body.style.cursor =
      type === "column" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";

    window.addEventListener("mousemove", handleDragMove);
    window.addEventListener("mouseup", stopDrag);
  }

  function handleDragMove(event: MouseEvent) {
    if (!dragState) return;

    const container = document.querySelector(".container")!;
    const { width, height } = container.getBoundingClientRect();

    if (dragState.type === "column") {
      const delta = ((event.clientX - dragState.startX) / width) * 100;
      const newColumns = [...dragState.startSize];

      // 调整相邻两列
      newColumns[dragState.index - 1] += delta;
      newColumns[dragState.index] -= delta;

      if (newColumns.every((c) => c >= 10)) {
        columns = newColumns;
      }
    } else {
      const delta = ((event.clientY - dragState.startY) / height) * 100;
      const newRows = [...dragState.startSize];

      // 调整相邻两行
      newRows[dragState.index - 1] += delta;
      newRows[dragState.index] -= delta;

      if (newRows.every((r) => r >= 10)) {
        rows = newRows;
      }
    }
  }

  function stopDrag() {
    dragState = null;
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    window.removeEventListener("mousemove", handleDragMove);
    window.removeEventListener("mouseup", stopDrag);
  }
</script>

<div
  class="container"
  style="grid-template-columns: {columns.join('% ')}%;
         grid-template-rows: {rows.join('% ')}%;"
>
  {#each panels as panel (panel.id)}
    <div
      class="panel"
      style="grid-column: {panel.gridColumn};
             grid-row: {panel.gridRow};"
    >
      {panel.content}
    </div>
  {/each}

  <!-- 动态生成垂直分隔线 -->
  {#each verticalSeparatorPositions as pos, i}
    <button
      type="button"
      aria-label="垂直分隔线"
      class="splitter"
      data-orientation="vertical"
      style="left: calc({pos}%);"
      on:mousedown={(e) => startDrag("column", i + 1, e)}
    ></button>
  {/each}

  <!-- 动态生成水平分隔线 -->
  {#each horizontalSeparatorPositions as pos, j}
    <button
      type="button"
      aria-label="水平分隔线"
      class="splitter"
      data-orientation="horizontal"
      style="top: calc({pos}%);"
      on:mousedown={(e) => startDrag("row", j + 1, e)}
    ></button>
  {/each}
</div>

<style>
  .container {
    display: grid;
    width: 100vw;
    height: 100vh;
    position: relative;
    gap: 15px;
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
