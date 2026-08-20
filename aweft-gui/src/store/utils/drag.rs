//! 拖动预览通用工具：与字幕行时间拖动、Area 合并拆分的最终偏移提交校验一致，
//! 仅在 `commit` 时校验最终偏移是否有效（`is_finite` + 范围），`preview` 阶段
//! 只做增量预览与钳制，右键取消时增量回退，无需全量克隆快照。

/// 校验拖动增量是否有效（与 `reducers/sequence.rs: DragTimecodePreview/Commit` 的
/// `delta_x.is_finite() && delta_x < i64::MAX` 校验同源）。
pub fn is_valid_delta(delta: f32) -> bool {
    delta.is_finite() && delta < i64::MAX as f32 && delta > i64::MIN as f32
}

pub fn is_valid_delta_pair(dx: f32, dy: f32) -> bool {
    is_valid_delta(dx) && is_valid_delta(dy)
}
