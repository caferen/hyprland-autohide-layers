use core::time;
use std::{
    collections::HashMap,
    process::Command,
    sync::{Arc, Condvar, Mutex, RwLock},
    thread::{self, sleep},
};

use clap::Parser;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Opts {
    #[arg(short, long)]
    namespace: Vec<String>,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq)]
struct CursorPos {
    x: f32,
    y: f32,
}

#[derive(Deserialize, Debug, Clone)]
struct Layer {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    namespace: String,
    #[serde(skip_deserializing)]
    visible: bool,
}

impl Layer {
    fn does_contain_cursor(&self, cursorpos: &CursorPos) -> bool {
        let y_buffer = self.h * 2.0 / 3.0;
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
            thread::sleep(time::Duration::from_secs(1));
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

fn get_layers(namespaces: &Vec<String>) -> anyhow::Result<Vec<Layer>> {
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
            for namespace in namespaces {
                if *namespace == layer.namespace {
                    return Some(Layer {
                        namespace: namespace.to_string(),
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

#[derive(Deserialize, Debug, Clone)]
struct Client {
    fullscreen: bool,
    floating: bool,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: u16,
}

fn fullscreen_or_floating_focused() -> anyhow::Result<bool> {
    let clients_stdout = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()?
        .stdout;
    let clients_str = String::from_utf8(clients_stdout)?;
    let clients: Vec<Client> = serde_json::from_str(&clients_str)?;
    Ok(clients
        .into_iter()
        .any(|client| client.focus_history_id == 0 && (client.fullscreen || client.floating)))
}

fn main() {
    let opts = Opts::parse();
    let mut layers = get_layers(&opts.namespace).unwrap();

    while layers.is_empty() {
        sleep(std::time::Duration::from_secs(1));
        layers = get_layers(&opts.namespace).unwrap();
    }

    println!("{:#?}", layers.clone());

    let cursorpos = Arc::new(RwLock::new(get_cursor_pos().unwrap()));

    let cursorpos_updater = Arc::clone(&cursorpos);
    let cursorpos_update_notifier = Arc::new((Mutex::new(false), Condvar::new()));

    layers.into_iter().for_each(|mut layer| {
        let cursorpos = Arc::clone(&cursorpos);
        let notifier = Arc::clone(&cursorpos_update_notifier);
        thread::spawn(move || loop {
            dbg!("Woke up to do update");
            {
                let curr_pos = cursorpos.read().unwrap();

                match layer.toggle_visibility(&curr_pos) {
                    Ok(_) => {}
                    Err(err) => eprintln!("{}", err),
                }
            }
            let lock = notifier.0.lock().unwrap();
            let _guard = notifier.1.wait(lock).unwrap();
        });
    });

    let notifier = Arc::clone(&cursorpos_update_notifier);

    loop {
        thread::sleep(time::Duration::from_millis(100));
        dbg!("Checking cursor pos");

        if fullscreen_or_floating_focused().is_ok_and(|res| res) {
            continue;
        }

        let mut prev_pos = cursorpos_updater.write().unwrap();

        let curr_pos = match get_cursor_pos() {
            Ok(new_cursorpos) => new_cursorpos,
            Err(err) => {
                eprintln!("{}", err);
                continue;
            }
        };

        if *prev_pos == curr_pos {
            continue;
        }

        *prev_pos = curr_pos;
        notifier.1.notify_all();
    }
}
