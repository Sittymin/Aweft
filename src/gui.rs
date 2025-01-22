use eframe::egui;
use rfd::FileDialog;
use saffron_ass::types::*;
use std::{path::PathBuf, sync::Arc};

pub fn gui() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions::default();

    eframe::run_native(
        "Saffron",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}

struct MyApp {
    events: Option<Vec<Event>>,
}

impl MyApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();

        // 添加中文字体
        fonts.font_data.insert(
            "霞鹜新晰黑".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../assets/LXGWNeoXiHei.ttf"
            ))),
        );

        // 将中文字体设置为首选字体
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "霞鹜新晰黑".to_owned());

        // cc.egui_ctx 是 egui::Context,可以在这里自定义字体和样式
        cc.egui_ctx.set_fonts(fonts);

        if let Ok(ass_file) = saffron_ass::load_ass("/mnt/CT1000MX500SSD1/[LittleBakas! x ANK-Raws&LKSUB x JYFanSub] 君の名は。 [BD 3840x2160 HEVC FLAC]/[LittleBakas! x ANK-Raws&LKSUB x JYFanSub] 君の名は。 [BD 3840x2160 HEVC-yuv444p10 FLACx4 PGSx4 Chap].SC精.ass") {
            Self {
                events: Some(ass_file.events),
            }
        } else {
            Self {
                events: None
            }
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(events) = &mut self.events {
                let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
                let row_height = text_height * 1.5;
                // 设置固定列宽
                let column_widths = [50.0, 80.0, 80.0, 100.0, 400.0];

                let header_bg = ui.style().visuals.widgets.noninteractive.bg_fill;

                // 添加一个固定的表头
                ui.horizontal(|ui| {
                    // 在表头下添加底边框
                    let header_rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(header_rect, 0.0, header_bg);

                    // Layer
                    ui.scope(|ui| {
                        ui.set_min_width(column_widths[0]);
                        ui.strong("层级");
                    });

                    // Start Time
                    ui.scope(|ui| {
                        ui.set_min_width(column_widths[1]);
                        ui.strong("开始时间");
                    });

                    // End Time
                    ui.scope(|ui| {
                        ui.set_min_width(column_widths[2]);
                        ui.strong("结束时间");
                    });

                    // Name
                    ui.scope(|ui| {
                        ui.set_min_width(column_widths[3]);
                        ui.strong("人物");
                    });

                    // Raw Text
                    ui.scope(|ui| {
                        ui.set_min_width(column_widths[4]);
                        ui.strong("文本");
                    });
                });

                // 添加一个分隔线
                ui.add(egui::Separator::default().spacing(1.0));

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show_rows(ui, row_height, events.len(), |ui, row_range| {
                        for row_idx in row_range {
                            // 先获取所需的值
                            let event_type = events[row_idx].event_type.clone();
                            let layer = events[row_idx].layer;
                            let start = events[row_idx].start.to_string();
                            let end = events[row_idx].end.to_string();
                            let name = events[row_idx].name.as_deref().unwrap_or("-").to_owned();
                            let mut text = events[row_idx].raw_text.clone();

                            let changed = ui
                                .horizontal(|ui| {
                                    let mut changed = false;

                                    match event_type {
                                        EventType::Comment => {
                                            let rect = ui.available_rect_before_wrap();
                                            ui.painter().rect_filled(
                                                rect,
                                                0.0,
                                                egui::Color32::from_rgba_unmultiplied(
                                                    150, 255, 150, 32,
                                                ),
                                            );

                                            ui.scope(|ui| {
                                                ui.set_min_width(column_widths.iter().sum());
                                                changed = ui
                                                    .add(
                                                        // WARN: 还不可以输入中文
                                                        egui::TextEdit::singleline(&mut text)
                                                            .desired_width(f32::INFINITY),
                                                    )
                                                    .changed();
                                            });
                                        }
                                        EventType::Dialogue => {
                                            ui.scope(|ui| {
                                                ui.set_min_width(column_widths[0]);
                                                ui.label(layer.to_string());
                                            });

                                            ui.scope(|ui| {
                                                ui.set_min_width(column_widths[1]);
                                                ui.label(&start);
                                            });

                                            ui.scope(|ui| {
                                                ui.set_min_width(column_widths[2]);
                                                ui.label(&end);
                                            });

                                            ui.scope(|ui| {
                                                ui.set_min_width(column_widths[3]);
                                                ui.label(&name);
                                            });

                                            ui.scope(|ui| {
                                                ui.set_min_width(column_widths[4]);
                                                changed = ui
                                                    .add(
                                                        egui::TextEdit::singleline(&mut text)
                                                            .desired_width(f32::INFINITY),
                                                    )
                                                    .changed();
                                            });
                                        }
                                    }
                                    changed
                                })
                                .inner;

                            // 如果文本改变了，更新原始数据
                            if changed {
                                events[row_idx].raw_text = text;
                            }
                        }
                    });
            } else {
                // 未选择文件时显示提示
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label("请选择 ASS 字幕文件");
                });
            }
        });
    }
}
