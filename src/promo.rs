//! Scripted input for recording the product intro. Only built with `--features promo`.
//!
//! Reads one command per line from stdin and feeds it to egui as if someone were
//! using the mouse and keyboard, so a recording never needs the real mouse or
//! focus. It paints its own cursor for the same reason. Positions are egui points.
//!
//! ```text
//! move X Y SECONDS   glide the cursor there
//! click              press and release the left button where the cursor is
//! text STRING        type STRING
//! paste STRING       paste STRING
//! key NAME           Enter, Escape, Tab or Backspace
//! scroll DY          scroll by DY points (negative moves the content up)
//! zoom FACTOR        set the UI zoom
//! ```

use eframe::egui::{
    self, Color32, Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Shape,
    Stroke, pos2, vec2,
};
use std::{
    io::BufRead,
    sync::mpsc::{Receiver, channel},
    time::Instant,
};

enum Command {
    Move(Pos2, f64),
    Click,
    Text(String),
    Paste(String),
    Key(Key),
    Scroll(f32),
    Zoom(f32),
}

fn parse(line: &str) -> Option<Command> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let mut numbers = rest
        .split_whitespace()
        .filter_map(|n| n.parse::<f64>().ok());
    Some(match word {
        "move" => Command::Move(
            pos2(numbers.next()? as f32, numbers.next()? as f32),
            numbers.next().unwrap_or(0.0),
        ),
        "click" => Command::Click,
        "text" => Command::Text(rest.to_owned()),
        "paste" => Command::Paste(rest.to_owned()),
        "key" => Command::Key(Key::from_name(rest.trim())?),
        "scroll" => Command::Scroll(numbers.next()? as f32),
        "zoom" => Command::Zoom(numbers.next()? as f32),
        _ => return None,
    })
}

pub struct Promo {
    rx: Receiver<Command>,
    clock: Instant,
    cursor: Pos2,
    /// From, to, start time, duration.
    glide: Option<(Pos2, Pos2, f64, f64)>,
    release: bool,
    ripples: Vec<(Pos2, f64)>,
}

impl Promo {
    pub fn start() -> Self {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if let Some(command) = parse(line.trim_end_matches('\r'))
                    && tx.send(command).is_err()
                {
                    break;
                }
            }
        });
        Self {
            rx,
            clock: Instant::now(),
            cursor: pos2(-40.0, -40.0),
            glide: None,
            release: false,
            ripples: Vec::new(),
        }
    }

    pub fn feed(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        let now = self.clock.elapsed().as_secs_f64();
        // Pretend the window has focus and keep drawing frames for the recorder.
        raw.focused = true;
        ctx.request_repaint();
        let events = &mut raw.events;
        if std::mem::take(&mut self.release) {
            events.push(button(self.cursor, false));
        }
        while let Ok(command) = self.rx.try_recv() {
            match command {
                Command::Move(to, seconds) => self.glide = Some((self.cursor, to, now, seconds)),
                Command::Click => {
                    events.push(button(self.cursor, true));
                    self.release = true;
                    self.ripples.push((self.cursor, now));
                }
                Command::Text(text) => events.push(Event::Text(text)),
                Command::Paste(text) => events.push(Event::Paste(text)),
                Command::Key(key) => {
                    for pressed in [true, false] {
                        events.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        });
                    }
                }
                Command::Scroll(dy) => events.push(Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0.0, dy),
                    modifiers: Modifiers::NONE,
                }),
                Command::Zoom(factor) => ctx.set_zoom_factor(factor),
            }
        }
        if let Some((from, to, start, seconds)) = self.glide {
            let t = if seconds <= 0.0 {
                1.0
            } else {
                ((now - start) / seconds).clamp(0.0, 1.0) as f32
            };
            self.cursor = from.lerp(to, egui::emath::easing::cubic_in_out(t));
            events.push(Event::PointerMoved(self.cursor));
            if t >= 1.0 {
                self.glide = None;
            }
        }
    }

    /// Draws the stand-in cursor and click ripples above everything else.
    pub fn paint(&mut self, ctx: &egui::Context) {
        let now = self.clock.elapsed().as_secs_f64();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            egui::Id::new("promo-cursor"),
        ));
        self.ripples.retain(|(_, at)| now - at < 0.45);
        for (pos, at) in &self.ripples {
            let t = ((now - at) / 0.45) as f32;
            let ease = egui::emath::easing::cubic_out(t);
            painter.circle_stroke(
                *pos,
                5.0 + 17.0 * ease,
                Stroke::new(2.0, Color32::from_white_alpha((150.0 * (1.0 - t)) as u8)),
            );
        }
        const ARROW: [(f32, f32); 7] = [
            (0.0, 0.0),
            (0.0, 18.0),
            (4.6, 14.0),
            (7.6, 21.0),
            (10.6, 19.8),
            (7.6, 13.0),
            (13.0, 13.0),
        ];
        let at = |offset: egui::Vec2| {
            ARROW
                .iter()
                .map(|&(x, y)| self.cursor + offset + vec2(x, y))
                .collect::<Vec<_>>()
        };
        painter.add(Shape::convex_polygon(
            at(vec2(1.0, 2.0)),
            Color32::from_black_alpha(70),
            Stroke::NONE,
        ));
        // The arrow is concave, so fill it as two convex halves, then outline it.
        let points = at(vec2(0.0, 0.0));
        for part in [&[0, 1, 2, 5, 6][..], &[2, 3, 4, 5][..]] {
            let half: Vec<Pos2> = part.iter().map(|&i| points[i]).collect();
            painter.add(Shape::convex_polygon(half, Color32::WHITE, Stroke::NONE));
        }
        painter.add(Shape::closed_line(
            points,
            Stroke::new(1.2, Color32::from_gray(20)),
        ));
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
