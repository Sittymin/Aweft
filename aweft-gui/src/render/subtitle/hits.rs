//! 字形块 → 每行一个框：视口能点中字幕、能把某一行拖走，靠的就是这一份几何。
//!
//! libass 只给一张扁平的字形块列表，块上没有「属于哪一行」。这里把它还原成行框，
//! 依据是两条都已核实的事实：
//!
//! 1. **块按行成段出现**：libass 先按 `(Layer, ReadOrder)` 排事件，再把每个事件的块
//!    连续接进列表（`ass_render.c` 的 `qsort` + 尾部拼接）。
//! 2. **同一层内的行不重叠**：参与碰撞避让的事件会被 `fix_collisions` 挤开（整块一起挤，
//!    多行 `\N` 也当一个整体），所以它们的块只会挨着、不会交叠。
//!
//! 于是「按矩形相交把块并成不相交的簇」就给出了与脚本里 `Dialogue` 同序的行框，
//! 顺序配上 [`aweft_convert::LineHit`] 即得每行的身份。
//!
//! 两处例外由 `LineHit::anchor`（也就是 `\pos`）兜着：
//! - 带 `\pos` 或 `\move` 的事件**豁免碰撞避让**（libass 在那里直接置 `detect_collisions = 0`），
//!   用户把两行拖到重叠时两簇会并成一簇，簇数与行数不再相等；
//! - 某一行完全在画面外时不产出任何块，也会少一簇。
//! 这两种情况下不再按顺序硬配，改为「每个簇找离它锚点最近、还没被占用的行」：锚点全同
//! （没拖过的行都在同一个底部中点）时退化成按序配，正好等于 libass 的挤压次序。
//!
//! 单位：块坐标是**光栅像素**（libass 的渲染帧尺寸），返回值换回**排版像素**——
//! 后者才是视口的世界坐标，也是 `SetPosition` 写进样式链的单位。

use aweft_ass::render::{Tile, TileKind};
use aweft_convert::LineHit;
use aweft_model::ids::ItemId;

/// 一块字形的几何，光栅像素。从 [`Tile`] 里只抽这几个字段：命中判定只看矩形，
/// 位图与颜色合成器才用得上；抽出来这一层，下面的聚类才测得到。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TileRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub kind: TileKind,
}

impl TileRect {
    pub(crate) fn of(tile: &Tile) -> Self {
        Self {
            x: f64::from(tile.x),
            y: f64::from(tile.y),
            w: tile.width as f64,
            h: tile.height as f64,
            kind: tile.kind,
        }
    }
}

/// 一行的包围框，排版像素（= 视口世界坐标）。拖动只在这份框上平移，提交写的是基线的
/// 增量，所以这里不必再留 `\pos` 那个锚点。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LineBox {
    pub item_id: ItemId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 一个不相交的矩形簇，光栅像素。
#[derive(Debug, Clone, Copy)]
struct Cluster {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Cluster {
    fn touches(&self, other: &Self) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    fn unite(&mut self, other: &Self) {
        self.x0 = self.x0.min(other.x0);
        self.y0 = self.y0.min(other.y0);
        self.x1 = self.x1.max(other.x1);
        self.y1 = self.y1.max(other.y1);
    }
}

/// 把本帧的字形块配成每行的框，顺序 = libass 画它们的顺序。
///
/// `layout` 是排版尺寸（脚本的 PlayRes 基准），`raster` 是这一次 libass 实际用的渲染帧
/// 尺寸；两者之比就是块的坐标要除回去的倍率。
pub(crate) fn line_boxes(
    tiles: &[TileRect],
    hits: &[LineHit],
    now_ms: i64,
    layout: (u32, u32),
    raster: (u32, u32),
) -> Vec<LineBox> {
    let clusters = cluster_tiles(tiles);
    let visible = visible_lines(hits, now_ms);
    // 光栅 → 排版：两轴各自换算（`raster_size` 逐边取整，倍率严格说只近似相等）
    let sx = f64::from(raster.0.max(1)) / f64::from(layout.0.max(1));
    let sy = f64::from(raster.1.max(1)) / f64::from(layout.1.max(1));
    assign(&clusters, &visible, layout)
        .into_iter()
        .map(|(cluster_index, line_index)| {
            let cluster = clusters[cluster_index];
            let line = visible[line_index];
            let (x, y) = (cluster.x0 / sx, cluster.y0 / sy);
            let (w, h) = (
                (cluster.x1 - cluster.x0) / sx,
                (cluster.y1 - cluster.y0) / sy,
            );
            LineBox {
                item_id: line.hit.item_id,
                x,
                y,
                w,
                h,
            }
        })
        .collect()
}

/// 按矩形相交把块并成簇，保持「簇的首块出现顺序」——那就是 libass 的事件顺序。
fn cluster_tiles(tiles: &[TileRect]) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    for tile in tiles {
        // 阴影块带着固定偏移（`\3cXy` 那个投影），算进去会让框朝阴影方向虚出一截
        if tile.kind == TileKind::Shadow || tile.w <= 0.0 || tile.h <= 0.0 {
            continue;
        }
        let block = Cluster {
            x0: tile.x,
            y0: tile.y,
            x1: tile.x + tile.w,
            y1: tile.y + tile.h,
        };
        // 与新块相交的簇可能有好几个（它们此前互不相交，现在经由新块连成一片）：
        // 全部并进第一个，其余从列表里摘掉。摘与并都保住首块的位置，顺序不乱。
        match clusters.iter().position(|c| c.touches(&block)) {
            Some(first) => {
                for index in (first + 1..clusters.len()).rev() {
                    if clusters[index].touches(&block) {
                        let merged = clusters.remove(index);
                        clusters[first].unite(&merged);
                    }
                }
                clusters[first].unite(&block);
            }
            None => clusters.push(block),
        }
    }
    clusters
}

