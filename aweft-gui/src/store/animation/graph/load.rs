//! Action 装载/提交：model ↔ 编辑数据往返 + 通道增删（reducer 侧，只改数据层）。

use slint::{Color, ModelRc, SharedString};

use super::{GraphEditorState, snap_ms};
use crate::bridge::converters::{model_key_to_slint, slint_key_to_model};
use crate::bridge::types::{SlintColorMode, SlintFCurveChannel, SlintKeyframe};
use crate::depsgraph::Tag;
use crate::store::utils::rainbow::rainbow_color;
use aweft_model::animation::{Action as ModelAction, Keyframe as ModelKeyframe, PropertyPath};
use aweft_model::ids::ActionId;

/// model `PropertyPath` → 通道显示名（GUI 侧映射；model 无 Display）。
/// 当前直接用 Rust 变体名（Debug 格式），后续可换友好名。
fn property_display_name(p: &PropertyPath) -> SharedString {
    format!("{:?}", p).into()
}

impl GraphEditorState {
    /// 追加一个通道（数据层）：写入 `channels_data` + `editing_keyframes`。
    /// 颜色默认"自动彩虹"：显示色取自彩虹调色板（按通道下标），传入的 color 作为用户色。
    /// 返回结构变化 tag（投影重建派生层）。
    ///
    /// 注意：`color` 会被当作「用户设置」模式的用户色保存；由于调色盘尚未实现，
    /// 这目前也是用户色唯一的来源（测试数据传显式颜色即为此）。
    pub fn push_channel(
        &mut self,
        name: impl Into<SharedString>,
        color: Color,
        keyframes: Vec<SlintKeyframe>,
    ) -> Vec<Tag> {
        let idx = self.channels_data.len();
        // 时间坐标以毫秒为最小单位：关键帧必须落在整数毫秒上（UI 先行阶段在此归一化，
        // 后续接入 model 后由数据层保证）。
        let keyframes: Vec<SlintKeyframe> = keyframes
            .into_iter()
            .map(|mut k| {
                k.time = snap_ms(k.time);
                k
            })
            .collect();
        self.editing_keyframes.insert(idx, keyframes);
        self.channels_data.push(SlintFCurveChannel {
            curve_index: idx as i32,
            name: name.into(),
            color: rainbow_color(idx),
            user_color: color,
            color_mode: SlintColorMode::AutoRainbow,
            visible: true,
            modifier_enabled: true,
            active: true,
            editable: true,
            selected: false,
            keyframes: ModelRc::default(),
        });
        vec![Tag::GraphStructureChanged]
    }

    /// 装载 model 的第 `index` 个 Action（数据层）：清空旧通道/关键帧数据，
    /// 按 model 曲线写入编辑数据。权威数据在 model（`Action.curves`）。
    pub fn load_action(&mut self, index: usize, action: &ModelAction) -> Vec<Tag> {
        self.channels_data.clear();
        self.editing_keyframes.clear();
        for ac in action.curves.iter() {
            let name = property_display_name(&ac.property);
            let kfs: Vec<SlintKeyframe> =
                ac.curve.keyframes.iter().map(model_key_to_slint).collect();
            // NOTE: model 的 ActionCurve 暂无颜色字段，通道色目前仅 UI 态（彩虹）；
            // 若需持久化用户色，应给 model 的 ActionCurve 加 color 字段。
            self.push_channel(name, Color::from_rgb_u8(0, 0, 0), kfs);
        }
        self.action_name = action.name.clone().into();
        self.current_action = index as i32;
        vec![Tag::GraphStructureChanged]
    }

    /// 当前编辑的 ActionId（`current_action` 下标 → `actions` 清单；-1/越界返回 None）。
    pub fn current_action_id(&self) -> Option<ActionId> {
        let idx = self.current_action;
        if idx < 0 {
            return None;
        }
        self.actions.get(idx as usize).map(|a| a.id)
    }

    /// 把当前编辑关键帧写回 model Action（拖拽/插值变更后提交）。
    ///
    /// 通道下标 ↔ `action.curves` 下标 1:1（`load_action` 建立的映射）；
    /// 关键帧按时间排序写回 `FCurve.keyframes`，UI 选中态（selected）不入 model。
    pub fn commit_to_model(&self, action: &mut ModelAction) {
        for (curve, ac) in action.curves.iter_mut().enumerate() {
            let Some(kfs) = self.editing_keyframes.get(&curve) else {
                continue;
            };
            let mut kfs: Vec<ModelKeyframe> = kfs.iter().map(slint_key_to_model).collect();
            kfs.sort_by(|a, b| {
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            ac.curve.keyframes = kfs;
        }
    }

    /// 下插到 NLA 后重置全局 Action（数据层）：清空当前 Action 的通道/关键帧数据，
    /// 开始一个全新的空 Action（无名称，UI 显示"<无动作>"）。
    pub fn reset_action(&mut self) -> Vec<Tag> {
        self.channels_data.clear();
        self.editing_keyframes.clear();
        self.action_name = SharedString::default();
        self.current_action = -1;
        vec![Tag::GraphStructureChanged]
    }
}
