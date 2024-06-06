use core::time;
use std::{collections::HashMap, process::Command, thread::sleep};

use serde::Deserialize;

const NAMESPACES: [(&str, &str); 2] = [("waybar", "waybar"), ("gtk-layer-shell", "nwg-dock")];

#[derive(Deserialize, Debug, Clone)]
struct CursorPos {
    x: i32,
    y: i32,
}

#[derive(Deserialize, Debug, Clone)]
struct Layer {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    namespace: String,
    #[serde(skip_deserializing)]
    visible: bool,
}

impl Layer {
    fn does_contain_cursor(&self, cursorpos: &CursorPos) -> bool {
        let y_buffer = self.h * 2 / 3;
        let mut bar_y_max = self.y + self.h;
        let mut bar_y_min = self.y;

        if self.y > self.h {
            if self.visible {
                bar_y_min -= y_buffer;
            } else {
                bar_y_min += y_buffer;
            };
        } else if self.visible {
            bar_y_max += y_buffer;
        } else {
            bar_y_max -= y_buffer;
        }

        let bar_x_max = self.x + self.w;
        let bar_x_min = self.x;

        cursorpos.y <= bar_y_max
            && cursorpos.y >= bar_y_min
            && cursorpos.x <= bar_x_max
            && cursorpos.x >= bar_x_min
    }

    fn toggle_visibility(&mut self, cursorpos: &CursorPos) -> anyhow::Result<()> {
        let cursor_over_layer = self.does_contain_cursor(cursorpos);
        let toggle = || -> anyhow::Result<()> {
            Command::new("pkill")
                .args(["-SIGUSR1", &self.namespace])
                .spawn()?;
            Ok(())
        };
        if cursor_over_layer && !self.visible {
            toggle()?;
            self.visible = true;
            println!("{} revealed.", self.namespace);
        } else if !cursor_over_layer && self.visible {
            sleep(time::Duration::from_secs(1));
            toggle()?;
            self.visible = false;
            println!("{} hidden.", self.namespace);
        }

        Ok(())
    }
}

type Level = u16;
type Monitor = String;

#[derive(Deserialize, Debug, Clone)]
struct LayerByLevel {
    levels: HashMap<Level, Vec<Layer>>,
}

fn get_layers() -> anyhow::Result<Vec<Layer>> {
    let layers_stdout = Command::new("hyprctl")
        .args(["layers", "-j"])
        .output()?
        .stdout;
    let layers_str = String::from_utf8(layers_stdout)?;
    let levels_by_monitor: HashMap<Monitor, LayerByLevel> = serde_json::from_str(&layers_str)?;

    Ok(levels_by_monitor
        .into_iter()
        .flat_map(|(_, layer_by_level)| {
            layer_by_level
                .levels
                .into_iter()
                .flat_map(|(_, layer)| layer)
                .collect::<Vec<Layer>>()
        })
        .filter_map(|layer| {
            for namespace in NAMESPACES {
                if namespace.0 == layer.namespace {
                    return Some(Layer {
                        namespace: namespace.1.to_string(),
                        visible: true,
                        ..layer
                    });
                }
            }
            None
        })
        .collect::<Vec<Layer>>())
}

fn get_cursor_pos() -> anyhow::Result<CursorPos> {
    let cursorpos_stdout = Command::new("hyprctl").args(["cursorpos", "-j"]).output()?;
    let cursorpos_stdout = cursorpos_stdout.stdout;
    let cursorpos_str = String::from_utf8(cursorpos_stdout)?;
    Ok(serde_json::from_str(cursorpos_str.as_str())?)
}

fn main() {
    let mut layers = get_layers().unwrap();

    while layers.is_empty() {
        sleep(std::time::Duration::from_secs(1));
        layers = get_layers().unwrap();
    }

    println!("{:#?}", layers.clone());

    loop {
        sleep(time::Duration::from_millis(200));

        let cursorpos = match get_cursor_pos() {
            Ok(cursorpos) => cursorpos,
            Err(err) => {
                eprintln!("{}", err);
                continue;
            }
        };

        for layer in layers.iter_mut() {
            match layer.toggle_visibility(&cursorpos) {
                Ok(_) => {}
                Err(err) => eprintln!("{}", err),
            }
        }
    }
}