#[derive(Debug, Clone, Copy)]
struct VisibleLine<'a> {
    hit: &'a LineHit,
    /// 在脚本里的下标 = libass 的 ReadOrder
    order: i64,
}

impl VisibleLine<'_> {
    /// 这一行锚点应在哪儿：`\pos` 写的值，没写就是底部中点（边距为 0 的 BottomCenter）。
    fn anchor(&self, layout: (u32, u32)) -> (f64, f64) {
        self.hit
            .anchor
            .unwrap_or((f64::from(layout.0) / 2.0, f64::from(layout.1)))
    }
}

/// 此刻在画面上的行，按 libass 的排序键 `(layer, 脚本里的先后)` 排好。
/// 一行在同一时刻只有一个段在生效（段把行的时长切成不重叠的区间），所以这里
/// 每个 `item_id` 至多出现一次。
fn visible_lines(hits: &[LineHit], now_ms: i64) -> Vec<VisibleLine<'_>> {
    let mut visible: Vec<VisibleLine> = hits
        .iter()
        .enumerate()
        .filter(|(_, hit)| hit.start_ms <= now_ms && now_ms < hit.end_ms)
        .map(|(order, hit)| VisibleLine {
            hit,
            order: order as i64,
        })
        .collect();
    visible.sort_by_key(|line| (line.hit.layer, line.order));
    visible
}

