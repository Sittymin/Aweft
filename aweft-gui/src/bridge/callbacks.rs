use crate::actions::sequence::SequenceAction;
use crate::actions::{Action, area::AreaAction};
use crate::bridge::types::{
    AnimationCallbacks, AreaCallbacks, DragCallbacks, MainWindow, NodeGraphCallbacks,
    SequenceCallbacks, VideoCallbacks, CollabCallbacks,
};
// 媒体选择弹层的回调只在 Android 装配（见 setup_media_picker_callbacks）。
#[cfg(target_os = "android")]
use crate::bridge::types::MediaPickerCallbacks;

use crate::bridge::{parse_time, slint_area_key_to_key, slint_area_type_to_rust};
use crate::render::subtitle::Composition;
use crate::store::AppStore;
use aweft_model::ids::ItemId;
use log::warn;
// winit 事件只在桌面后端存在：Android 特性集里 winit 后端被彻底排除
//（见 aweft-gui/Cargo.toml），slint::winit_030 模块在该目标上不存在。
#[cfg(not(target_os = "android"))]
use slint::winit_030::winit::{self, event::WindowEvent};
#[cfg(not(target_os = "android"))]
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, RenderingState};
use std::{cell::RefCell, rc::Rc};

/// 设置所有回调
pub fn setup_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    setup_area_type_callback(instance, Rc::clone(&store));
    setup_rendering_notifier_callback(instance, Rc::clone(&store));
    #[cfg(not(target_os = "android"))]
    setup_winit_window_event_callback(instance, Rc::clone(&store));
    setup_actionzone_drag_callback(instance, Rc::clone(&store));
    setup_actionzone_drag_end_callback(instance, Rc::clone(&store));
    setup_edge_drag_callback(instance, Rc::clone(&store));
    setup_edge_drag_end_callback(instance, Rc::clone(&store));
    setup_toggle_group_collapsed_callback(instance, Rc::clone(&store));
    setup_sequence_timecode_editor_callback(instance, Rc::clone(&store));
    setup_sequence_timecode_step_callbacks(instance, Rc::clone(&store));
    setup_sequence_drag_timecode_callbacks(instance, Rc::clone(&store));
    setup_sequence_drag_timecode_cancel_callback(instance, Rc::clone(&store));
    setup_sequence_selection_callbacks(instance, Rc::clone(&store));
    setup_sequence_add_line_callback(instance, Rc::clone(&store));
    setup_sequence_add_group_callback(instance, Rc::clone(&store));
    setup_sequence_delete_selected_items_callback(instance, Rc::clone(&store));
    setup_sequence_clipboard_callbacks(instance, Rc::clone(&store));
    setup_sequence_ungroup_callbacks(instance, Rc::clone(&store));
    setup_node_graph_callbacks(instance, Rc::clone(&store));
    setup_animation_callbacks(instance, Rc::clone(&store));
    setup_nla_callbacks(instance, Rc::clone(&store));
    setup_video_callbacks(instance, Rc::clone(&store));
    setup_preview_clock(instance, &store);
    setup_audio_callbacks(instance, Rc::clone(&store));
    setup_subtitle_fit(Rc::clone(&store));
    setup_media_picker_callbacks(instance, Rc::clone(&store));
    setup_collab_callbacks(instance, store);
}

/// 登记预览时钟的心跳回调（登记后立即停表，起停由 transport 决定）。
///
/// 心跳抓 store 的弱引用：时钟本身住在 `AppState` 里，抓强引用会构成循环。
fn setup_preview_clock(instance: &MainWindow, store: &Rc<RefCell<AppStore>>) {
    let (weak_store, weak_instance) = (Rc::downgrade(store), instance.as_weak());
    store
        .borrow()
        .state()
        .preview_clock
        .install(&weak_store, &weak_instance);
}

fn setup_collab_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let callbacks = instance.global::<CollabCallbacks>();

    let member_store = Rc::clone(&store);
    callbacks.on_copy_member_id(move || {
        let member_id = member_store.borrow().collab_bridge.member_id();
        if let Err(error) = member_store.borrow_mut().clipboard.write_text(&member_id) {
            log::error!("复制本机成员 ID 失败：{error:?}");
        }
    });

    let create_store = Rc::clone(&store);
    callbacks.on_create_session(move |name| {
        let mut app = create_store.borrow_mut();
        let result = {
            let AppStore { collab_bridge, state, .. } = &mut *app;
            collab_bridge.create_session(&mut state.project, name.as_str())
        };
        match result {
            Ok(()) => {
                app.sync_collab_ui();
                drop(app);
                crate::collab_bridge::start_event_pump(&create_store);
            }
            Err(error) => log::error!("创建协作会话失败：{error:?}"),
        }
    });

    let invite_store = Rc::clone(&store);
    callbacks.on_copy_invite(move |member_id| {
        let result = invite_store.borrow_mut().collab_bridge.make_invite(member_id.as_str());
        match result {
            Ok(invite) => {
                if let Err(error) = invite_store.borrow_mut().clipboard.write_text(&invite) {
                    log::error!("邀请码生成成功，但复制到剪贴板失败：{error:?}");
                }
            }
            Err(error) => log::error!("生成协作邀请失败：{error:?}"),
        }
    });

    let accept_store = Rc::clone(&store);
    callbacks.on_accept_invite(move |invite| {
        let mut app = accept_store.borrow_mut();
        let result = {
            let AppStore { collab_bridge, state, .. } = &mut *app;
            collab_bridge.accept_invite(&mut state.project, invite.as_str())
        };
        match result {
            Ok(()) => {
                app.sync_collab_ui();
                drop(app);
                crate::collab_bridge::start_event_pump(&accept_store);
            }
            Err(error) => log::error!("加入协作会话失败：{error:?}"),
        }
    });

    callbacks.on_leave_session(move || {
        let mut app = store.borrow_mut();
        let result = {
            let AppStore { collab_bridge, state, .. } = &mut *app;
            collab_bridge.leave(&mut state.project)
        };
        if let Err(error) = result {
            log::error!("离开协作会话失败：{error:?}");
        }
        app.sync_collab_ui();
    });
}

