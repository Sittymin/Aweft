//! 通用一维分段工具：把连续区间切成最少段数，使每段都满足给定的有效性条件。
//!
//! 与动画转换逻辑解耦：这里不关心曲线、标签或误差的具体含义，只负责在
//! 格点图上做最短路切分。

/// 均匀格点序列 `0..=n` 到时间轴的映射。
pub(crate) struct UniformGrid {
    start: f64,
    end: f64,
    n: usize,
}

impl UniformGrid {
    /// 新建一个从 `start` 到 `end`、共 `n` 个等宽区间的网格（`n+1` 个格点）。
    pub(crate) fn new(start: f64, end: f64, n: usize) -> Self {
        Self { start, end, n }
    }

    /// 第 `idx` 个格点对应的时间。
    pub(crate) fn time_at(&self, idx: usize) -> f64 {
        self.start + (idx as f64 / self.n as f64) * (self.end - self.start)
    }

    /// 区间总数（`0..=n` 共 `n+1` 个格点）。
    pub(crate) fn len(&self) -> usize {
        self.n
    }
}

/// 在 `0..=n` 的格点上求「覆盖到每个格点所需的最少段数」，返回 `parent` 表。
///
/// 每处理完一个 `end` 就回调一次 `on_end(end, chosen)`，界面上那条进度读它；不需要
/// 就传 `|_, _| {}`。`chosen` 是本趟为这个 `end` 选出的起点，`None` 表示还没有起点
/// 可达。回调在外层串行循环里，不在并行段里，因此没有额外同步开销。
pub(crate) fn min_segmentation(
    n: usize,
    is_valid: impl Fn(usize, usize) -> bool + Sync,
    mut on_end: impl FnMut(usize, Option<usize>),
) -> Vec<usize> {
    use rayon::prelude::*;
    let mut dp = vec![u32::MAX; n + 1];
    let mut parent = vec![usize::MAX; n + 1];
    dp[0] = 0;
    for end in 1..=n {
        if end % (n / 10).max(1) == 0 {
            log::info!("DP grid {}/{} ({}%)", end, n, end * 100 / n);
        }
        let best = (0..end)
            .into_par_iter()
            .rev()
            .filter(|&start| dp[start] != u32::MAX)
            .filter(|&start| is_valid(start, end))
            .min_by_key(|&start| (dp[start] + 1, std::cmp::Reverse(start)));
        if let Some(start) = best {
            let cost = dp[start] + 1;
            if cost < dp[end] {
                dp[end] = cost;
                parent[end] = start;
            }
        }
        on_end(end, best);
    }
    parent
}

/// 追踪版 DP：算法与 [`min_segmentation`] 完全一致，另外把每个 `end` 的逐起点有效性
/// 和本趟选出的起点交给回调，供实时可视化消费。
///
/// `valid[start]` 为真表示「`start` 可达且 `is_valid(start, end)`」，长度 = `end`。
/// 两个 `end` 之间可达性会变（前面的格点可能一直连不上），所以这张表只描述当前这个
/// `end`；回调借用它，想留一份自己拷。
///
/// 只比 [`min_segmentation`] 多一次布尔表的分配，`is_valid` 的次数一样。
pub(crate) fn min_segmentation_traced(
    n: usize,
    is_valid: impl Fn(usize, usize) -> bool + Sync,
    mut on_end: impl FnMut(usize, &[bool], Option<usize>),
) -> Vec<usize> {
    use rayon::prelude::*;
    let mut dp = vec![u32::MAX; n + 1];
    let mut parent = vec![usize::MAX; n + 1];
    dp[0] = 0;
    for end in 1..=n {
        // 不可达的起点进不了路径，也不必问 `is_valid`：与生产版的 filter 条件同形。
        let valid: Vec<bool> = (0..end)
            .into_par_iter()
            .map(|start| dp[start] != u32::MAX && is_valid(start, end))
            .collect();
        let best = (0..end)
            .rev()
            .filter(|&start| valid[start])
            .min_by_key(|&start| (dp[start] + 1, std::cmp::Reverse(start)));
        // 先写 parent[end] 再回调：回调看到的 `chosen` 就是已定案的这一格。
        if let Some(start) = best {
            dp[end] = dp[start] + 1;
            parent[end] = start;
        }
        on_end(end, &valid, best);
    }
    parent
}

/// 兜底切分：把尚未连通（`parent[end] == usize::MAX`）的格点强制接到前一个
/// 格点，保证从任意终点都能反推到 0。单格步长在任何连续性场景下都总是合法。
pub(crate) fn force_single_step_path(parent: &mut [usize]) {
    for end in 1..parent.len() {
        if parent[end] == usize::MAX {
            parent[end] = end - 1;
        }
    }
}

