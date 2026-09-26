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
pub(crate) fn min_segmentation(
    n: usize,
    is_valid: impl Fn(usize, usize) -> bool + Sync,
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
    }
    parent
}

/// 诊断版 DP：与 `min_segmentation` 完全相同的算法，同时每个 `end` 处理完后调用
/// `on_end(end, valid, chosen, parent_chain)` 传出该 end 的完整有效性和最优选择，
/// 供实时可视化消费。
pub(crate) fn diagnostic_min_segmentation(
    n: usize,
    is_valid: impl Fn(usize, usize) -> bool + Sync,
    // on_end(end, valid_mask, chosen, parent_chain)：parent_chain 是当前已确定的
    // parent 表（0..=end 的前缀），用于实时显示已锁定的路径。
    mut on_end: impl FnMut(usize, Vec<bool>, Option<usize>, Vec<usize>),
) -> Vec<usize> {
    use rayon::prelude::*;
    let mut dp = vec![u32::MAX; n + 1];
    let mut parent = vec![usize::MAX; n + 1];
    dp[0] = 0;
    for end in 1..=n {
        // 并行计算所有可达起点的有效性（与生产版 min_segmentation 一致）。
        let reachable: Vec<usize> = (0..end).filter(|&s| dp[s] != u32::MAX).collect();
        let valid_flags: Vec<bool> = reachable.par_iter().map(|&s| is_valid(s, end)).collect();
        let mut valid_mask = vec![false; end];
        let mut candidates = Vec::new();
        for (&start, &valid) in reachable.iter().zip(&valid_flags) {
            if valid {
                valid_mask[start] = true;
                candidates.push(start);
            }
        }
        let best = candidates
            .into_par_iter()
            .rev()
            .min_by_key(|&start| (dp[start] + 1, std::cmp::Reverse(start)));
        // 先更新 parent[end] 再回调，这样 on_end 能看到完整前缀路径
        if let Some(start) = best {
            let cost = dp[start] + 1;
            if cost < dp[end] {
                dp[end] = cost;
                parent[end] = start;
            }
        }
        let chain = parent[..=end].to_vec();
        on_end(end, valid_mask, best, chain);
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
        let parent = min_segmentation(8, |_, _| true);
        assert_eq!(parent[8], 0); // 整段一次切完
        let path = reconstruct_path(&parent);
        assert_eq!(path, vec![(0, 8)]);
    }

    #[test]
    fn min_segmentation_splits_at_blocked_boundary() {
        // 只有完全落在 [0,3] 或 [3,6] 内的段合法，跨越 3 的段不合法。
        let is_valid = |start: usize, end: usize| end <= 3 || start >= 3;
        let parent = min_segmentation(6, is_valid);
        assert_eq!(parent[3], 0);
        assert_eq!(parent[6], 3);
        let path = reconstruct_path(&parent);
        assert_eq!(path, vec![(0, 3), (3, 6)]);
    }

    #[test]
    fn min_segmentation_prefers_fewer_segments() {
        // 全部合法时，最少段数路径优先（整段一段）。
        let parent = min_segmentation(4, |_, _| true);
        assert_eq!(parent[4], 0);
        assert_eq!(reconstruct_path(&parent), vec![(0, 4)]);
    }

    #[test]
    fn min_segmentation_returns_max_when_unreachable() {
        // 没有任何段能到达格点 5 → 终点不可达。
        let is_valid = |_start: usize, end: usize| end <= 4;
        let parent = min_segmentation(5, is_valid);
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
