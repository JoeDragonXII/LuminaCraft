use eframe::egui::{self, Vec2};

pub(super) struct Status<'a> {
    pub message: &'a str,
    pub metadata: &'a str,
    pub elapsed_ms: u128,
    pub blocking: bool,
    pub motion: bool,
    pub committing: bool,
    pub cancelled: bool,
}

/// Preview renders must never change the available photo viewport. Long tasks
/// may show feedback, but all controls stay inside the same reserved footer.
pub(super) fn show(ui: &mut egui::Ui, status: Status<'_>) -> bool {
    let mut cancel = false;
    let panel = egui::Panel::bottom("status")
        .exact_size(36.0).resizable(false).show_separator_line(false)
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if status.blocking {
                    if status.motion { ui.add(egui::Spinner::new().size(16.0)); }
                    let label = if status.committing { "正在写入…" } else if status.cancelled { "取消中…" } else { "取消" };
                    let r = ui.add_enabled_ui(!status.committing && !status.cancelled, |ui| ui.add_sized([90.0,24.0],egui::Button::new(label))).inner;
                    lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.task.cancel",r.rect);
                    cancel = r.clicked();
                }
                let width=(ui.available_width()-280.0).max(0.0);
                ui.add_sized([width,20.0],egui::Label::new(status.message).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                    ui.small(format!("预览 {}ms",status.elapsed_ms));
                    ui.small(status.metadata);
                });
            });
        });
    lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.status",panel.response.rect);
    cancel
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn footer_transitions_preserve_the_entire_canvas() {
        for size in [[1380.0,900.0],[980.0,680.0]] {
            let ctx=egui::Context::default();
            let mut baseline=None;
            for (blocking,motion,committing,cancelled) in [(false,true,false,false),(true,true,false,false),(true,true,true,false),(true,false,false,true),(false,true,false,false)] {
                for _ in 0..3 {
                    let output=ctx.run_ui(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size.into())),..Default::default()},|ui| {
                        crate::chrome::apply(ui,motion);
                        show(ui,Status{message:if blocking{"Rendering a long task message"}else{""},metadata:"4608 × 3072 · RAW",elapsed_ms:35,blocking,motion,committing,cancelled});
                        let rect=ui.available_rect_before_wrap();
                        if let Some(expected)=baseline {assert_eq!(rect,expected,"footer state changed canvas bounds");}else{baseline=Some(rect);}
                        let registry=lightcraft_ui_egui::widgets::take_registry(ui.ctx());
                        let footer=registry.iter().find(|(id,_)|id=="luminacraft.status").unwrap().1;
                        assert_eq!(registry.iter().any(|(id,_)|id=="luminacraft.task.cancel"),blocking,"only long tasks expose cancellation");
                        for (_,r) in registry.iter().filter(|(id,_)|id=="luminacraft.task.cancel") {assert!(footer.contains_rect(*r),"cancel escaped footer");}
                    });output.drop_without_applying_deltas();
                }
            }
        }
    }
}
