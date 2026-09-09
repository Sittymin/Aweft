//! 右键菜单「点菜单外任意处收起」无头测试。
//!
//! 复现波形图音轨列表曾经的问题：菜单只在点中菜单项时关闭。用真实 `ContextMenu` +
//! 真实 `DismissOverlay`（MainWindow 顶层那一份差集遮罩）搭一个最小宿主，锁定三件事：
//! 1. 菜单打开时上报的挖孔矩形是 MainWindow 窗口坐标（`ContextMenu` 内建 absolute-position）；
//! 2. 孔外按下（含右键，`clicked` 只覆盖左键）→ 遮罩收起 → 菜单自动关闭；
//! 3. 菜单收起后不再吞掉落在它原来位置上的点击。
//! 遮罩状态用同形本地镜像而非 `bridge/state.slint`：避免把整个应用状态树拖进本测试编译单元。

use std::time::Duration;

use i_slint_backend_testing::{ElementHandle, init_no_event_loop, mock_elapsed_time};
use slint::platform::PointerEventButton;
use slint::ComponentHandle;

slint::slint! {
    import { ContextMenu, MenuItem, MenuRect } from "ui/widgets/context_menu.slint";
    import { DismissOverlay } from "ui/widgets/dismiss_overlay.slint";

    // 与 bridge/state.slint 的 DismissOverlayState 同形的本地镜像
    export global OverlayMirror {
        in-out property <bool> active: false;
        in-out property <length> hole-x: 0px;
        in-out property <length> hole-y: 0px;
        in-out property <length> hole-width: 0px;
        in-out property <length> hole-height: 0px;

        public function show(rect: MenuRect) {
            hole-x = rect.x;
            hole-y = rect.y;
            hole-width = rect.width;
            hole-height = rect.height;
            active = true;
        }
        public function hide() { active = false; }
    }

    export component MenuDismissHarness inherits Window {
        width: 400px;
        height: 300px;
        in-out property <bool> menu-open: false;
        in-out property <int> activations: 0;

        // Area 在窗口里有偏移 (100,50)，菜单在 Area 本地 (30,40) → 窗口坐标 (130,90)
        Rectangle {
            x: 100px;
            y: 50px;
            width: 250px;
            height: 200px;
            ContextMenu {
                open <=> root.menu-open;
                menu-x: 30px;
                menu-y: 40px;
                container-width: parent.width;
                container-height: parent.height;
                overlay-active: OverlayMirror.active;
                overlay-shown(rect) => { OverlayMirror.show(rect); }
                overlay-hidden() => { OverlayMirror.hide(); }
                MenuItem {
                    text: "设置为当前播放音轨";
                    menu-open <=> root.menu-open;
                    activated => { root.activations += 1; }
                }
            }
        }

        // 模拟"右键命中音轨行"的入口
        opener := Rectangle {
            x: 0px;
            y: 0px;
            width: 40px;
            height: 40px;
            TouchArea {
                clicked => { root.menu-open = true; }
            }
        }

        // 孔外背景：中心 (200,275) 在菜单下方
        bg := Rectangle { x: 0px; y: 260px; width: 400px; height: 30px; }

        if OverlayMirror.active: DismissOverlay {
            hole-x: OverlayMirror.hole-x;
            hole-y: OverlayMirror.hole-y;
            hole-width: OverlayMirror.hole-width;
            hole-height: OverlayMirror.hole-height;
            dismissed => { OverlayMirror.active = false; }
        }
    }
}

fn element(ui: &MenuDismissHarness, id: &str) -> ElementHandle {
    ElementHandle::find_by_element_id(ui, id)
        .next()
        .unwrap_or_else(|| panic!("宿主窗口里找不到 {id}"))
}

/// 打开菜单：左键点 opener（真实事件路径，顺带把布局/变更处理器推到稳定态）
fn open_menu(ui: &MenuDismissHarness) {
    element(ui, "MenuDismissHarness::opener").mock_single_click(PointerEventButton::Left);
    mock_elapsed_time(Duration::from_millis(16));
}

#[test]
fn menu_hole_uses_window_coordinates() {
    init_no_event_loop();
    let ui = MenuDismissHarness::new().unwrap();

    open_menu(&ui);

    let state = ui.global::<OverlayMirror>();
    assert!(state.get_active(), "菜单打开后遮罩应激活");
    assert_eq!(state.get_hole_x(), 130.0, "孔 x = Area 偏移 100 + 菜单本地 30");
    assert_eq!(state.get_hole_y(), 90.0, "孔 y = Area 偏移 50 + 菜单本地 40");
    assert_eq!(state.get_hole_width(), 180.0, "孔宽 = 菜单固定宽");
    assert!(
        state.get_hole_height() > 0.0,
        "孔高应随菜单内容布局得出，实际 {}",
        state.get_hole_height()
    );
}

#[test]
fn click_outside_menu_closes_it() {
    init_no_event_loop();
    let ui = MenuDismissHarness::new().unwrap();

    open_menu(&ui);
    assert!(ui.get_menu_open(), "前置条件：菜单已打开");

    // 孔外右键按下（原实现用 clicked 只认左键，右键收不起来）
    element(&ui, "MenuDismissHarness::bg").mock_single_click(PointerEventButton::Right);

    let state = ui.global::<OverlayMirror>();
    assert!(!state.get_active(), "孔外点击后遮罩应收起");
    assert!(!ui.get_menu_open(), "遮罩收起后菜单应自动关闭");
    assert_eq!(ui.get_activations(), 0, "孔外点击不应激活菜单项");
}

#[test]
fn click_inside_menu_reaches_the_item() {
    init_no_event_loop();
    let ui = MenuDismissHarness::new().unwrap();

    open_menu(&ui);

    // 孔内（菜单项中心）左键：遮罩不介入，菜单项正常响应并关闭菜单
    let item = ElementHandle::find_by_element_type_name(&ui, "MenuItem")
        .next()
        .expect("菜单里应有 1 个 MenuItem");
    item.mock_single_click(PointerEventButton::Left);

    assert_eq!(ui.get_activations(), 1, "孔内点击应命中菜单项");
    assert!(!ui.get_menu_open(), "点中菜单项后菜单关闭");
    assert!(
        !ui.global::<OverlayMirror>().get_active(),
        "菜单收起后必须释放遮罩，否则整窗被盖住"
    );

    // 复位后点菜单原位置：已收起的菜单（visible: false）不该再吃掉点击
    ui.set_activations(0);
    item.mock_single_click(PointerEventButton::Left);
    assert_eq!(ui.get_activations(), 0, "收起的菜单不应再吞点击");
}