/// 簇 ↔ 生效行的配对，返回下标对。
///
/// 数量相等时按序配（模块头那两条事实都成立，这是常态）；不等时逐簇取锚点最近的未占用行。
fn assign(clusters: &[Cluster], visible: &[VisibleLine], layout: (u32, u32)) -> Vec<(usize, usize)> {
    if clusters.len() == visible.len() {
        return (0..clusters.len()).map(|index| (index, index)).collect();
    }
    let mut free: Vec<usize> = (0..visible.len()).collect();
    let mut pairs = Vec::with_capacity(clusters.len());
    for (index, cluster) in clusters.iter().enumerate() {
        // 簇的底部中点与行的锚点同一单位（光栅像素）；两轴的缩放倍率近似相同，
        // 比距离够用，省掉一次换算。
        let (center_x, bottom_y) = (
            (cluster.x0 + cluster.x1) / 2.0,
            cluster.y1,
        );
        let best = free
            .iter()
            .enumerate()
            .map(|(slot, line_index)| {
                let (ax, ay) = visible[*line_index].anchor(layout);
                let (dx, dy) = (center_x - ax, bottom_y - ay);
                (dx * dx + dy * dy, slot)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, slot)) = best else {
            break;
        };
        pairs.push((index, free.remove(slot)));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> TileRect {
        TileRect {
            x,
            y,
            w,
            h,
            kind: TileKind::Glyph,
        }
    }

    fn hit(item_id: ItemId) -> LineHit {
        LineHit {
            item_id,
            layer: 0,
            start_ms: 0,
            end_ms: 1_000,
            anchor: None,
        }
    }

    const LAYOUT: (u32, u32) = (1920, 1080);
    const RASTER: (u32, u32) = (1920, 1080);

    /// 同一行的多个块并成一个框；两块挨着但不相交的（相邻两行）不并。
    #[test]
    fn tiles_of_one_line_unite_and_stay_separate_across_lines() {
        let (first, second) = (ItemId::new(), ItemId::new());
        let tiles = vec![
            rect(100.0, 100.0, 20.0, 20.0),
            rect(110.0, 105.0, 30.0, 10.0),
            rect(90.0, 115.0, 10.0, 10.0),
            rect(400.0, 300.0, 20.0, 20.0),
        ];
        let boxes = line_boxes(&tiles, &[hit(first), hit(second)], 500, LAYOUT, RASTER);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].item_id, first);
        assert_eq!(
            (boxes[0].x, boxes[0].y, boxes[0].w, boxes[0].h),
            (90.0, 100.0, 50.0, 25.0)
        );
        assert_eq!(boxes[1].item_id, second);
        assert_eq!((boxes[1].x, boxes[1].y), (400.0, 300.0));
    }

    /// 阴影块不算进框。
    #[test]
    fn shadow_tiles_are_left_out() {
        let first = ItemId::new();
        let mut shadow = rect(106.0, 106.0, 20.0, 20.0);
        shadow.kind = TileKind::Shadow;
        let tiles = vec![rect(100.0, 100.0, 20.0, 20.0), shadow];
        let boxes = line_boxes(&tiles, &[hit(first)], 500, LAYOUT, RASTER);
        assert_eq!(boxes.len(), 1);
        assert_eq!(
            (boxes[0].x, boxes[0].y, boxes[0].w, boxes[0].h),
            (100.0, 100.0, 20.0, 20.0)
        );
    }

    /// 数量对得上就按 libass 的顺序配：未拖过的行锚点全相同，只能靠顺序。
    #[test]
    fn same_anchor_lines_pair_by_script_order() {
        let (first, second) = (ItemId::new(), ItemId::new());
        // `fix_collisions` 把后一条往上挤，所以脚本在前的行画在下面
        let tiles = vec![
            rect(900.0, 1040.0, 60.0, 30.0),
            rect(900.0, 1000.0, 60.0, 30.0),
        ];
        let boxes = line_boxes(&tiles, &[hit(first), hit(second)], 500, LAYOUT, RASTER);
        assert_eq!(boxes[0].item_id, first);
        assert_eq!(boxes[1].item_id, second);
    }

    /// 数量对不上（把一行拖到画面外，只剩一簇）时按锚点就近配，不误伤别的行。
    #[test]
    fn mismatched_counts_pair_by_nearest_anchor() {
        let (first, second) = (ItemId::new(), ItemId::new());
        let tiles = vec![rect(100.0, 100.0, 20.0, 20.0)];
        let mut a = hit(first);
        a.anchor = Some((110.0, 120.0));
        let mut b = hit(second);
        b.anchor = Some((2800.0, 3000.0));
        let boxes = line_boxes(&tiles, &[a, b], 500, LAYOUT, RASTER);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].item_id, first);
    }

    /// 不在生效区间的行不参与配对（时间判定与 libass 同一口径：`start <= now < end`）。
    #[test]
    fn only_lines_visible_at_this_moment_are_paired() {
        let (first, second) = (ItemId::new(), ItemId::new());
        let mut late = hit(second);
        late.start_ms = 2_000;
        late.end_ms = 3_000;
        let tiles = vec![rect(100.0, 100.0, 20.0, 20.0)];
        let boxes = line_boxes(&tiles, &[hit(first), late], 1_500, LAYOUT, RASTER);
        assert!(boxes.is_empty(), "此刻没有一行在生效，簇配不上行就不该有框");
    }

    /// 光栅倍率要除回去：块坐标是光栅像素，框给的是排版像素。
    #[test]
    fn raster_scale_is_divided_out() {
        let first = ItemId::new();
        let tiles = vec![rect(200.0, 200.0, 40.0, 40.0)];
        let boxes = line_boxes(&tiles, &[hit(first)], 500, LAYOUT, (3840, 2160));
        assert_eq!(
            (boxes[0].x, boxes[0].y, boxes[0].w, boxes[0].h),
            (100.0, 100.0, 20.0, 20.0)
        );
    }
}
