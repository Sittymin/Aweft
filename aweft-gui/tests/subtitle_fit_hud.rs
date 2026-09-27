//! 字幕拟合浮层不再挡操作。
//!
//! 浮层原先是窗口顶层的整屏模态遮罩（`TouchArea` 吞掉全部按下事件），现在改成挂在视口
//! 右上角的一块局部进度画面。这里用真实的 `SubtitleFitOverlay` 搭一个最小宿主，锁定两件事：
//! 1. 卡片贴在宿主右上角，尺寸小于宿主——它是一块角标，不是整屏；
//! 2. 点击卡片中心仍然命中它下面的画布 `TouchArea`——浮层不参与命中，拟合期间照常操作。
//!
//! 第 2 条是本次改动的全部目的，也是以后往卡片里加交互控件时最先被撞到的约束。
//! 宿主不 import `bridge/state.slint`：理由同 `context_menu_dismiss.rs`，别把整个应用状态
//! 树拖进本测试的编译单元。

use std::time::Duration;

use i_slint_backend_testing::{ElementHandle, init_no_event_loop, mock_elapsed_time};
use slint::platform::PointerEventButton;

slint::slint! {
    import { SubtitleFitOverlay } from "ui/areas/viewport/subtitle_fit.slint";

    export component FitHudHarness inherits Window {
        width: 600px;
        height: 400px;
        in-out property <bool> show-hud: true;
        in-out property <int> canvas-clicks: 0;

        // 视口画布的替身：`CanvasNavigation` 那份要吃 `VideoState` 与回调，这里只需要一个
        // 能吃下点击的整窗 `TouchArea`
        TouchArea {
            width: 100%;
            height: 100%;
            clicked => {
                root.canvas-clicks += 1;
            }
        }

        // 几何与 areas/viewport/view.slint 的挂载点保持一致
        if show-hud: SubtitleFitOverlay {
            width: min(420px, parent.width - 24px);
            height: min(224px, parent.height - 40px);
            x: parent.width - self.width - 12px;
            y: 20px;
        }
    }
}

fn hud(ui: &FitHudHarness) -> ElementHandle {
    ElementHandle::find_by_element_type_name(ui, "SubtitleFitOverlay")
        .next()
        .expect("宿主里应有一个 SubtitleFitOverlay")
}

/// 卡片贴在右上角，且只占宿主的一部分。
#[test]
fn fit_hud_is_a_corner_panel_not_a_full_window_mask() {
    init_no_event_loop();
    let ui = FitHudHarness::new().unwrap();
    mock_elapsed_time(Duration::from_millis(16));

    let hud = hud(&ui);
    let position = hud.absolute_position();
    let size = hud.size();

    assert!(
        size.width > 0.0 && size.height > 0.0,
        "卡片应有尺寸，实际 {size:?}"
    );
    assert!(
        size.width < 600.0 && size.height < 400.0,
        "卡片是局部画面，不该铺满宿主，实际 {size:?}"
    );
    // 右边界留 12px 边距：x + width == 600 - 12
    assert!(
        (position.x + size.width - 588.0).abs() < 0.5,
        "卡片应贴右边界（留 12px 边距），实际右边界 {}",
        position.x + size.width
    );
    assert!(
        (position.y - 20.0).abs() < 0.5,
        "卡片 y 应让开顶部 16px 标尺，实际 {}",
        position.y
    );
}

/// 点在卡片正中间的按下仍然命中画布：浮层不吃事件。
///
/// 卡片没被实例化时 `hud` 直接 panic，所以这里不存在「浮层其实没显示、点击当然穿过」的假绿。
#[test]
fn click_on_fit_hud_reaches_the_canvas() {
    init_no_event_loop();
    let ui = FitHudHarness::new().unwrap();
    mock_elapsed_time(Duration::from_millis(16));

    assert!(ui.get_show_hud(), "前置条件：浮层显示中");
    let hud = hud(&ui);

    hud.mock_single_click(PointerEventButton::Left);

    assert_eq!(
        ui.get_canvas_clicks(),
        1,
        "卡片中心的按下应穿过浮层命中画布，而不是被浮层吃掉"
    );
}
