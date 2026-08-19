//! 摄影表连接线：线性/常量插值段的直线连接（扁平模型）。

use slint::Model;

use super::GraphEditorState;
use crate::bridge::types::{SlintDopeConnection, SlintInterpolation, SlintKeyframe};

impl GraphEditorState {
    /// 重建摄影表连接线：遍历摄影表过滤视图（压缩，仅匹配搜索的通道），
    /// 每个通道内按时间排序后，相邻关键帧之间为线性/常量插值的加一条连接。
    /// row-index 为压缩视图行号（画布行位置用）；visible 反映通道眼睛。
    pub(super) fn rebuild_dope_connections(&mut self) {
        let mut conns: Vec<SlintDopeConnection> = Vec::new();
        for row in 0..self.dope_filter.row_count() {
            let Some(ch) = self.dope_filter.row_data(row) else {
                continue;
            };
            let c = ch.curve_index as usize;
            let Some(kfs) = self.editing_keyframes.get(&c) else {
                continue;
            };
            let Some(ch_data) = self.channels_data.get(c) else {
                continue;
            };
            let mut kfs: Vec<&SlintKeyframe> = kfs.iter().collect();
            kfs.sort_by(|a, b| {
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for i in 0..kfs.len().saturating_sub(1) {
                let interp = kfs[i].interpolation;
                if interp == SlintInterpolation::Linear || interp == SlintInterpolation::Hold {
                    conns.push(SlintDopeConnection {
                        curve_index: c as i32,
                        row_index: row as i32,
                        x0: kfs[i].time,
                        x1: kfs[i + 1].time,
                        color: ch_data.color,
                        is_hold: interp == SlintInterpolation::Hold,
                        visible: ch_data.visible,
                    });
                }
            }
        }
        self.dope_connections.set_vec(conns);
    }

    /// 重算单个通道的摄影表连接线：移除旧连接，插入新连接，保持其余通道不动。
    /// 只在关键帧时间/插值变化后调用（`rebuild_curve`）。
    pub(super) fn rebuild_dope_connections_for_curve(&mut self, curve: usize) {
        use std::collections::BTreeSet;

        let curve_i32 = curve as i32;
        let row_count = self.dope_filter.row_count();

        // 1. 找出该通道旧连接的索引范围（连续，因按 dope_filter 行序排列）
        let mut old_indices: BTreeSet<usize> = BTreeSet::new();
        for i in 0..self.dope_connections.row_count() {
            if let Some(c) = self.dope_connections.row_data(i) {
                if c.curve_index == curve_i32 {
                    old_indices.insert(i);
                }
            }
        }
        let old_start = old_indices
            .iter()
            .next()
            .copied()
            .unwrap_or(self.dope_connections.row_count());
        let old_count = old_indices.len();

        // 2. 算该通道的新连接
        let mut new_conns = Vec::new();
        // 找该通道在 dope_filter 中的行号
        for row in 0..row_count {
            if let Some(ch) = self.dope_filter.row_data(row) {
                if ch.curve_index as usize == curve {
                    let Some(kfs) = self.editing_keyframes.get(&curve) else {
                        continue;
                    };
                    let Some(ch_data) = self.channels_data.get(curve) else {
                        continue;
                    };
                    let mut kfs: Vec<&SlintKeyframe> = kfs.iter().collect();
                    kfs.sort_by(|a, b| {
                        a.time
                            .partial_cmp(&b.time)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    for i in 0..kfs.len().saturating_sub(1) {
                        let interp = kfs[i].interpolation;
                        if interp == SlintInterpolation::Linear
                            || interp == SlintInterpolation::Hold
                        {
                            new_conns.push(SlintDopeConnection {
                                curve_index: curve_i32,
                                row_index: row as i32,
                                x0: kfs[i].time,
                                x1: kfs[i + 1].time,
                                color: ch_data.color,
                                is_hold: interp == SlintInterpolation::Hold,
                                visible: ch_data.visible,
                            });
                        }
                    }
                    break;
                }
            }
        }

        // 3. 替换：移除旧连接，插入新连接
        if old_count == new_conns.len() {
            // 数量不变：原地 set_row_data
            for (i, conn) in new_conns.iter().enumerate() {
                self.dope_connections
                    .set_row_data(old_start + i, conn.clone());
            }
        } else if old_count == 0 {
            // 无旧连接：插入新连接
            for (i, conn) in new_conns.iter().enumerate() {
                self.dope_connections.insert(old_start + i, conn.clone());
            }
        } else {
            // 数量变化：移除旧连接，插入新连接（用 splice 语义）
            for _ in 0..old_count {
                self.dope_connections.remove(old_start);
            }
            for (i, conn) in new_conns.iter().enumerate() {
                self.dope_connections.insert(old_start + i, conn.clone());
            }
        }
    }
}
