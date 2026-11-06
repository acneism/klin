//! Кнопка и панель загрузок — по образцу панели загрузок Firefox.

use egui::{Align, Key, Layout, Rect, RichText, Vec2};
use klin_shared::download::{Download, DownloadState};

use crate::Action;
use crate::icons::{self, Icon};

/// Размер в единицах, как в списке загрузок Firefox: «4,2 МБ».
pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["байт", "КБ", "МБ", "ГБ"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

/// Строка состояния под именем файла.
pub(crate) fn status_text(download: &Download) -> String {
    match &download.state {
        DownloadState::InProgress {
            received,
            total: Some(total),
        } => {
            format!("{} из {}", format_size(*received), format_size(*total))
        }
        DownloadState::InProgress {
            received,
            total: None,
        } => format_size(*received),
        DownloadState::Finished { size } => format!("Завершено — {}", format_size(*size)),
        DownloadState::Failed { .. } => "Ошибка".to_owned(),
        DownloadState::Canceled => "Отменено".to_owned(),
    }
}

/// Доля выполнения общей суммы идущих загрузок — для полоски на кнопке.
fn overall_progress(downloads: &[Download]) -> Option<f32> {
    let (mut got, mut all, mut any) = (0u64, 0u64, false);
    for d in downloads {
        if let DownloadState::InProgress { received, total } = d.state {
            any = true;
            got += received;
            all += total.unwrap_or(received.max(1));
        }
    }
    any.then(|| {
        if all == 0 {
            0.0
        } else {
            got as f32 / all as f32
        }
    })
}

/// Кнопка на панели навигации. Видна, только если в сеансе были загрузки
/// (`browser.download.autohideButton`).
pub(crate) fn button(ui: &mut egui::Ui, downloads: &[Download]) -> Option<egui::Response> {
    if downloads.is_empty() {
        return None;
    }
    let response = icons::button(ui, true, Icon::Download, 30.0).on_hover_text("Загрузки (Ctrl+J)");
    if let Some(progress) = overall_progress(downloads) {
        let r = response.rect;
        let bar = Rect::from_min_size(
            egui::pos2(r.left() + 6.0, r.bottom() - 5.0),
            Vec2::new(r.width() - 12.0, 2.5),
        );
        let painter = ui.painter();
        painter.rect_filled(bar, 1.0, ui.visuals().widgets.inactive.bg_fill);
        let mut done = bar;
        done.set_width(bar.width() * progress.clamp(0.0, 1.0));
        painter.rect_filled(done, 1.0, ui.visuals().selection.bg_fill);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    Some(response)
}

/// Панель под кнопкой. Возвращает `false`, когда панель нужно закрыть.
pub(crate) fn panel(
    ctx: &egui::Context,
    anchor: Rect,
    downloads: &[Download],
    actions: &mut Vec<Action>,
) -> bool {
    let width = 360.0;
    let pos = egui::pos2((anchor.right() - width).max(4.0), anchor.bottom() + 4.0);
    let area = egui::Area::new(egui::Id::new("klin-downloads-panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            // Новые сверху, как в Firefox.
                            for download in downloads.iter().rev() {
                                row(ui, download, actions);
                                ui.separator();
                            }
                        });
                    if downloads
                        .iter()
                        .any(|d| !matches!(d.state, DownloadState::InProgress { .. }))
                        && ui.button("Очистить загрузки").clicked()
                    {
                        actions.push(Action::ClearDownloads);
                    }
                });
        });
    let pressed_outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .press_origin()
                .is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p))
    });
    !(pressed_outside || ctx.input(|i| i.key_pressed(Key::Escape)))
}

fn row(ui: &mut egui::Ui, download: &Download, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(270.0);
            let name = RichText::new(download.file_name()).strong();
            let label = ui.add(
                egui::Label::new(name)
                    .truncate()
                    .sense(egui::Sense::click()),
            );
            let finished = matches!(download.state, DownloadState::Finished { .. });
            if finished && label.clicked() {
                actions.push(Action::OpenDownload(download.path.clone()));
            }
            label.on_hover_text(download.path.display().to_string());
            if let DownloadState::InProgress { received, total } = download.state {
                let fraction = total.map(|t| {
                    if t == 0 {
                        0.0
                    } else {
                        received as f32 / t as f32
                    }
                });
                let bar = egui::ProgressBar::new(fraction.unwrap_or(0.0)).desired_height(4.0);
                ui.add(if fraction.is_none() {
                    bar.animate(true)
                } else {
                    bar
                });
            }
            let status = status_text(download);
            let status = match &download.state {
                DownloadState::Failed { error } => {
                    ui.label(RichText::new(status).small()).on_hover_text(error)
                }
                _ => ui.label(RichText::new(status).small().weak()),
            };
            let _ = status;
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            match download.state {
                DownloadState::InProgress { .. } => {
                    if icons::button(ui, true, Icon::Close, 26.0)
                        .on_hover_text("Отменить")
                        .clicked()
                    {
                        actions.push(Action::CancelDownload(download.id));
                    }
                }
                DownloadState::Finished { .. }
                    if icons::button(ui, true, Icon::Folder, 26.0)
                        .on_hover_text("Показать в папке")
                        .clicked() =>
                {
                    actions.push(Action::ShowDownloadInFolder(download.path.clone()));
                }
                _ => {}
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_like_firefox() {
        assert_eq!(format_size(512), "512 байт");
        assert_eq!(format_size(1536), "1,5 КБ");
        assert_eq!(format_size(4_404_019), "4,2 МБ");
    }
}