/// 从 `parent` 反推切分路径，返回按起点升序的 `(起点格点, 终点格点)` 对。
/// 要求终点可达（必要时先用 [`force_single_step_path`] 兜底）。
pub(crate) fn reconstruct_path(parent: &[usize]) -> Vec<(usize, usize)> {
    let mut path = Vec::new();
    let mut end = parent.len() - 1;
    while end != 0 {
        let start = parent[end];
        path.push((start, end));
        end = start;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_segmentation_all_valid_yields_single_segment() {
        let parent = min_segmentation(8, |_, _| true, |_, _| {});
        assert_eq!(parent[8], 0); // 整段一次切完
        let path = reconstruct_path(&parent);
        assert_eq!(path, vec![(0, 8)]);
    }

    /// 进度回调每个格点报一次、按序报完，界面上那条进度就读它。
    #[test]
    fn min_segmentation_reports_progress_once_per_grid_point() {
        let mut seen = Vec::new();
        min_segmentation(5, |_, _| true, |end, chosen| seen.push((end, chosen)));
        assert_eq!(
            seen,
            vec![
                (1, Some(0)),
                (2, Some(0)),
                (3, Some(0)),
                (4, Some(0)),
                (5, Some(0))
            ]
        );
    }

    /// 同一个回调契约下，报出的 `chosen` 就是最终 `parent[end]`：界面靠它逐格重建
    /// 已锁定路径，所以两者不能各说各话。
    #[test]
    fn reported_chosen_matches_parent_table() {
        let is_valid = |start: usize, end: usize| end <= 3 || start >= 3;
        let mut seen = Vec::new();
        let parent = min_segmentation(6, is_valid, |end, chosen| seen.push((end, chosen)));
        for (end, chosen) in seen {
            assert_eq!(parent[end], chosen.unwrap_or(usize::MAX), "格点 {end}");
        }
    }

    /// 追踪版只多给一张有效性表，切分结果必须与生产版逐格一致：一边管导出、
    /// 一边管界面画的那条路径，两者分叉的话界面展示的就不是导出的结果。
    #[test]
    fn traced_version_yields_the_same_path_as_production() {
        let cases: Vec<(usize, &str, fn(usize, usize) -> bool)> = vec![
            (8, "all valid", |_, _| true),
            (6, "blocked middle", |s, e| e <= 3 || s >= 3),
            (7, "single steps only", |s, e| e == s + 1),
            (5, "unreachable tail", |_, e| e <= 4),
            (9, "window of two", |s, e| e == s + 1 || e == s + 2),
        ];
        for (n, label, is_valid) in cases {
            let plain = min_segmentation(n, is_valid, |_, _| {});
            let mut traced_valid_len = n * n;
            let traced =
                min_segmentation_traced(n, is_valid, |_, valid, _| traced_valid_len = valid.len());
            assert_eq!(traced, plain, "{label}");
            assert_eq!(traced_valid_len, n, "{label}：末格的有效性表长度 = end");
        }
    }

    /// 有效性表逐格变长：`valid.len() == end`，下标就是起点格点。
    /// 这里让「跨 1 格或跨 3 格」合法，表里就有真有假，且每格都可达。
    #[test]
    fn traced_version_reports_validity_per_start() {
        let is_valid = |start: usize, end: usize| matches!(end - start, 1 | 3);
        let mut rows = Vec::new();
        min_segmentation_traced(4, is_valid, |end, valid, _| {
            assert_eq!(valid.len(), end);
            rows.push(valid.to_vec());
        });
        assert_eq!(rows[0], vec![true]);
        assert_eq!(rows[1], vec![false, true]);
        assert_eq!(rows[2], vec![true, false, true]);
    }

    #[test]
    fn min_segmentation_splits_at_blocked_boundary() {
        // 只有完全落在 [0,3] 或 [3,6] 内的段合法，跨越 3 的段不合法。
        let is_valid = |start: usize, end: usize| end <= 3 || start >= 3;
        let parent = min_segmentation(6, is_valid, |_, _| {});
        assert_eq!(parent[3], 0);
        assert_eq!(parent[6], 3);
        let path = reconstruct_path(&parent);
        assert_eq!(path, vec![(0, 3), (3, 6)]);
    }

    #[test]
    fn min_segmentation_prefers_fewer_segments() {
        // 全部合法时，最少段数路径优先（整段一段）。
        let parent = min_segmentation(4, |_, _| true, |_, _| {});
        assert_eq!(parent[4], 0);
        assert_eq!(reconstruct_path(&parent), vec![(0, 4)]);
    }

    #[test]
    fn min_segmentation_returns_max_when_unreachable() {
        // 没有任何段能到达格点 5 → 终点不可达。
        let is_valid = |_start: usize, end: usize| end <= 4;
        let parent = min_segmentation(5, is_valid, |_, _| {});
        assert_eq!(parent[5], usize::MAX);
    }

    #[test]
    fn force_single_step_path_repairs_unreachable() {
        let mut parent = vec![usize::MAX; 5];
        parent[2] = 1;
        force_single_step_path(&mut parent);
        assert_eq!(parent, vec![usize::MAX, 0, 1, 2, 3]);
        // 反推必然能回到 0
        let path = reconstruct_path(&parent);
        assert_eq!(path, vec![(0, 1), (1, 2), (2, 3), (3, 4)]);
    }

    #[test]
    fn uniform_grid_maps_indices_to_times() {
        let grid = UniformGrid::new(10.0, 30.0, 4);
        assert_eq!(grid.len(), 4);
        assert_eq!(grid.time_at(0), 10.0);
        assert_eq!(grid.time_at(2), 20.0);
        assert_eq!(grid.time_at(4), 30.0);
    }
}