/// 设置视口回调：打开/追加/更换视频走系统文件对话框（rfd xdg-portal，异步不阻塞 UI）。
/// 选中后 dispatch 建管线（reducer 层），再连接 WGPU 输出并自动播放（渲染层）。
/// 对话框 future 在 `spawn_local` 中 await（同官方 gstreamer-player 示例的
/// 异步模式；portal 后端用 async-std 特性以脱离 tokio 上下文）。
/// 命中检测（`video-picked`/`video-context-picked`）为同步查询（点击需要立即结果）。
fn setup_video_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    // 一次性绑定 Rust 持有的 VecModel 到 VideoState 全局（共享 inner，后续自动通知）
    {
        let s = store.borrow();
        let vstate = instance.global::<crate::bridge::types::VideoState>();
        vstate.set_video_frames(slint::ModelRc::from(s.ui_models.video.frames.clone()));
        vstate.set_video_widths(slint::ModelRc::from(s.ui_models.video.widths.clone()));
        vstate.set_video_heights(slint::ModelRc::from(s.ui_models.video.heights.clone()));
        vstate.set_video_positions(slint::ModelRc::from(s.ui_models.video.positions.clone()));
        vstate.set_time_bars(slint::ModelRc::from(s.ui_models.video.time_bars.clone()));
    }

    // 打开/追加视频（首个自动成为主视频）：系统文件对话框（异步不阻塞 UI）。
    // Android 没有 rfd，走 MediaStore 查库 + Slint 自绘弹层（见 file_picker::android）。
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        #[cfg(not(target_os = "android"))]
        instance.global::<VideoCallbacks>().on_open_video(move || {
            let future = crate::file_picker::pick_video_file();
            let weak = weak.clone();
            let store = Rc::clone(&s);
            let _ = slint::spawn_local(async move {
                // 用户取消选择
                let Some(path) = future.await else {
                    return;
                };
                // 本地路径 → file URI（playbin 需要 URI）
                let Ok(uri) = url::Url::from_file_path(&path).map(|u| u.to_string()) else {
                    log::warn!("打开视频：路径无法转换为 URI：{}", path.display());
                    return;
                };
                dispatch_and_render(
                    &store,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::Append { uri },
                    )),
                    &weak,
                );
            });
        });
        #[cfg(target_os = "android")]
        instance.global::<VideoCallbacks>().on_open_video(move || {
            let weak = weak.clone();
            let store = Rc::clone(&s);
            let _ = slint::spawn_local(crate::file_picker::android::run_android_picker(
                weak,
                store,
                crate::file_picker::android::PickKind::Append,
            ));
        });
    }

    // 播放/暂停切换（时间线 Header 播放按钮）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_toggle_playback(move || {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::TogglePlayback,
                    )),
                    &weak,
                );
            });
    }

    // 点击视口视频 / 控制时间条：选中该视频（-1 = 取消选中）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_select_video(move |index, modifier| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::Select {
                            index: (index >= 0).then_some(index as usize),
                            modifier,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 控制时间条按下：先选中该视频（随后拖拽 seek 即作用到它）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_time_bar_pressed(move |index, is_batch| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekBegin {
                            index: index as usize,
                            is_batch,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 控制时间条拖拽：seek 当前选中（无选中则主视频）到指定秒
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_time_bar_dragged(move |time| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekPreview {
                            timestamp: (time * 1000.0) as i64,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 控制时间条拖拽结束：若拖拽前处于暂停，回到暂停（暂停预览）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_time_bar_released(move |_index| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekCommit,
                    )),
                    &weak,
                );
            });
    }

    // 音频波形可查看任意视频的音轨；定位该音轨不能改变全局选中视频。
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_seek_standalone(move |index, time| {
                if index < 0 {
                    return;
                }
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekStandalone {
                            index: index as usize,
                            timestamp: (time * 1000.0) as i64,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 右键非主视频"设为主视频"
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_set_main_video(move |index| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SetMain {
                            index: index as usize,
                        },
                    )),
                    &weak,
                );
            });
    }

    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_toggle_mute_video(move |index| {
                if index < 0 {
                    return;
                }
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::ToggleMute {
                            index: index as usize,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 右键任意视频"更换视频"：重新选择文件原地替换
    // （Android 分支同上，选完按原下标 Replace，见 file_picker::android）。
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        #[cfg(target_os = "android")]
        instance
            .global::<VideoCallbacks>()
            .on_replace_video(move |index| {
                let weak = weak.clone();
                let store = Rc::clone(&s);
                let _ = slint::spawn_local(crate::file_picker::android::run_android_picker(
                    weak,
                    store,
                    crate::file_picker::android::PickKind::Replace(index.max(0) as usize),
                ));
            });
        #[cfg(not(target_os = "android"))]
        instance
            .global::<VideoCallbacks>()
            .on_replace_video(move |index| {
                let future = crate::file_picker::pick_video_file();
                let weak = weak.clone();
                let store = Rc::clone(&s);
                let _ = slint::spawn_local(async move {
                    let Some(path) = future.await else {
                        return;
                    };
                    let Ok(uri) = url::Url::from_file_path(&path).map(|u| u.to_string()) else {
                        log::warn!("更换视频：路径无法转换为 URI：{}", path.display());
                        return;
                    };
                    dispatch_and_render(
                        &store,
                        Action::Media(crate::actions::media::MediaAction::Video(
                            crate::actions::media::VideoAction::Replace {
                                index: index as usize,
                                uri,
                            },
                        )),
                        &weak,
                    );
                });
            });
    }

    // 视口点击命中：重叠视频按选中顺序循环（同步查询）
    let s = Rc::clone(&store);
    instance
        .global::<VideoCallbacks>()
        .on_video_picked(move |x, y| -> i32 {
            s.borrow()
                .pick_video_at(x as f64, y as f64)
                .map(|i| i as i32)
                .unwrap_or(-1)
        });

    // 拖拽移动非主视频
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_video_moved(move |index, x, y| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::MoveVideo {
                            index: index as usize,
                            x: x as f64,
                            y: y as f64,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 视口命中：该点最上层的视频（拖拽移动起点/右键菜单目标）
    let s = Rc::clone(&store);
    instance
        .global::<VideoCallbacks>()
        .on_video_hit_top(move |x, y| -> i32 {
            s.borrow()
                .hit_top_video_at(x as f64, y as f64)
                .map(|i| i as i32)
                .unwrap_or(-1)
        });
}

/// 设置节点图编辑器回调：图操作走 `Action::NodeGraph` → dispatch → tag → 投影。
/// 只读查询（命中检测/找连线）保持同步调用（拖拽需要立即结果）。
fn setup_node_graph_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::node_graph::NodeGraphAction;
    use crate::bridge::types::GraphType;
    use aweft_model::node_graph::NodeGraphKind;

    let graph_type_of = |slint_type: GraphType| -> NodeGraphKind {
        match slint_type {
            GraphType::StyleGraph => NodeGraphKind::Style,
            GraphType::AppGraph => NodeGraphKind::App,
        }
    };

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_at(move |graph_type, node_index, x, y| {
            s.borrow()
                .find_socket_at(graph_type_of(graph_type), node_index as usize, x, y)
        });

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_node_dragged(move |graph_type, node_index, x, y| {
            s.borrow_mut()
                .set_node_position(graph_type_of(graph_type), node_index as usize, x, y);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_add_node(
        move |graph_type, template_index, x, y| -> i32 {
            let action = Action::NodeGraph(NodeGraphAction::AddNode {
                graph_type: graph_type_of(graph_type),
                template_index: template_index as usize,
                x,
                y,
            });
            dispatch_and_render(&s, action, &weak);
            // 投影已 sync：新节点在 nodes 模型末尾
            let store_ref = s.borrow();
            let count = match graph_type {
                GraphType::StyleGraph => store_ref.ui_models().style_graph.nodes.row_count(),
                GraphType::AppGraph => store_ref.ui_models().app_graph.nodes.row_count(),
            };
            (count as i32) - 1
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_delete_node(move |graph_type, node_index| {
            let action = Action::NodeGraph(NodeGraphAction::DeleteNode {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_find_link_for_input(move |graph_type, node_index, socket_index| -> i32 {
            s.borrow().find_link_for_input(
                graph_type_of(graph_type),
                node_index as usize,
                socket_index as usize,
            )
        });

    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_find_drop_socket(
        move |graph_type, x, y, data_type| -> crate::bridge::types::SocketHit {
            s.borrow()
                .find_drop_socket(graph_type_of(graph_type), x, y, data_type)
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_drag_ended(
            move |graph_type, node_index, socket_index, is_input, x, y| {
                let action = Action::NodeGraph(NodeGraphAction::SocketDragEnded {
                    graph_type: graph_type_of(graph_type),
                    node_index: node_index as usize,
                    socket_index: socket_index as usize,
                    is_input,
                    x,
                    y,
                });
                dispatch_and_render(&s, action, &weak);
            },
        );

    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_load_chain(move |graph_type, index| {
            s.borrow_mut()
                .load_chain(graph_type_of(graph_type), index as usize);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_new_chain(move |graph_type| -> i32 {
            let action = Action::NodeGraph(NodeGraphAction::NewChain {
                graph_type: graph_type_of(graph_type),
            });
            dispatch_and_render(&s, action, &weak);
            // 投影已刷新链菜单：新链在菜单末尾
            let store_ref = s.borrow();
            let count = match graph_type {
                GraphType::StyleGraph => store_ref.ui_models().style_graph.chain_items.row_count(),
                GraphType::AppGraph => store_ref.ui_models().app_graph.chain_items.row_count(),
            };
            (count as i32) - 1
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_menu_selected(
        move |graph_type, node_index, menu_index| {
            let action = Action::NodeGraph(NodeGraphAction::MenuSelected {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                menu_index: menu_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_option_selected(move |graph_type, node_index, socket_row, option_index| {
            let action = Action::NodeGraph(NodeGraphAction::SocketOptionSelected {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                option_index: option_index as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<NodeGraphCallbacks>().on_create_actor(
        move |graph_type, node_index, socket_row, name| {
            let action = Action::NodeGraph(NodeGraphAction::CreateActor {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                name: name.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_input_changed(move |graph_type, node_index, socket_row, text| {
            let action = Action::NodeGraph(NodeGraphAction::SocketInputChanged {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
                text: text.to_string(),
            });
            dispatch_and_render(&s, action, &weak);
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<NodeGraphCallbacks>()
        .on_socket_bool_toggled(move |graph_type, node_index, socket_row| {
            let action = Action::NodeGraph(NodeGraphAction::SocketBoolToggled {
                graph_type: graph_type_of(graph_type),
                node_index: node_index as usize,
                socket_index: socket_row as usize,
            });
            dispatch_and_render(&s, action, &weak);
        });
}

/// 设置区域类型切换回调 - 使用 Action 模式
fn setup_area_type_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    let s = Rc::clone(&store);

    instance.global::<AreaCallbacks>().on_switch_area_type(
        move |slint_area_key, slint_area_type| {
            let area_key = slint_area_key_to_key(&slint_area_key);
            let area_type = slint_area_type_to_rust(&slint_area_type);

            // 创建 Action
            let action = Action::Area(AreaAction::SwitchType {
                area_key,
                area_type,
            });

            // 分发 Action 并更新 UI
            dispatch_and_render(&s, action, &weak_instance);
        },
    );

    // 插件一列：枚举带不出插件 id，交的是 id，落成同一个 SwitchType（同一个 reducer、
    // 同一个 Tag），面板工厂由 Area 投影按 id 现取。
    let weak = instance.as_weak();
    instance
        .global::<AreaCallbacks>()
        .on_switch_area_type_plugin(move |slint_area_key, plugin_id| {
            let action = Action::Area(AreaAction::SwitchType {
                area_key: slint_area_key_to_key(&slint_area_key),
                area_type: crate::screen::area::AreaType::Plugin(plugin_id.to_string()),
            });
            dispatch_and_render(&store, action, &weak);
        });
}

fn setup_rendering_notifier_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let window = instance.window();
    let weak_instance = instance.as_weak();

    if let Err(e) = window.set_rendering_notifier(move |rendering_state, graphics_api| {
        let instance_opt = weak_instance.upgrade();

        match rendering_state {
            RenderingState::RenderingSetup => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_rendering_setup(instance, &store, graphics_api);
                }
            }
            RenderingState::BeforeRendering => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_before_rendering(instance, &store);
                }
            }
            RenderingState::AfterRendering => {
                let mut store_mut = store.borrow_mut();
                for session in store_mut.state_mut().video_sessions.iter_mut() {
                    session.wgpu_sink.after_rendering();
                }
                if let Some(subtitle) = store_mut.state_mut().subtitle.as_mut() {
                    subtitle.after_rendering();
                }
            }
            RenderingState::RenderingTeardown => {
                // Teardown 通常不需要实例
                handle_rendering_teardown(&store);
            }
            _ => (),
        }
    }) {
        log::error!("设置渲染通知器失败: {}", e);
    }
}

/// 设置音频接线：绑定 AudioState 音轨模型，并启动波形批次的 UI 线程 pump 任务。
/// 波形批次属于实时渲染增量例外，直接行级更新持久 VecModel（见
/// `docs/architecture/video-playback.md`），不走 reducer/depsgraph。
fn setup_audio_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    // 一次性绑定 Rust 持有的音轨模型到 AudioState 全局（共享 inner，后续自动通知）
    {
        let s = store.borrow();
        let astate = instance.global::<crate::bridge::types::AudioState>();
        astate.set_audio_tracks(slint::ModelRc::from(s.ui_models.audio.tracks.clone()));
        astate.set_playback_tracks(slint::ModelRc::from(s.ui_models.audio.playback_tracks.clone()));
    }

    // 波形后台 worker 经通道发批次，pump 任务在 UI 线程写模型（见
    // `store::audio::ui_pump_waveform_batches`）。发送端存入 AppState 供任务入队时克隆。
    {
        let (ui_sender, ui_receiver) = futures::channel::mpsc::unbounded::<
            crate::store::audio::WaveformBatch,
        >();
        let _ = slint::spawn_local(crate::store::audio::ui_pump_waveform_batches(
            Rc::clone(&store),
            ui_receiver,
        ));
        store.borrow_mut().state_mut().waveform_ui_sender = Some(ui_sender);
    }

    // 视图上报：Slint 绑定按导航求值，返回该 Area 的持久列模型（标识恒定，
    // Slint 侧不重建槽位）。副作用是登记视图并同步列，pump 据此重算新批次。
    {
        use crate::bridge::types::{AudioCallbacks, SlintKey};
        use crate::screen::area::ScrAreaKey;
        use crate::store::audio::AreaWaveformView;
        let s = Rc::clone(&store);
        instance.global::<AudioCallbacks>().on_waveform_view(
            move |area_key: SlintKey,
                  video_index: i32,
                  track_index: i32,
                  bucket_ms: f32,
                  first: i32,
                  count: i32| {
                if video_index < 0 || track_index < 0 || count <= 0 {
                    return slint::ModelRc::from(Rc::new(slint::VecModel::default()));
                }
                let key: ScrAreaKey = crate::bridge::types::slint_area_key_to_key(&area_key);
                let view = AreaWaveformView {
                    video_index: video_index as usize,
                    track_index: track_index as usize,
                    bucket_ms: (bucket_ms * 1_000.0).round().max(1.0) as usize,
                    first,
                    count: count.max(0) as usize,
                };
                let mut store = s.borrow_mut();
                slint::ModelRc::from(crate::store::audio::ui_sync_waveform_columns(
                    &mut store.ui_models.audio,
                    key,
                    view,
                ))
            },
        );
    }

    {
        use crate::actions::media::{MediaAction, VideoAction};
        use crate::bridge::types::AudioCallbacks;
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<AudioCallbacks>()
            .on_select_audio_track(move |video_index, track_index| {
                if video_index < 0 || track_index < 0 {
                    return;
                }
                dispatch_and_render(
                    &s,
                    crate::actions::Action::Media(MediaAction::Video(
                        VideoAction::SelectAudioTrack {
                            video_index: video_index as usize,
                            track_index: track_index as usize,
                        },
                    )),
                    &weak,
                );
            });
    }
}

/// 装好字幕拟合的后台通路：建通道、起 UI 线程 pump、把发送端交给 `AppState`。
///
/// 与波形批次同一套形状（后台增量直接写模型/浮层，不走 reducer）。没装这条通路的
/// 场合（无头测试）里 `request_rebuild` 会自己跳过，投影不会退回同步慢路径去冻界面。
fn setup_subtitle_fit(store: Rc<RefCell<AppStore>>) {
    let (ui_sender, ui_receiver) =
        futures::channel::mpsc::unbounded::<crate::store::subtitle_fit::FitMessage>();
    let _ = slint::spawn_local(crate::store::subtitle_fit::ui_pump_fit_messages(
        Rc::clone(&store),
        ui_receiver,
    ));
    store.borrow_mut().state_mut().fit_ui_sender = Some(ui_sender);
}

fn handle_rendering_teardown(store: &Rc<RefCell<AppStore>>) {
    crate::render::video_sink::teardown(&mut store.borrow_mut());
    if let Some(subtitle) = store.borrow_mut().state_mut().subtitle.as_mut() {
        subtitle.teardown();
    }
}

/// 逐帧字幕出图：与视频帧、播放时钟同属实时例外，直接写 Slint 图像属性，
/// 不经 reducer/depsgraph（脚本本身由 `projections::subtitle` 在用户操作时编好）。
///
/// 时间取动画域当前时间（播放头显示值）；排版基准取预览帧尺寸（主视频显示像素尺寸，
/// 无主视频时兜底 1080P），脚本里的事件时间、PlayRes 与覆盖层贴的矩形同一基准。
/// 光栅尺寸另取覆盖层在屏幕上的物理像素数（视口报来的逻辑尺寸 × 窗口缩放），
/// 字幕纹理因此与屏幕 1:1，放大画面时不会让 Slint 替我们做插值放大。
fn render_subtitle_frame(instance: &MainWindow, store: &mut AppStore) {
    let layout_size = store.state().preview_frame_size();
    let Some(pipeline) = store.state_mut().subtitle.as_mut() else {
        return;
    };

    let subtitle_state = instance.global::<crate::bridge::types::SubtitleState>();
    let scale_factor = instance.window().scale_factor() as f32;
    let on_screen_physical = (
        subtitle_state.get_overlay_width() * scale_factor,
        subtitle_state.get_overlay_height() * scale_factor,
    );

    let now_ms = (instance
        .global::<crate::bridge::types::AnimationState>()
        .get_current_time() as f64
        * 1000.0) as i64;
    let composition = match pipeline.render_at(now_ms, layout_size, on_screen_physical) {
        Ok(composition) => composition,
        Err(error) => {
            log::warn!("字幕出帧失败: {error}");
            return;
        }
    };

    match composition {
        Composition::Frame(image) => subtitle_state.set_current_subtitle_frame(image),
        // 当前时刻没有字幕：交回默认空图，覆盖层因此不可见
        Composition::Empty => subtitle_state.set_current_subtitle_frame(slint::Image::default()),
        // 输出槽位全忙或画面未变：保持上一帧，一个属性都不写
        Composition::Skipped => {}
    }
}

/// 逐帧视频通路：拉取各会话最新帧、同步播放位置、校正 transport 标量。
///
/// 新 sample/preroll 回调是播放渲染的唯一驱动；其它窗口重绘不查询媒体管线。
fn render_video_frames(store: &mut AppStore) {
    // 逐个会话拉取最新帧；帧消费与对应 UI 行立即配对，不创建临时帧数组。
    let session_count = store.state().video_sessions.len();
    let mut received_frame = false;
    for index in 0..session_count {
        let frame = store.state_mut().video_sessions[index]
            .wgpu_sink
            .fetch_next_frame();
        if let Some((image, width, height)) = frame {
            received_frame = true;
            store.ui_models.video.frames.set_row_data(index, image);
            if let (Some(w), Some(h)) = (width, height) {
                store.ui_models.video.widths.set_row_data(index, w as i32);
                store.ui_models.video.heights.set_row_data(index, h as i32);
            }
            render_sync_video_playback_time(store, index);
        }
    }

    if received_frame {
        crate::projections::ui_sync_active_video_time_realtime(store);
    }
    // Bus 在 EOS 时置一次脏标记并请求重绘。普通窗口重绘不遍历 transport 目标。
    let playback_dirty = store
        .state()
        .video_playback_dirty
        .swap(false, std::sync::atomic::Ordering::AcqRel);
    if playback_dirty && crate::reducers::media::media_refresh_video_playing_state(store) {
        crate::projections::ui_sync_video_playback_scalar(store);
    }
}

fn handle_before_rendering(instance: &MainWindow, store: &Rc<RefCell<AppStore>>) {
    let mut store_mut = store.borrow_mut();

    // 没有视频会话就不碰媒体管线；字幕照常出图
    if !store_mut.state().video_sessions.is_empty() {
        render_video_frames(&mut store_mut);
    }

    // 动画域当前时间已随上面的帧同步写完，字幕按同一时刻出图
    render_subtitle_frame(instance, &mut store_mut);
}

/// 播放中仅同步本次确有新帧的会话位置，并局部刷新其时间条模型。活跃视频
/// （选中无则主视频）随后驱动动画域当前时间。高频路径与视频帧一样直接写：
/// 不经 dispatch/depsgraph，但复用只更新 current-time 的实时投影出口。
/// 时间按视频帧时长向下取整——播放头逐帧跳动而非逐查询抖动。
fn render_sync_video_playback_time(store: &mut AppStore, index: usize) {
    if index >= store.state().video_sessions.len() {
        return;
    }
    // 拖拽时间条时 current_time 由媒体 reducer 的 seek 写入（鼠标目标位置），这里不要以
    // 实际解码（关键帧）位置覆盖，否则游标在目标/关键帧间反复跳动。
    let dragged = store
        .state()
        .video_seek_session
        .as_ref()
        .is_some_and(|drag| drag.targets.iter().any(|target| target.index == index));
    let bars = store.ui_models.video.time_bars.clone();
    let session = &mut store.state.video_sessions[index];
    if session.duration.is_none() {
        session.duration =
            crate::render::video_sink::gst_query_pipeline_duration_secs(&session.pipeline);
        if let Some(duration) = session.duration {
            crate::projections::ui_update_video_time_bar_duration(&bars, index, duration);
        }
    }
    let Some((secs, frame_dur)) = crate::reducers::media::gst_session_playing_position(session)
    else {
        return;
    };
    let snapped = match frame_dur {
        Some(d) if d > f64::EPSILON => (secs / d).floor() * d,
        _ => secs,
    };
    if !dragged && (session.current_time - snapped).abs() >= 0.0005 {
        session.current_time = snapped;
        crate::projections::ui_update_video_time_bar_value(&bars, index, snapped);
    }
}

fn handle_rendering_setup(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    graphics_api: &slint::GraphicsAPI,
) {
    log::info!("窗口初始化完成",);
    let windows_size = instance.window().size();
    // 现读一份 theme：store 里那份是建窗时读的，Android 上那一刻 scale_factor
    // 还是 1.0，真值要到本回调才有。
    let theme = crate::bridge::types::ThemeSettings::from_main_window(instance);
    let scale_factor = theme.get_scale_factor();
    // Slint 的 safe-area-insets 是逻辑像素，顶点坐标系是物理像素，走 theme 换算。
    let inset = {
        let px = |logical: f32| theme.to_physical_px(f64::from(logical)).round().max(0.0) as u32;
        crate::screen::primitives::Insets {
            left: px(instance.get_safe_inset_left()),
            top: px(instance.get_safe_inset_top()),
            right: px(instance.get_safe_inset_right()),
            bottom: px(instance.get_safe_inset_bottom()),
        }
    };
    log::info!(
        "窗口 {}x{}@{scale_factor:.2}，安全区（物理像素）左{} 上{} 右{} 下{}",
        windows_size.width,
        windows_size.height,
        inset.left,
        inset.top,
        inset.right,
        inset.bottom
    );
    let action = Action::Screen(crate::actions::screen::ScreenAction::InitializeLayout {
        width: windows_size.width,
        height: windows_size.height,
        scale_factor,
        inset,
    });
    dispatch_and_render(store, action, &instance.as_weak());

    // 缓存渲染器 GPU 句柄：GraphicsAPI 仅在本回调期间有效，克隆出的
    // Instance/Device/Queue 供之后按需连接视频 Sink（打开视频时，reducers/media）。
    match graphics_api {
        slint::GraphicsAPI::WGPU30 {
            instance,
            device,
            queue,
            ..
        } => {
            let handles = (
                std::sync::Arc::new(instance.clone()),
                std::sync::Arc::new(device.clone()),
                std::sync::Arc::new(queue.clone()),
            );
            store.borrow_mut().state_mut().video_wgpu = Some(handles.clone());
            // 字幕合成器与视频输出共用同一对 GPU 句柄
            store.borrow_mut().state_mut().subtitle =
                Some(crate::render::subtitle::SubtitlePipeline::new(&handles));
            // 通路建好之前打过的字幕 tag 不会再来了：按当前真相补编一次脚本
            crate::projections::subtitle_rebuild_script(&mut store.borrow_mut());
        }
        _ => log::warn!("非 WGPU 渲染器，视频输出不可用"),
    }
}

/// 媒体选择弹层的选择/取消（仅 Android）：取走 AppStore 里的等待，
/// 把下标（或取消）发回给 run_android_picker 的 oneshot。弹层收起由
/// run_android_picker 在收到回执后统一做，这里只管送信。
#[cfg(target_os = "android")]
fn setup_media_picker_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let s = Rc::clone(&store);
    instance
        .global::<MediaPickerCallbacks>()
        .on_pick_media(move |index| {
            if let Some(pending) = s.borrow_mut().pending_media_pick.take() {
                let _ = pending.reply.send(Some(index.max(0) as usize));
            }
        });
    let s = Rc::clone(&store);
    instance
        .global::<MediaPickerCallbacks>()
        .on_cancel_media(move || {
            if let Some(pending) = s.borrow_mut().pending_media_pick.take() {
                let _ = pending.reply.send(None);
            }
        });
}

/// 空实现：桌面没有媒体选择弹层（走 rfd），保持 setup_callbacks 调用点无分叉。
#[cfg(not(target_os = "android"))]
fn setup_media_picker_callbacks(_instance: &MainWindow, _store: Rc<RefCell<AppStore>>) {}

#[cfg(not(target_os = "android"))]
fn setup_winit_window_event_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let window = instance.window();
    let weak_instance = instance.as_weak();

    window.on_winit_window_event(move |_slint_window, event| {
        let instance_opt = weak_instance.upgrade();
        match event {
            WindowEvent::Resized(new_size) => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_resized(instance, &store, &new_size);
                }
            }
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(instance) = instance_opt.as_ref() {
                    handle_scale_factor_changed(instance, &store, *scale_factor);
                }
            }
            _ => {}
        }
        // 确保事件继续执行
        EventResult::Propagate
    });
}

#[cfg(not(target_os = "android"))]
fn handle_resized(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    new_size: &winit::dpi::PhysicalSize<u32>,
) {
    log::debug!("窗口大小发生改变: {}x{}", new_size.width, new_size.height);

    let action = Action::Screen(crate::actions::screen::ScreenAction::WindowResized {
        width: new_size.width,
        height: new_size.height,
    });

    dispatch_and_render(store, action, &instance.as_weak());
}

#[cfg(not(target_os = "android"))]
fn handle_scale_factor_changed(
    instance: &MainWindow,
    store: &Rc<RefCell<AppStore>>,
    scale_factor: f64,
) {
    log::info!("检测到屏幕缩放因子变化: {:.2}", scale_factor);
    let action =
        Action::Screen(crate::actions::screen::ScreenAction::ScaleFactorChanged { scale_factor });

    dispatch_and_render(store, action, &instance.as_weak());
}

fn setup_actionzone_drag_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    instance.global::<DragCallbacks>().on_actionzone_drag(
        move |area_key, corner, offset_x, offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::ActionZone(
                crate::actions::drag::ActionZoneDrag::Update {
                    area_key,
                    corner,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

fn setup_actionzone_drag_end_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<DragCallbacks>()
        .on_actionzone_drag_end(move |area_key, is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::ActionZone(
                crate::actions::drag::ActionZoneDrag::End {
                    area_key,
                    is_cancel,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_edge_drag_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<DragCallbacks>().on_edge_drag(
        move |vert_keys,
              // 此坐标是相较于 UI Edge 矩形左上角的
              offset_x,
              offset_y| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::Update {
                    vert_keys,
                    offset_x,
                    offset_y,
                },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}
fn setup_edge_drag_end_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<DragCallbacks>()
        .on_edge_drag_end(move |is_cancel| {
            let action = Action::Drag(crate::actions::drag::DragAction::Edge(
                crate::actions::drag::EdgeDrag::End { is_cancel },
            ));

            dispatch_and_render(&store, action, &weak_instance);
        });
}

/// 设置分组折叠/展开回调
fn setup_toggle_group_collapsed_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_toggle_group_collapsed(move |group_id_str| {
            use aweft_model::ids::ItemId;
            let Ok(group_id) = uuid::Uuid::parse_str(group_id_str.as_str()).map(ItemId) else {
                warn!("无效的 group_id: {}", group_id_str);
                return;
            };

            let action = Action::Sequence(
                crate::actions::sequence::SequenceAction::ToggleGroupCollapsed { group_id },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_sequence_timecode_editor_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<SequenceCallbacks>().on_timecode_input(
        move |event_id_str, is_start_time, text, is_batch| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            log::debug!("输入文本: {text}");
            if is_batch {
                // 遍历所有选中的 event 项，应用时间修改
                let selected_ids = store
                    .borrow()
                    .ui_models()
                    .sequence_state
                    .directly_selected_items();
                for &id in &selected_ids {
                    let action = Action::Sequence(SequenceAction::InputTimecode {
                        event_id: id,
                        is_start_time,
                        text: text.to_string(),
                    });
                    // dispatch each
                    dispatch_and_render(&store, action, &weak_instance);
                }
            } else {
                let action = Action::Sequence(SequenceAction::InputTimecode {
                    event_id,
                    is_start_time,
                    text: text.to_string(),
                });
                dispatch_and_render(&store, action, &weak_instance);
            }
        },
    );
}
fn setup_sequence_timecode_step_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    let register = |setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString, bool, bool)>),
                    build: fn(
        aweft_model::ids::ItemId,
        bool,
        bool,
    ) -> crate::actions::sequence::SequenceAction| {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        setter(
            instance,
            Box::new(move |event_id_str, is_start_time, is_batch| {
                use aweft_model::ids::ItemId;
                let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                    warn!("无效的 event_id: {}", event_id_str);
                    return;
                };
                let action = Action::Sequence(build(event_id, is_start_time, is_batch));
                dispatch_and_render(&s, action, &w);
            }),
        );
    };

    register(
        |inst, cb| inst.global::<SequenceCallbacks>().on_increment_timecode(cb),
        |event_id, is_start_time, is_batch| {
            crate::actions::sequence::SequenceAction::IncrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            }
        },
    );

    register(
        |inst, cb| inst.global::<SequenceCallbacks>().on_decrement_timecode(cb),
        |event_id, is_start_time, is_batch| {
            crate::actions::sequence::SequenceAction::DecrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            }
        },
    );
}

fn setup_sequence_drag_timecode_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    let register =
        |setter: fn(&MainWindow, Box<dyn FnMut(slint::SharedString, bool, f32, bool)>),
         build: fn(
            aweft_model::ids::ItemId,
            bool,
            f32,
            bool,
        ) -> crate::actions::sequence::SequenceAction| {
            let s = Rc::clone(&store);
            let w = weak_instance.clone();
            setter(
                instance,
                Box::new(move |event_id_str, is_start_time, delta_x, is_batch| {
                    use aweft_model::ids::ItemId;
                    let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId)
                    else {
                        warn!("无效的 event_id: {}", event_id_str);
                        return;
                    };
                    let action =
                        Action::Sequence(build(event_id, is_start_time, delta_x, is_batch));
                    dispatch_and_render(&s, action, &w);
                }),
            );
        };

    register(
        |inst, cb| {
            inst.global::<SequenceCallbacks>()
                .on_drag_timecode_preview(cb)
        },
        |event_id, is_start_time, delta_x, is_batch| {
            crate::actions::sequence::SequenceAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            }
        },
    );

    register(
        |inst, cb| {
            inst.global::<SequenceCallbacks>()
                .on_drag_timecode_commit(cb)
        },
        |event_id, is_start_time, delta_x, is_batch| {
            crate::actions::sequence::SequenceAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            }
        },
    );
}
fn setup_sequence_drag_timecode_cancel_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_drag_timecode_cancel(move |event_id_str, is_start_time, is_batch| {
            use aweft_model::ids::ItemId;
            let Ok(event_id) = uuid::Uuid::parse_str(event_id_str.as_str()).map(ItemId) else {
                warn!("无效的 event_id: {}", event_id_str);
                return;
            };
            let action = Action::Sequence(
                crate::actions::sequence::SequenceAction::DragTimecodeCancel {
                    event_id,
                    is_start_time,
                    is_batch,
                },
            );

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_sequence_selection_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use aweft_model::ids::ItemId;
    let weak_instance = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_select_row(move |item_id_str, modifier| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            let action = Action::Sequence(SequenceAction::SelectRow { item_id, modifier });
            dispatch_and_render(&s, action, &weak_instance);
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_marquee_select(move |x_lo, x_hi, row_lo, row_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::MarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                }),
                &weak,
            );
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_deselect_all(move || {
            dispatch_and_render(&s, Action::Sequence(SequenceAction::DeselectAll), &weak);
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_block_dragged(move |item_id_str, delta_x| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::TimelineBlockDragPreview { item_id, delta_x }),
                &weak,
            );
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_block_drag_commit(move |item_id_str, delta_x| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::TimelineBlockDragCommit { item_id, delta_x }),
                &weak,
            );
        });
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<SequenceCallbacks>()
        .on_timeline_block_drag_cancel(move |item_id_str| {
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                warn!("无效的 item_id: {}", item_id_str);
                return;
            };
            dispatch_and_render(
                &s,
                Action::Sequence(SequenceAction::TimelineBlockDragCancel { item_id }),
                &weak,
            );
        });
}

fn setup_sequence_add_line_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance.global::<SequenceCallbacks>().on_add_line(
        move |item_id_str, _start_time_str, end_time_str, _text| {
            use aweft_model::ids::ItemId;
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                log::warn!("[callback] 无效的 item_id: {}", item_id_str);
                return;
            };

            let end_time = if let Some(end_time) = parse_time(&end_time_str) {
                end_time
            } else {
                log::warn!("[callback] 无效的 : end_time_str {}", end_time_str);
                return;
            };

            let action = Action::Sequence(crate::actions::sequence::SequenceAction::AddLine {
                item_id,
                end_time,
                actor: None,
                // 新行不要延续参考行文本
                // TODO: 如果为明显序列 （1,2,3）可以尝试按规则延续
                // text: text.into(),
                text: String::new(),
                segment_timing: None,
                style: None,
            });

            dispatch_and_render(&store, action, &weak_instance);
        },
    );
}

fn setup_sequence_add_group_callback(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_add_group(move |item_id_str| {
            use aweft_model::ids::ItemId;
            let Ok(item_id) = uuid::Uuid::parse_str(item_id_str.as_str()).map(ItemId) else {
                log::warn!("[callback] 无效的 item_id: {}", item_id_str);
                return;
            };

            let action =
                Action::Sequence(crate::actions::sequence::SequenceAction::AddGroup { item_id });

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_sequence_delete_selected_items_callback(
    instance: &MainWindow,
    store: Rc<RefCell<AppStore>>,
) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_delete_selected_items(move || {
            let action = Action::Sequence(crate::actions::sequence::SequenceAction::DeleteItems);

            dispatch_and_render(&store, action, &weak_instance);
        });
}

fn setup_sequence_clipboard_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();

    {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        instance
            .global::<SequenceCallbacks>()
            .on_copy_items(move || {
                let action = Action::Sequence(SequenceAction::CopyItems);
                dispatch_and_render(&s, action, &w);
            });
    }

    {
        let s = Rc::clone(&store);
        let w = weak_instance.clone();
        instance
            .global::<SequenceCallbacks>()
            .on_paste_items(move |anchor_item_id_str| {
                use aweft_model::ids::ItemId;
                let Ok(anchor) = uuid::Uuid::parse_str(anchor_item_id_str.as_str()).map(ItemId)
                else {
                    warn!("无效的 paste target item_id: {}", anchor_item_id_str);
                    return;
                };
                let action = Action::Sequence(SequenceAction::PasteItems { anchor });
                dispatch_and_render(&s, action, &w);
            });
    }

    {
        let s = Rc::clone(&store);
        let w = weak_instance;
        instance
            .global::<SequenceCallbacks>()
            .on_cut_items(move || {
                let action = Action::Sequence(SequenceAction::CutItems);
                dispatch_and_render(&s, action, &w);
            });
    }
}

fn setup_sequence_ungroup_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let weak_instance = instance.as_weak();
    instance
        .global::<SequenceCallbacks>()
        .on_ungroups(move |group_item_id_str| {
            let Ok(group_item_id) = uuid::Uuid::parse_str(group_item_id_str.as_str()).map(ItemId)
            else {
                log::warn!("[callback] ungroup 无效的 item_id: {}", group_item_id_str);
                return;
            };

            let action = Action::Sequence(crate::actions::sequence::SequenceAction::UnGroup {
                group_item_id,
            });

            dispatch_and_render(&store, action, &weak_instance);
        });
}

/// 统一的 dispatch 和渲染逻辑。
///
/// reducer 修改数据并向依赖图打 tag；`evaluate_depsgraph` 只重算受影响的投影。
 pub(crate) fn dispatch_and_render(
    store: &Rc<RefCell<AppStore>>,
    action: Action,
    _weak_instance: &slint::Weak<MainWindow>,
) {
    let mut store_mut = store.borrow_mut();

    // 1. 分发 Action：修改数据 + 打 tag
    if let Err(e) = store_mut.dispatch(action) {
        log::error!("❌ Action 处理失败: {}", e);
        return;
    }

    // 2. 依赖图求值：只重算受影响投影
    store_mut.evaluate_depsgraph();
}

/// 设置动画编辑器（曲线编辑器）回调：全部交互走 `Action::Animation` → dispatch → tag → 投影。
fn setup_animation_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::animation::{AnimationAction, graph::GraphAction};

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_pressed(move |curve, key, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::KeyframePressed {
                    curve: curve as usize,
                    key: key as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<AnimationCallbacks>().on_keyframe_dragged(
        move |curve, key, delta_time, delta_value, mode| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::KeyframeDragged {
                    curve: curve as usize,
                    key: key as usize,
                    delta_time,
                    delta_value,
                    mode,
                })),
                &weak,
            );
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_released(move |curve, key, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::KeyframeReleased {
                    curve: curve as usize,
                    key: key as usize,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DeselectKeyframes)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<AnimationCallbacks>().on_handle_pressed(
        move |curve, key, is_in, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::HandlePressed {
                    curve: curve as usize,
                    key: key as usize,
                    is_in,
                    modifier,
                })),
                &weak,
            );
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance.global::<AnimationCallbacks>().on_handle_dragged(
        move |curve, key, is_in, delta_time, delta_value, mode| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::HandleDragged {
                    curve: curve as usize,
                    key: key as usize,
                    is_in,
                    delta_time,
                    delta_value,
                    mode,
                })),
                &weak,
            );
        },
    );

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_handle_released(move |curve, key, is_in, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::HandleReleased {
                    curve: curve as usize,
                    key: key as usize,
                    is_in,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_graph_marquee_select(move |x_lo, x_hi, y_lo, y_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::GraphMarqueeSelect {
                    x_lo,
                    x_hi,
                    y_lo,
                    y_hi,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_pressed(move |curve, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelPressed {
                    curve: curve as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelDeselectAll)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_graph_search_changed(move |text| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::GraphSearchChanged {
                    text: text.to_string(),
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_search_changed(move |text| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSearchChanged {
                    text: text.to_string(),
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_channel_pressed(move |view_row, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeChannelPressed {
                    view_row: view_row as usize,
                    modifier,
                })),
                &weak,
            );
        });

    // —— 摄影表（Dope Sheet）：关键帧（共用曲线编辑器数据/选中，拖拽只调时间） ——
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_pressed(move |curve, key, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframePressed {
                    curve: curve as usize,
                    key: key as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_dragged(move |curve, key, delta_time| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeDragged {
                    curve: curve as usize,
                    key: key as usize,
                    delta_time,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_released(move |curve, key, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeReleased {
                    curve: curve as usize,
                    key: key as usize,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_keyframe_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeKeyframeDeselectAll)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_marquee_select(move |x_lo, x_hi, row_lo, row_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeMarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                })),
                &weak,
            );
        });

    // —— 摄影表：汇总条带（聚合所有通道同一时刻的关键帧；跨通道同步拖拽） ——
    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_summary_pressed(move |index, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryPressed {
                    index: index as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_summary_dragged(move |index, delta_time| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryDragged {
                    index: index as usize,
                    delta_time,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_dope_summary_released(move |index, moved| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DopeSummaryReleased {
                    index: index as usize,
                    moved,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_curve_color_mode_changed(move |curve, mode| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::CurveColorModeChanged {
                    curve: curve as usize,
                    mode,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_keyframe_interpolation_changed(move |interp| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(
                    GraphAction::KeyframeInterpolationChanged {
                        interpolation: interp,
                    },
                )),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_visible(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleVisible {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_modifier(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleModifier {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_active(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleActive {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_channel_toggle_editable(move |curve| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ChannelToggleEditable {
                    curve: curve as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_set_current_time(move |time| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::SetCurrentTime { time })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_view_changed(move |zoom_x, zoom_y| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ViewChanged {
                    zoom_x,
                    zoom_y,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_action_selected(move |index| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::ActionSelected {
                    index,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_drag_cancel(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Graph(GraphAction::DragCancel)),
                &weak,
            );
        });
}

/// 设置 NLA 编辑器回调：片断/轨道交互走 `Action::Nla` → dispatch → tag → 投影。
/// 命中检测（`nla-strip-pick`）为同步查询（拖拽需要立即结果）+ 选中修改：
/// 数据层就地更新后手动打 tag 并 evaluate（与 dispatch 同管线）。
fn setup_nla_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    use crate::actions::animation::{AnimationAction, nla::NlaAction};

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_pressed(move |track, strip, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
                    track: track as usize,
                    strip: strip as usize,
                    modifier,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_dragged(move |delta_x, delta_row| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripDragged {
                    delta_x,
                    delta_row,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_released(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripReleased)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_deselect_all(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripDeselectAll)),
                &weak,
            );
        });

    // 命中检测：纯命中（只读，不改数据层）→ 若命中则走标准 dispatch 管线选中并记录拖拽起点。
    // 返回编码供画布判定拖拽起点（track*10000+strip），未命中 -1。
    // 命中查询与 dispatch 分作用域，避免 RefCell 借用重叠。
    let s = Rc::clone(&store);
    let weak = instance.as_weak();
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_pick(move |x, row, modifier| -> i32 {
            let hit = {
                let store_ref = s.borrow();
                store_ref.ui_models.animation.nla.strip_hit_test(x, row)
            };
            if let Some((track, strip)) = hit {
                dispatch_and_render(
                    &s,
                    Action::Animation(AnimationAction::Nla(NlaAction::StripPressed {
                        track,
                        strip,
                        modifier,
                    })),
                    &weak,
                );
                (track as i32) * 10000 + (strip as i32)
            } else {
                -1
            }
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_track_toggle_enabled(move |track| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::ToggleTrackEnabled {
                    track: track as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_track_toggle_locked(move |track| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::ToggleTrackLocked {
                    track: track as usize,
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_track_push_down(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::PushDown)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_search_changed(move |text| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::SearchChanged {
                    text: text.to_string(),
                })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_anim_selected(move |index| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::AnimDataSelected { index })),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_strip_drag_cancel(move || {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::StripDragCancel)),
                &weak,
            );
        });

    let weak = instance.as_weak();
    let s = Rc::clone(&store);
    instance
        .global::<AnimationCallbacks>()
        .on_nla_marquee_select(move |x_lo, x_hi, row_lo, row_hi, modifier| {
            dispatch_and_render(
                &s,
                Action::Animation(AnimationAction::Nla(NlaAction::MarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                })),
                &weak,
            );
        });
}
