use core::time;
use std::{collections::HashMap, process::Command, thread::sleep};

use serde::Deserialize;

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
}

impl Layer {
    fn does_contain_cursor(&self, cursorpos: &CursorPos, bar_visible: bool) -> bool {
        let y_buffer = self.h * 2 / 3;
        let mut bar_y_max = self.y + self.h;
        let mut bar_y_min = self.y;

        if self.y > self.h {
            if bar_visible {
                bar_y_min -= y_buffer;
            } else {
                bar_y_min += y_buffer;
            };
        } else if bar_visible {
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
}

type Level = u16;
type Monitor = String;

#[derive(Deserialize, Debug, Clone)]
struct LayerByLevel {
    levels: HashMap<Level, Vec<Layer>>,
}

fn main() {
    let primary_monitor = std::env::var("PRIMARY_MONITOR").unwrap();

    let layers_stdout = Command::new("hyprctl")
        .args(["layers", "-j"])
        .output()
        .unwrap()
        .stdout;
    let layers_str = String::from_utf8(layers_stdout).unwrap();
    let layers: HashMap<Monitor, LayerByLevel> = serde_json::from_str(&layers_str).unwrap();

    let primary_monitor_layers = layers.get(&primary_monitor).unwrap();
    let bar_layer = primary_monitor_layers
        .levels
        .iter()
        .filter_map(|(_, layers)| {
            if layers.is_empty() || layers.first().unwrap().namespace != "waybar" {
                return None;
            }

            Some(layers.first().cloned().unwrap())
        })
        .collect::<Vec<Layer>>()
        .first()
        .unwrap()
        .clone();

    let mut bar_visible = true;
    loop {
        sleep(time::Duration::from_millis(200));
        let Ok(cursorpos_stdout) = Command::new("hyprctl").args(["cursorpos", "-j"]).output()
        else {
            eprintln!("Couldn't get cursor position");
            continue;
        };
        let cursorpos_stdout = cursorpos_stdout.stdout;
        let Ok(cursorpos_str) = String::from_utf8(cursorpos_stdout) else {
            eprintln!("Parsing stdout to a String failed");
            continue;
        };
        let Ok(cursorpos) = serde_json::from_str(&cursorpos_str) else {
            eprintln!("Deserializing the cursorpos string failed");
            continue;
        };

        let cursor_over_bar = bar_layer.does_contain_cursor(&cursorpos, bar_visible);

        if cursor_over_bar && !bar_visible {
            let _ = Command::new("pkill").args(["-SIGUSR1", "waybar"]).spawn();
            let _ = Command::new("pkill").args(["-SIGUSR1", "nwg-dock"]).spawn();
            bar_visible = true;
            println!("Bar revealed.");
        } else if !cursor_over_bar && bar_visible {
            let _ = Command::new("pkill").args(["-SIGUSR1", "waybar"]).spawn();
            let _ = Command::new("pkill").args(["-SIGUSR1", "nwg-dock"]).spawn();
            bar_visible = false;
            println!("Bar hidden.");
        }
    }
}
