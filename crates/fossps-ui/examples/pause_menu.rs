//! Builds a HUD and a pause menu, presses down once, and prints what
//! would be drawn. Run with `cargo run -p fossps-ui --example pause_menu`.

use fossps_ui::{Anchor, Color, Draw, Ui, UiError, UiInput};

fn main() -> Result<(), UiError> {
    let mut ui = Ui::default();
    ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
    ui.hud_bar(
        Anchor::BottomLeft,
        [0.0, 0.0],
        [300.0, 20.0],
        Color::rgb(0.8, 0.1, 0.1),
        0.6,
    );

    let pause = ui.add_screen("Paused", |screen| {
        screen.button("Resume");
        screen.slider("Volume", 0.0, 100.0, 5.0, 80.0);
        screen.button("Quit");
    })?;
    ui.open(pause)?;

    let down = UiInput {
        direction: [0.0, -1.0],
        ..UiInput::default()
    };
    ui.update(&down, 1.0 / 60.0, [1920, 1080]);

    for draw in ui.draw() {
        match draw {
            Draw::Rect {
                position: [x, y],
                size: [w, h],
                ..
            } => {
                println!("box   {w:6.0} x {h:<5.0} at {x:6.0}, {y:<6.0}");
            }
            Draw::Text {
                position: [x, y],
                size,
                text,
                ..
            } => {
                println!("text  {size:6.0} high    at {x:6.0}, {y:<6.0} \"{text}\"");
            }
        }
    }
    Ok(())
}
