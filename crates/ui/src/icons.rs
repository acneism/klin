//! Значки панели, нарисованные линиями: не зависят от того, какие символы есть в шрифте.

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Stop,
    Home,
    Plus,
    Close,
    /// Значок страницы без собственного значка (в Firefox — defaultFavicon).
    Globe,
    /// Кнопка загрузок: стрелка вниз в лоток.
    Download,
    /// «Показать в папке».
    Folder,
}

/// Кнопка-значок размером `size` точек.
pub fn button(ui: &mut Ui, enabled: bool, icon: Icon, size: f32) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), sense);
    let visuals = ui.visuals();
    if enabled && response.hovered() {
        let fill = visuals.widgets.hovered.weak_bg_fill;
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    let color = if enabled {
        visuals.text_color()
    } else {
        visuals.weak_text_color().gamma_multiply(0.6)
    };
    paint(ui.painter(), rect.shrink(size * 0.28), icon, color);
    response
}

pub fn paint(painter: &egui::Painter, r: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let line = |points: Vec<Pos2>| painter.add(Shape::line(points, stroke));
    match icon {
        Icon::Back => {
            line(vec![at(1.0, 0.5), at(0.0, 0.5)]);
            line(vec![at(0.45, 0.05), at(0.0, 0.5), at(0.45, 0.95)]);
        }
        Icon::Forward => {
            line(vec![at(0.0, 0.5), at(1.0, 0.5)]);
            line(vec![at(0.55, 0.05), at(1.0, 0.5), at(0.55, 0.95)]);
        }
        Icon::Reload => {
            let c = r.center();
            let radius = r.width() * 0.45;
            // Дуга почти в полный круг с разрывом справа сверху.
            let arc: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = -0.35 + (i as f32 / 24.0) * 5.4;
                    c + vec2(a.cos(), -a.sin()) * radius
                })
                .collect();
            let end = arc[0];
            line(arc);
            line(vec![
                end + vec2(-radius * 0.55, -radius * 0.05),
                end,
                end + vec2(0.05 * radius, -radius * 0.6),
            ]);
        }
        Icon::Stop | Icon::Close => {
            line(vec![at(0.1, 0.1), at(0.9, 0.9)]);
            line(vec![at(0.9, 0.1), at(0.1, 0.9)]);
        }
        Icon::Home => {
            line(vec![at(0.0, 0.5), at(0.5, 0.05), at(1.0, 0.5)]);
            line(vec![
                at(0.18, 0.38),
                at(0.18, 0.95),
                at(0.82, 0.95),
                at(0.82, 0.38),
            ]);
        }
        Icon::Globe => {
            let c = r.center();
            let radius = r.width() * 0.48;
            painter.circle_stroke(c, radius, Stroke::new(1.2, color));
            painter.add(Shape::ellipse_stroke(
                c,
                vec2(radius * 0.45, radius),
                Stroke::new(1.0, color),
            ));
            line(vec![at(0.02, 0.5), at(0.98, 0.5)]);
        }
        Icon::Download => {
            line(vec![at(0.5, 0.0), at(0.5, 0.72)]);
            line(vec![at(0.22, 0.45), at(0.5, 0.72), at(0.78, 0.45)]);
            line(vec![
                at(0.05, 0.75),
                at(0.05, 1.0),
                at(0.95, 1.0),
                at(0.95, 0.75),
            ]);
        }
        Icon::Folder => {
            line(vec![
                at(0.0, 0.15),
                at(0.38, 0.15),
                at(0.48, 0.28),
                at(1.0, 0.28),
                at(1.0, 0.92),
                at(0.0, 0.92),
                at(0.0, 0.15),
            ]);
        }
        Icon::Plus => {
            line(vec![at(0.5, 0.0), at(0.5, 1.0)]);
            line(vec![at(0.0, 0.5), at(1.0, 0.5)]);
        }
    }
}

/// Индикатор загрузки на месте значка вкладки: вращающаяся дуга.
pub fn spinner(painter: &egui::Painter, r: Rect, color: Color32, time: f64) {
    let c = r.center();
    let radius = r.width() * 0.45;
    let start = (time * 6.0) as f32;
    let arc: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = start + i as f32 / 20.0 * 4.2;
            c + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(Shape::line(arc, Stroke::new(2.0, color)));
}
